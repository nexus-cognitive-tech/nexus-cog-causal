//! Blast-radius analysis: how much of the system does a change affect?

use nexus_cog_core::causal::{BlastRadius, CausalNode, CausalNodeType};
use indexmap::IndexMap;

use crate::graph::CausalGraphEngine;

/// Computes the blast radius of a change.
#[derive(Debug, Clone)]
pub struct BlastRadiusCalculator {
    engine: CausalGraphEngine,
}

impl BlastRadiusCalculator {
    /// Construct a calculator.
    #[must_use]
    pub fn new(engine: CausalGraphEngine) -> Self {
        Self { engine }
    }

    /// Compute the blast radius of changing `entity`.
    #[must_use]
    pub fn compute(&self, entity: &str) -> BlastRadius {
        let closure = self.engine.forward_closure(entity);
        let affected: Vec<CausalNode> = self
            .engine
            .nodes()
            .into_iter()
            .filter(|n| closure.contains(&n.id) && n.id != entity)
            .collect();

        let mut by_type: IndexMap<String, usize> = IndexMap::new();
        for n in &affected {
            let key = node_type_str(n.node_type).to_string();
            *by_type.entry(key).or_insert(0) += 1;
        }

        let total_nodes = self.engine.node_count().max(1);
        let risk_score = (affected.len() as f32 / total_nodes as f32).clamp(0.0, 1.0);

        let recommendation = if affected.is_empty() {
            "Change is local; safe to proceed with normal review.".into()
        } else if affected.len() < 5 {
            format!(
                "Affects {} entities; coordinate with their owners and add targeted tests.",
                affected.len()
            )
        } else if affected.len() < 20 {
            format!(
                "Affects {} entities. Strongly recommend phased rollout and feature flag.",
                affected.len()
            )
        } else {
            format!(
                "Affects {} entities ({}% of the system). Consider splitting the change or scheduling a longer review window.",
                affected.len(),
                (risk_score * 100.0) as u32
            )
        };

        BlastRadius {
            id: format!("br-{}", uuid::Uuid::new_v4()),
            changed: entity.to_string(),
            affected: affected.iter().map(|n| n.id.clone()).collect(),
            affected_count: affected.len(),
            risk_score,
            by_type,
            recommendation,
        }
    }
}

fn node_type_str(ty: CausalNodeType) -> &'static str {
    match ty {
        CausalNodeType::CodeEntity => "code",
        CausalNodeType::Behavior => "behavior",
        CausalNodeType::Feature => "feature",
        CausalNodeType::Invariant => "invariant",
        CausalNodeType::Assumption => "assumption",
        CausalNodeType::Decision => "decision",
        CausalNodeType::Constraint => "constraint",
        CausalNodeType::Bug => "bug",
        CausalNodeType::ExternalDep => "external_dep",
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
        for id in ["core", "feature_a", "feature_b", "invariant_x"] {
            e.add_node(CausalNode {
                id: id.into(),
                node_type: if id.starts_with("feature") {
                    CausalNodeType::Feature
                } else if id.starts_with("invariant") {
                    CausalNodeType::Invariant
                } else {
                    CausalNodeType::CodeEntity
                },
                name: id.into(),
                description: String::new(),
                file: None,
                line: None,
                confidence: Confidence::new(1.0),
                tags: vec![],
            });
        }
        e.add_edge(CausalEdge {
            from: "core".into(),
            to: "feature_a".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        e.add_edge(CausalEdge {
            from: "core".into(),
            to: "feature_b".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        e.add_edge(CausalEdge {
            from: "feature_a".into(),
            to: "invariant_x".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        e
    }

    #[test]
    fn computes_affected_count() {
        let e = build();
        let r = BlastRadiusCalculator::new(e).compute("core");
        assert_eq!(r.affected_count, 3);
    }

    #[test]
    fn risk_score_reflects_proportion() {
        let e = build();
        let r = BlastRadiusCalculator::new(e).compute("core");
        assert!(r.risk_score > 0.0);
    }

    #[test]
    fn zero_affected_yields_low_risk() {
        let mut e = CausalGraphEngine::new();
        e.add_node(CausalNode {
            id: "isolated".into(),
            node_type: CausalNodeType::CodeEntity,
            name: "isolated".into(),
            description: String::new(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        });
        let r = BlastRadiusCalculator::new(e).compute("isolated");
        assert_eq!(r.affected_count, 0);
        assert_eq!(r.risk_score, 0.0);
    }
}
