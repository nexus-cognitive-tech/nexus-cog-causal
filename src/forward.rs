//! Forward causal reasoning: "if I change X, what else breaks?"

use nexus_cog_core::causal::CausalNode;
use indexmap::IndexMap;

use crate::graph::CausalGraphEngine;

/// Computes the forward causal impact of a change.
#[derive(Debug, Clone)]
pub struct ForwardReasoner {
    engine: CausalGraphEngine,
}

impl ForwardReasoner {
    /// Construct a forward reasoner.
    #[must_use]
    pub fn new(engine: CausalGraphEngine) -> Self {
        Self { engine }
    }

    /// What breaks if we change `entity`?
    #[must_use]
    pub fn impact_of(&self, entity: &str) -> Vec<CausalNode> {
        let ids = self.engine.forward_closure(entity);
        self.engine
            .nodes()
            .into_iter()
            .filter(|n| ids.contains(&n.id) && n.id != entity)
            .collect()
    }

    /// Compute the cascade depth — how many hops does the impact reach?
    #[must_use]
    pub fn cascade_depth(&self, entity: &str) -> usize {
        use petgraph::visit::EdgeRef;
        let guard = self.engine.read();
        let g = guard.graph();
        let Some(start) = guard.index_of(entity) else {
            return 0;
        };
        // DFS to find longest path from start.
        fn dfs_depth(
            g: &petgraph::Graph<nexus_cog_core::causal::CausalNode, nexus_cog_core::causal::CausalEdge>,
            node: petgraph::graph::NodeIndex,
            visited: &mut std::collections::HashSet<petgraph::graph::NodeIndex>,
        ) -> usize {
            if !visited.insert(node) {
                return 0;
            }
            let mut max_depth = 0;
            for edge in g.edges_directed(node, petgraph::Direction::Outgoing) {
                let depth = dfs_depth(g, edge.target(), visited);
                max_depth = max_depth.max(depth + 1);
            }
            visited.remove(&node);
            max_depth
        }
        let mut visited = std::collections::HashSet::new();
        dfs_depth(g, start, &mut visited)
    }

    /// Group impacts by type.
    #[must_use]
    pub fn impact_by_type(&self, entity: &str) -> IndexMap<&'static str, Vec<String>> {
        let mut out: IndexMap<&'static str, Vec<String>> = IndexMap::new();
        for n in self.impact_of(entity) {
            let key: &'static str = match n.node_type {
                nexus_cog_core::causal::CausalNodeType::CodeEntity => "code",
                nexus_cog_core::causal::CausalNodeType::Behavior => "behavior",
                nexus_cog_core::causal::CausalNodeType::Feature => "feature",
                nexus_cog_core::causal::CausalNodeType::Invariant => "invariant",
                nexus_cog_core::causal::CausalNodeType::Assumption => "assumption",
                nexus_cog_core::causal::CausalNodeType::Decision => "decision",
                nexus_cog_core::causal::CausalNodeType::Constraint => "constraint",
                nexus_cog_core::causal::CausalNodeType::Bug => "bug",
                nexus_cog_core::causal::CausalNodeType::ExternalDep => "external_dep",
            };
            out.entry(key).or_default().push(n.id.clone());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::CausalGraphEngine;
    use nexus_cog_core::causal::{CausalEdge, CausalEdgeType, CausalNode, CausalNodeType};
    use nexus_cog_core::common::Confidence;

    fn build() -> CausalGraphEngine {
        let mut e = CausalGraphEngine::in_memory().unwrap();
        e.add_node(CausalNode {
            id: "a".into(),
            node_type: CausalNodeType::CodeEntity,
            name: "a".into(),
            description: String::new(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        });
        e.add_node(CausalNode {
            id: "b".into(),
            node_type: CausalNodeType::Behavior,
            name: "b".into(),
            description: String::new(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        });
        e.add_edge(CausalEdge {
            from: "a".into(),
            to: "b".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        e
    }

    #[test]
    fn impact_of_finds_descendants() {
        let e = build();
        let r = ForwardReasoner::new(e);
        let impact = r.impact_of("a");
        assert_eq!(impact.len(), 1);
        assert_eq!(impact[0].id, "b");
    }

    #[test]
    fn cascade_depth_counts() {
        let e = build();
        let r = ForwardReasoner::new(e);
        assert_eq!(r.cascade_depth("a"), 1);
    }
}
