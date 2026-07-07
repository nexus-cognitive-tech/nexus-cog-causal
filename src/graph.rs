//! Causal graph engine.

use std::sync::Arc;

use nexus_cog_core::causal::{CausalEdge, CausalEdgeType, CausalGraph, CausalNode, CausalNodeType};
use indexmap::IndexMap;
use parking_lot::RwLock;
use petgraph::graph::DiGraph;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

/// Inner mutable state of [`CausalGraphEngine`].
#[derive(Debug)]
struct Inner {
    graph: DiGraph<CausalNode, ()>,
    index: IndexMap<String, NodeIndex>,
}

/// Maintains a causal graph with efficient traversal.
///
/// Cloning is cheap (Arc clone). Mutations use interior mutability via
/// [`parking_lot::RwLock`], so reasoners and other readers can hold their
/// own snapshot of the engine without lifetime ties.
#[derive(Debug, Clone)]
pub struct CausalGraphEngine {
    inner: Arc<RwLock<Inner>>,
}

impl Default for CausalGraphEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CausalGraphEngine {
    /// Construct an empty engine.
        pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                graph: DiGraph::new(),
                index: IndexMap::new(),
            })),
        }
    }

    /// Construct from a snapshot.
        pub fn from_graph(graph: CausalGraph) -> Self {
        let mut engine = Self::new();
        for node in graph.nodes {
            engine.add_node(node);
        }
        for edge in graph.edges {
            engine.add_edge(edge);
        }
        engine
    }

    /// Number of nodes.
        pub fn node_count(&self) -> usize {
        self.inner.read().graph.node_count()
    }

    /// Number of edges.
        pub fn edge_count(&self) -> usize {
        self.inner.read().graph.edge_count()
    }

    /// Add a node.
    pub fn add_node(&mut self, node: CausalNode) -> NodeIndex {
        let id = node.id.clone();
        let mut inner = self.inner.write();
        let idx = inner.graph.add_node(node);
        inner.index.insert(id, idx);
        idx
    }

    /// Add an edge.
    pub fn add_edge(&mut self, edge: CausalEdge) -> bool {
        let mut inner = self.inner.write();
        let Some(&from) = inner.index.get(&edge.from) else { return false };
        let Some(&to) = inner.index.get(&edge.to) else { return false };
        inner.graph.add_edge(from, to, ());
        true
    }

    /// Get a node by ID.
        pub fn node(&self, id: &str) -> Option<CausalNode> {
        let inner = self.inner.read();
        inner.index.get(id).and_then(|idx| inner.graph.node_weight(*idx).cloned())
    }

    /// All nodes of a particular type.
        pub fn nodes_of_type(&self, ty: CausalNodeType) -> Vec<CausalNode> {
        let inner = self.inner.read();
        inner
            .graph
            .node_indices()
            .filter_map(|idx| inner.graph.node_weight(idx).cloned())
            .filter(|n| n.node_type == ty)
            .collect()
    }

    /// Forward closure from a node (all positive descendants).
        pub fn forward_closure(&self, id: &str) -> std::collections::HashSet<String> {
        let inner = self.inner.read();
        let Some(&start) = inner.index.get(id) else { return std::collections::HashSet::new() };
        let mut visited = std::collections::HashSet::new();
        let mut order = Vec::new();
        let mut stack = vec![start];
        while let Some(idx) = stack.pop() {
            if !visited.insert(idx) {
                continue;
            }
            order.push(idx);
            for edge in inner.graph.edges_directed(idx, petgraph::Direction::Outgoing) {
                stack.push(edge.target());
            }
        }
        order
            .into_iter()
            .filter_map(|idx| inner.graph.node_weight(idx))
            .map(|n| n.id.clone())
            .collect()
    }

    /// Backward closure from a node (all positive ancestors).
        pub fn backward_closure(&self, id: &str) -> std::collections::HashSet<String> {
        let inner = self.inner.read();
        let Some(&start) = inner.index.get(id) else { return std::collections::HashSet::new() };
        let mut visited = std::collections::HashSet::new();
        let mut order = Vec::new();
        let mut stack = vec![start];
        while let Some(idx) = stack.pop() {
            if !visited.insert(idx) {
                continue;
            }
            order.push(idx);
            for edge in inner.graph.edges_directed(idx, petgraph::Direction::Incoming) {
                stack.push(edge.source());
            }
        }
        order
            .into_iter()
            .filter_map(|idx| inner.graph.node_weight(idx))
            .map(|n| n.id.clone())
            .collect()
    }

    /// Snapshot the engine to a serializable graph.
        pub fn snapshot(&self) -> CausalGraph {
        let inner = self.inner.read();
        let nodes: Vec<CausalNode> = inner
            .graph
            .node_indices()
            .filter_map(|idx| inner.graph.node_weight(idx).cloned())
            .collect();
        let mut edges = Vec::new();
        for edge_ref in inner.graph.edge_references() {
            let from = inner.graph.node_weight(edge_ref.source()).unwrap();
            let to = inner.graph.node_weight(edge_ref.target()).unwrap();
            edges.push(CausalEdge {
                from: from.id.clone(),
                to: to.id.clone(),
                edge_type: CausalEdgeType::Causes,
                strength: 1.0,
                confidence: from.confidence,
                evidence: Vec::new(),
            });
        }
        CausalGraph {
            nodes,
            edges,
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now(),
            scope: String::new(),
        }
    }

    /// Look up the internal node index for a node ID.
        pub fn index_of(&self, id: &str) -> Option<NodeIndex> {
        self.inner.read().index.get(id).copied()
    }

    /// Iterator over internal node indices.
        pub fn node_indices(&self) -> Vec<NodeIndex> {
        self.inner.read().graph.node_indices().collect()
    }

    /// Get the weight (node) at a given index.
        pub fn graph_weight(&self, idx: NodeIndex) -> Option<CausalNode> {
        self.inner.read().graph.node_weight(idx).cloned()
    }

    /// Get immediate parent node indices of the given node index.
        pub fn immediate_parents(&self, idx: NodeIndex) -> Vec<NodeIndex> {
        let inner = self.inner.read();
        inner
            .graph
            .edges_directed(idx, petgraph::Direction::Incoming)
            .map(|e| e.source())
            .collect()
    }

    /// Get immediate child node indices of the given node index.
        pub fn immediate_children(&self, idx: NodeIndex) -> Vec<NodeIndex> {
        let inner = self.inner.read();
        inner
            .graph
            .edges_directed(idx, petgraph::Direction::Outgoing)
            .map(|e| e.target())
            .collect()
    }

    /// Iterate over all nodes (cloned).
        pub fn nodes(&self) -> Vec<CausalNode> {
        let inner = self.inner.read();
        inner
            .graph
            .node_indices()
            .filter_map(|idx| inner.graph.node_weight(idx).cloned())
            .collect()
    }

    /// Iterate over all edges (cloned).
        pub fn edges(&self) -> Vec<CausalEdge> {
        let inner = self.inner.read();
        let mut edges = Vec::new();
        for edge_ref in inner.graph.edge_references() {
            let from = inner.graph.node_weight(edge_ref.source()).unwrap();
            let to = inner.graph.node_weight(edge_ref.target()).unwrap();
            edges.push(CausalEdge {
                from: from.id.clone(),
                to: to.id.clone(),
                edge_type: CausalEdgeType::Causes,
                strength: 1.0,
                confidence: from.confidence,
                evidence: Vec::new(),
            });
        }
        edges
    }

    /// Acquire a read guard for advanced traversal.
        pub fn read(&self) -> CausalGraphReadGuard<'_> {
        CausalGraphReadGuard { inner: self.inner.read() }
    }
}

/// Read-only guard over a [`CausalGraphEngine`] for advanced traversal.
pub struct CausalGraphReadGuard<'a> {
    inner: parking_lot::RwLockReadGuard<'a, Inner>,
}

impl<'a> CausalGraphReadGuard<'a> {
    /// Direct access to the underlying petgraph.
        pub fn graph(&self) -> &DiGraph<CausalNode, ()> {
        &self.inner.graph
    }

    /// Direct access to the id → index map.
        pub fn index(&self) -> &IndexMap<String, NodeIndex> {
        &self.inner.index
    }

    /// Look up the internal node index for a node ID.
        pub fn index_of(&self, id: &str) -> Option<NodeIndex> {
        self.inner.index.get(id).copied()
    }

    /// Iterator over all node indices.
        pub fn node_indices(&self) -> impl Iterator<Item = NodeIndex> + '_ {
        self.inner.graph.node_indices()
    }

    /// Get the weight (node) at a given index.
        pub fn graph_weight(&self, idx: NodeIndex) -> Option<&CausalNode> {
        self.inner.graph.node_weight(idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_cog_core::common::Confidence;

    fn node(id: &str) -> CausalNode {
        CausalNode {
            id: id.into(),
            node_type: CausalNodeType::CodeEntity,
            name: id.into(),
            description: String::new(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        }
    }

    #[test]
    fn add_and_get_node() {
        let mut e = CausalGraphEngine::new();
        e.add_node(node("a"));
        assert!(e.node("a").is_some());
    }

    #[test]
    fn forward_closure_walks_descendants() {
        let mut e = CausalGraphEngine::new();
        e.add_node(node("a"));
        e.add_node(node("b"));
        e.add_node(node("c"));
        e.add_edge(CausalEdge {
            from: "a".into(),
            to: "b".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        e.add_edge(CausalEdge {
            from: "b".into(),
            to: "c".into(),
            edge_type: CausalEdgeType::Enables,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        let closure = e.forward_closure("a");
        assert!(closure.contains("b"));
        assert!(closure.contains("c"));
    }

    #[test]
    fn backward_closure_walks_ancestors() {
        let mut e = CausalGraphEngine::new();
        e.add_node(node("a"));
        e.add_node(node("b"));
        e.add_edge(CausalEdge {
            from: "a".into(),
            to: "b".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        let closure = e.backward_closure("b");
        assert!(closure.contains("a"));
    }
}
