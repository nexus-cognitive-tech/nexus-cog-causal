//! Backward causal reasoning: "why does this bug exist?"

use nexus_cog_core::causal::{CausalEdge, CausalEdgeType, CausalNode, CausalNodeType};
use petgraph::graph::NodeIndex;

use crate::graph::CausalGraphEngine;

/// Traces backward through causal chains.
#[derive(Debug, Clone)]
pub struct BackwardReasoner {
    engine: CausalGraphEngine,
}

impl BackwardReasoner {
    /// Construct a backward reasoner.
    #[must_use]
    pub fn new(engine: CausalGraphEngine) -> Self {
        Self { engine }
    }

    /// Why does `outcome` exist? Returns the chain of causes.
    #[must_use]
    pub fn causes_of(&self, outcome: &str) -> Vec<CausalNode> {
        let ids = self.engine.backward_closure(outcome);
        self.engine
            .nodes()
            .into_iter()
            .filter(|n| ids.contains(&n.id) && n.id != outcome)
            .collect()
    }

    /// The most direct causes — only immediate parents.
    #[must_use]
    pub fn immediate_causes(&self, outcome: &str) -> Vec<CausalNode> {
        if let Some(idx) = self.engine.index_of(outcome) {
            self.engine
                .immediate_parents(idx)
                .into_iter()
                .filter_map(|i| self.engine.graph_weight(i))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Generate a textual explanation of why this outcome exists.
    #[must_use]
    pub fn explain(&self, outcome: &str) -> String {
        let outcome_node = match self.engine.node(outcome) {
            Some(n) => n,
            None => return format!("No causal node found for `{outcome}`."),
        };
        let mut lines = Vec::new();
        lines.push(format!("## Why does `{}` exist?", outcome_node.name));
        lines.push(format!("Type: `{}`", node_type_label(outcome_node.node_type)));
        lines.push(String::new());
        lines.push("### Causal chain:".into());
        let chain = self.causes_of(outcome);
        if chain.is_empty() {
            lines.push("(no upstream causes recorded)".into());
        } else {
            for (i, n) in chain.iter().enumerate() {
                lines.push(format!("{}. **{}** ({})", i + 1, n.name, node_type_label(n.node_type)));
            }
        }
        let immediate = self.immediate_causes(outcome);
        if !immediate.is_empty() {
            lines.push(String::new());
            lines.push("### Immediate causes:".into());
            for n in &immediate {
                lines.push(format!("- **{}**: {}", n.name, n.description));
            }
        }
        lines.join("\n")
    }
}

fn node_type_label(ty: CausalNodeType) -> &'static str {
    match ty {
        CausalNodeType::CodeEntity => "code",
        CausalNodeType::Behavior => "behavior",
        CausalNodeType::Feature => "feature",
        CausalNodeType::Invariant => "invariant",
        CausalNodeType::Assumption => "assumption",
        CausalNodeType::Decision => "decision",
        CausalNodeType::Constraint => "constraint",
        CausalNodeType::Bug => "bug",
        CausalNodeType::ExternalDep => "external dependency",
    }
}

/// Helper: enumerate edges along the backward chain.
pub fn edges_to_root(engine: &CausalGraphEngine, outcome: &str) -> Vec<(String, String, CausalEdgeType)> {
    let mut out = Vec::new();
    let mut visited_edges = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(outcome.to_string());
    while let Some(current) = queue.pop_front() {
        let Some(idx) = engine.index_of(&current) else { break };
        let parents = engine.immediate_parents(idx);
        if parents.is_empty() {
            continue;
        }
        for parent_idx in &parents {
            if let (Some(p), Some(_c)) = (engine.graph_weight(*parent_idx), engine.graph_weight(idx)) {
                let key = (p.id.clone(), current.clone());
                if visited_edges.insert(key) {
                    out.push((p.id.clone(), current.clone(), CausalEdgeType::Causes));
                    queue.push_back(p.id.clone());
                }
            }
        }
    }
    out
}

/// Helper to detect cycles using DFS.
pub fn has_cycle(engine: &CausalGraphEngine) -> bool {
    use petgraph::visit::EdgeRef;
    use std::collections::HashSet;
    let guard = engine.read();
    let g = guard.graph();
    for start in guard.node_indices() {
        let mut on_stack = HashSet::new();
        let mut visited = HashSet::new();
        let mut stack: Vec<(NodeIndex, bool)> = vec![(start, false)];
        while let Some((node, exiting)) = stack.pop() {
            if exiting {
                on_stack.remove(&node);
                continue;
            }
            if on_stack.contains(&node) {
                return true; // back edge = cycle
            }
            if visited.contains(&node) {
                continue;
            }
            on_stack.insert(node);
            stack.push((node, true));
            for edge in g.edges_directed(node, petgraph::Direction::Outgoing) {
                if !visited.contains(&edge.target()) {
                    stack.push((edge.target(), false));
                }
            }
            visited.insert(node);
        }
    }
    false
}

#[allow(dead_code)]
fn _unused() -> CausalEdge {
    CausalEdge {
        from: String::new(),
        to: String::new(),
        edge_type: CausalEdgeType::Causes,
        strength: 1.0,
        confidence: Default::default(),
        evidence: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::CausalGraphEngine;
    use nexus_cog_core::causal::{CausalEdge, CausalEdgeType, CausalNode, CausalNodeType};
    use nexus_cog_core::common::Confidence;

    fn build() -> CausalGraphEngine {
        let mut e = CausalGraphEngine::new();
        for id in ["root_cause", "intermediate", "bug"] {
            e.add_node(CausalNode {
                id: id.into(),
                node_type: CausalNodeType::Bug,
                name: id.into(),
                description: String::new(),
                file: None,
                line: None,
                confidence: Confidence::new(1.0),
                tags: vec![],
            });
        }
        e.add_edge(CausalEdge {
            from: "root_cause".into(),
            to: "intermediate".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        e.add_edge(CausalEdge {
            from: "intermediate".into(),
            to: "bug".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        e
    }

    #[test]
    fn causes_of_walks_back() {
        let e = build();
        let r = BackwardReasoner::new(e);
        let causes = r.causes_of("bug");
        assert_eq!(causes.len(), 2);
    }

    #[test]
    fn explain_returns_text() {
        let e = build();
        let r = BackwardReasoner::new(e);
        let s = r.explain("bug");
        assert!(s.contains("Why"));
    }

    #[test]
    fn immediate_causes_only_direct() {
        let e = build();
        let r = BackwardReasoner::new(e);
        let immediate = r.immediate_causes("bug");
        assert_eq!(immediate.len(), 1);
        assert_eq!(immediate[0].id, "intermediate");
    }
}
