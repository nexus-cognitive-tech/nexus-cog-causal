//! Graph algorithms for causal analysis.
//!
//! Three algorithms that turn the causal graph from "I can walk neighbors" into
//! "I can answer structural questions":
//!
//! - **Strongly Connected Components** (Tarjan's algorithm, O(V+E)) — clusters
//!   of nodes that mutually reach each other. Surfaces circular dependencies
//!   and feedback loops in code.
//! - **Shortest Causal Path** (BFS over positive edges, O(V+E)) — the minimum-hop
//!   chain from a cause to an effect. Useful for "what's the simplest way X leads to Y?"
//! - **Topological Sort** (Kahn's algorithm, O(V+E)) — orders nodes so that every
//!   causal edge points forward. Only defined for DAGs; for cyclic graphs it
//!   returns the partial order plus the cycle members.

use std::collections::{HashMap, HashSet, VecDeque};

use nexus_cog_core::causal::CausalNode;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::CausalGraphEngine;

/// One strongly connected component.
#[derive(Debug, Clone, PartialEq)]
pub struct Scc {
    /// Stable identifier (assigned in DFS finish order).
    pub id: usize,
    /// Members of the component.
    pub members: Vec<String>,
    /// Size of the component.
    pub size: usize,
}

impl Scc {
    /// `true` if this component has more than one node — i.e. a real cycle.
    #[must_use]
    pub fn is_cycle(&self) -> bool {
        self.size > 1
    }

    /// `true` if this component is a single-node self-loop. Use the
    /// [`SccResult::cycles`] list to find out which singletons actually loop.
    #[must_use]
    pub fn is_self_loop(&self) -> bool {
        self.size == 1
    }
}

/// Result of computing SCCs.
#[derive(Debug, Clone)]
pub struct SccResult {
    /// All components in reverse topological order (largest first within a tier).
    pub components: Vec<Scc>,
    /// The component each node belongs to (by node ID).
    pub membership: HashMap<String, usize>,
    /// Indices into `components` that represent cycles (size > 1).
    pub cycles: Vec<usize>,
    /// Total number of cycles (including self-loops).
    pub cycle_count: usize,
}

impl SccResult {
    /// Returns the components that are cycles (size > 1).
    #[must_use]
    pub fn cyclic_components(&self) -> Vec<&Scc> {
        self.components
            .iter()
            .filter(|c| c.is_cycle())
            .collect()
    }

    /// Returns the components that are pure singletons (size 1, no self-loop).
    #[must_use]
    pub fn singleton_components(&self) -> Vec<&Scc> {
        self.components
            .iter()
            .filter(|c| c.size == 1 && !self.cycles.contains(&c.id))
            .collect()
    }
}

/// Compute strongly connected components using Tarjan's algorithm.
///
/// `min_size` filters out trivial components: pass `2` to only see real cycles
/// (size > 1) and self-loops.
#[must_use]
pub fn strongly_connected_components(engine: &CausalGraphEngine, min_size: usize) -> SccResult {
    let guard = engine.read();
    let g = guard.graph();
    let mut indices: HashMap<NodeIndex, usize> = HashMap::new();
    let mut lowlinks: HashMap<NodeIndex, usize> = HashMap::new();
    let mut on_stack: HashSet<NodeIndex> = HashSet::new();
    let mut stack: Vec<NodeIndex> = Vec::new();
    let mut components: Vec<Vec<NodeIndex>> = Vec::new();
    let mut next_index = 0_usize;

    for start in guard.node_indices() {
        if !indices.contains_key(&start) {
            tarjan_strongconnect(
                start,
                g,
                &mut next_index,
                &mut indices,
                &mut lowlinks,
                &mut on_stack,
                &mut stack,
                &mut components,
            );
        }
    }

    // Pre-compute which nodes have self-loops.
    let mut self_loop_nodes: HashSet<NodeIndex> = HashSet::new();
    for edge in g.edge_references() {
        if edge.source() == edge.target() {
            self_loop_nodes.insert(edge.source());
        }
    }

    let mut membership: HashMap<String, usize> = HashMap::new();
    let mut sccs: Vec<Scc> = Vec::with_capacity(components.len());
    let mut cycles: Vec<usize> = Vec::new();
    for (i, members) in components.into_iter().enumerate() {
        let member_ids: Vec<String> = members
            .iter()
            .filter_map(|idx| g.node_weight(*idx).map(|n| n.id.clone()))
            .collect();
        let size = member_ids.len();
        let is_cycle = size > 1;
        let is_self_loop = size == 1 && members.iter().any(|idx| self_loop_nodes.contains(idx));
        let scc = Scc { id: i, size, members: member_ids.clone() };
        for id in &member_ids {
            membership.insert(id.clone(), i);
        }
        if is_cycle || is_self_loop {
            cycles.push(i);
        }
        if size >= min_size {
            sccs.push(scc);
        }
    }

    // Order: largest components first, then by first member for determinism.
    sccs.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.members.first().cmp(&b.members.first())));

    SccResult {
        cycle_count: cycles.len(),
        components: sccs,
        membership,
        cycles,
    }
}

#[allow(clippy::too_many_arguments)]
fn tarjan_strongconnect(
    v: NodeIndex,
    g: &petgraph::graph::DiGraph<CausalNode, ()>,
    index: &mut usize,
    indices: &mut HashMap<NodeIndex, usize>,
    lowlinks: &mut HashMap<NodeIndex, usize>,
    on_stack: &mut HashSet<NodeIndex>,
    stack: &mut Vec<NodeIndex>,
    components: &mut Vec<Vec<NodeIndex>>,
) {
    let v_idx = *index;
    *index += 1;
    indices.insert(v, v_idx);
    lowlinks.insert(v, v_idx);
    stack.push(v);
    on_stack.insert(v);

    for edge in g.edges_directed(v, petgraph::Direction::Outgoing) {
        let w = edge.target();
        if !indices.contains_key(&w) {
            tarjan_strongconnect(w, g, index, indices, lowlinks, on_stack, stack, components);
            let w_low = lowlinks[&w];
            let v_low = lowlinks[&v];
            if w_low < v_low {
                lowlinks.insert(v, w_low);
            }
        } else if on_stack.contains(&w) {
            let w_idx = indices[&w];
            let v_low = lowlinks[&v];
            if w_idx < v_low {
                lowlinks.insert(v, w_idx);
            }
        }
    }

    if lowlinks[&v] == indices[&v] {
        let mut component = Vec::new();
        loop {
            let w = stack.pop().expect("stack invariant violated");
            on_stack.remove(&w);
            component.push(w);
            if w == v {
                break;
            }
        }
        component.reverse(); // root-first order
        components.push(component);
    }
}

/// Shortest path (by hop count) between two nodes using only positive causal edges.
///
/// Returns `None` if no path exists. Returns the path as a sequence of node IDs
/// (inclusive of both endpoints).
#[must_use]
pub fn shortest_path(engine: &CausalGraphEngine, from: &str, to: &str) -> Option<Vec<String>> {
    let guard = engine.read();
    let g = guard.graph();
    let start = engine.index_of(from)?;
    let goal = engine.index_of(to)?;

    if start == goal {
        return Some(vec![from.to_string()]);
    }

    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    let mut parent: HashMap<NodeIndex, NodeIndex> = HashMap::new();

    visited.insert(start);
    queue.push_back(start);

    while let Some(v) = queue.pop_front() {
        if v == goal {
            // Reconstruct path.
            let mut path = vec![v];
            let mut cur = v;
            while let Some(&p) = parent.get(&cur) {
                path.push(p);
                cur = p;
            }
            path.reverse();
            return Some(
                path.into_iter()
                    .filter_map(|idx| g.node_weight(idx).map(|n| n.id.clone()))
                    .collect(),
            );
        }
        for edge in g.edges_directed(v, petgraph::Direction::Outgoing) {
            let next = edge.target();
            if visited.insert(next) {
                parent.insert(next, v);
                queue.push_back(next);
            }
        }
    }
    None
}

/// Result of a topological sort attempt.
#[derive(Debug, Clone)]
pub struct TopoResult {
    /// Topological order (empty if the graph has cycles).
    pub order: Vec<String>,
    /// Nodes that participate in cycles (and therefore could not be ordered).
    pub cyclic_nodes: Vec<String>,
}

impl TopoResult {
    /// `true` if the graph is a DAG.
    #[must_use]
    pub fn is_dag(&self) -> bool {
        self.cyclic_nodes.is_empty()
    }
}

/// Compute a topological order using Kahn's algorithm. Nodes involved in cycles
/// are returned separately in `cyclic_nodes`.
#[must_use]
pub fn topological_sort(engine: &CausalGraphEngine) -> TopoResult {
    let guard = engine.read();
    let g = guard.graph();

    // Compute in-degrees over the original nodes (we want causal edges, not the
    // collapse structure).
    let mut in_degree: HashMap<NodeIndex, usize> = HashMap::new();
    for idx in guard.node_indices() {
        in_degree.insert(idx, 0);
    }
    for edge in g.edge_references() {
        *in_degree.entry(edge.target()).or_insert(0) += 1;
    }

    let mut queue: VecDeque<NodeIndex> = in_degree
        .iter()
        .filter_map(|(idx, deg)| if *deg == 0 { Some(*idx) } else { None })
        .collect();
    // Sort the initial queue for determinism.
    let mut initial: Vec<NodeIndex> = queue.into_iter().collect();
    initial.sort_by_key(|idx| g.node_weight(*idx).map(|n| n.id.as_str()).unwrap_or(""));
    queue = initial.into_iter().collect();

    let mut order: Vec<String> = Vec::new();
    let mut remaining_in = in_degree.clone();
    while let Some(v) = queue.pop_front() {
        if let Some(n) = g.node_weight(v) {
            order.push(n.id.clone());
        }
        for edge in g.edges_directed(v, petgraph::Direction::Outgoing) {
            let next = edge.target();
            if let Some(deg) = remaining_in.get_mut(&next) {
                *deg = deg.saturating_sub(1);
                if *deg == 0 {
                    queue.push_back(next);
                }
            }
        }
    }

    let cyclic_nodes: Vec<String> = in_degree.into_keys().filter_map(|idx| {
            if order.iter().any(|id| g.node_weight(idx).is_some_and(|n| &n.id == id)) {
                None
            } else {
                g.node_weight(idx).map(|n| n.id.clone())
            }
        })
        .collect();

    TopoResult { order, cyclic_nodes }
}

/// Detect all cycles in the causal graph as lists of node IDs.
///
/// Returns cycles ordered from shortest to longest. Self-loops count as
/// length-1 cycles.
#[must_use]
pub fn all_cycles(engine: &CausalGraphEngine, max_per_component: usize) -> Vec<Vec<String>> {
    let sccs = strongly_connected_components(engine, 2);
    sccs.cyclic_components()
        .into_iter()
        .take(max_per_component)
        .map(|c| c.members.clone())
        .collect()
}

/// Build the condensed component graph: each SCC becomes one node.
///
/// Returns `(node_ids, edges)` where edges are `(from_component, to_component, edge_count)`.
#[must_use]
pub fn condensation(
    engine: &CausalGraphEngine,
) -> (Vec<String>, Vec<(usize, usize, usize)>) {
    let sccs = strongly_connected_components(engine, 1);
    let guard = engine.read();
    let g = guard.graph();
    let mut edges: std::collections::BTreeMap<(usize, usize), usize> = std::collections::BTreeMap::new();
    for edge in g.edge_references() {
        let from = sccs.membership.get(&g.node_weight(edge.source()).unwrap().id).copied().unwrap_or(0);
        let to = sccs.membership.get(&g.node_weight(edge.target()).unwrap().id).copied().unwrap_or(0);
        if from != to {
            *edges.entry((from, to)).or_insert(0) += 1;
        }
    }
    let node_ids: Vec<String> = sccs.components.iter().map(|c| {
        if c.members.len() == 1 {
            c.members.first().cloned().unwrap_or_default()
        } else {
            format!("[cycle:{}]", c.members.join(","))
        }
    }).collect();
    let edge_list: Vec<_> = edges.into_iter().map(|((from, to), count)| (from, to, count)).collect();
    (node_ids, edge_list)
}

/// Helper: build a causal graph with a small example for tests.
#[cfg(test)]
pub fn example_graph() -> (CausalGraphEngine, Vec<&'static str>) {
    use nexus_cog_core::causal::{CausalEdge, CausalEdgeType, CausalNode, CausalNodeType};
    use nexus_cog_core::common::Confidence;

    let mut g = CausalGraphEngine::new();
    for id in ["a", "b", "c", "d", "e"] {
        g.add_node(CausalNode {
            id: id.into(),
            node_type: CausalNodeType::CodeEntity,
            name: id.into(),
            description: String::new(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        });
    }
    let edges = [
        ("a", "b", CausalEdgeType::Causes),
        ("b", "c", CausalEdgeType::Causes),
        ("a", "c", CausalEdgeType::Causes),
        ("c", "d", CausalEdgeType::Causes),
    ];
    for (from, to, ty) in edges {
        g.add_edge(CausalEdge {
            from: from.into(),
            to: to.into(),
            edge_type: ty,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
    }
    (g, vec!["a", "b", "c", "d", "e"])
}
