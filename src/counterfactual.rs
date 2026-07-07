//! Counterfactual reasoning: "what change would have prevented this outcome?"

use nexus_cog_core::causal::{Counterfactual, CausalNode};

use crate::graph::CausalGraphEngine;

/// Generates counterfactual hypotheses.
#[derive(Debug, Clone)]
pub struct CounterfactualReasoner {
    engine: CausalGraphEngine,
}

impl CounterfactualReasoner {
    /// Construct a reasoner.
    #[must_use]
    pub fn new(engine: CausalGraphEngine) -> Self {
        Self { engine }
    }

    /// Propose counterfactual changes that would have prevented `outcome`.
    #[must_use]
    pub fn propose_counterfactuals(&self, outcome: &str) -> Vec<Counterfactual> {
        let mut out = Vec::new();
        let immediate = self.immediate_causes(outcome);
        for cause in immediate {
            let cf = self.build_counterfactual(outcome, &cause);
            out.push(cf);
        }
        // Add high-level counterfactuals.
        out.push(self.propose_validation_counterfactual(outcome));
        out.push(self.propose_monitoring_counterfactual(outcome));
        out
    }

    /// The single most plausible counterfactual.
    #[must_use]
    pub fn most_plausible(&self, outcome: &str) -> Option<Counterfactual> {
        self.propose_counterfactuals(outcome)
            .into_iter()
            .max_by(|a, b| a.plausibility.partial_cmp(&b.plausibility).unwrap_or(std::cmp::Ordering::Equal))
    }

    fn immediate_causes(&self, outcome: &str) -> Vec<CausalNode> {
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

    fn build_counterfactual(&self, outcome: &str, cause: &CausalNode) -> Counterfactual {
        Counterfactual {
            id: format!("cf-{}", uuid::Uuid::new_v4()),
            outcome: outcome.to_string(),
            proposed_change: format!("Eliminate or guard against `{}`", cause.name),
            reasoning: format!(
                "`{}` is a direct cause of `{}`. Removing or guarding against it would break the causal chain.",
                cause.name, outcome
            ),
            plausibility: 0.7,
            evidence: vec![format!("Direct causal edge from `{}` to `{}`.", cause.name, outcome)],
            alternatives: self.alternative_interventions(cause),
        }
    }

    fn propose_validation_counterfactual(&self, outcome: &str) -> Counterfactual {
        Counterfactual {
            id: format!("cf-validate-{}", uuid::Uuid::new_v4()),
            outcome: outcome.to_string(),
            proposed_change: "Add validation at the system boundary".into(),
            reasoning: format!(
                "Many `{}` outcomes are caused by invalid inputs. Validation at the boundary would prevent the bad state from being created in the first place.",
                outcome
            ),
            plausibility: 0.5,
            evidence: vec!["Validation is a generic, high-leverage intervention.".into()],
            alternatives: vec!["Add input validation".into(), "Add type-level constraints".into()],
        }
    }

    fn propose_monitoring_counterfactual(&self, outcome: &str) -> Counterfactual {
        Counterfactual {
            id: format!("cf-monitor-{}", uuid::Uuid::new_v4()),
            outcome: outcome.to_string(),
            proposed_change: "Add monitoring/alerting on the precursor state".into(),
            reasoning: "Detecting the precursor state early would have enabled mitigation before `outcome` manifested.".into(),
            plausibility: 0.4,
            evidence: vec!["Early warning systems reduce MTTR.".into()],
            alternatives: vec!["Add metric".into(), "Add health check".into()],
        }
    }

    fn alternative_interventions(&self, cause: &CausalNode) -> Vec<String> {
        let mut alts = Vec::new();
        match cause.node_type {
            nexus_cog_core::causal::CausalNodeType::Assumption => {
                alts.push("Document the assumption explicitly".into());
                alts.push("Validate the assumption at runtime".into());
            }
            nexus_cog_core::causal::CausalNodeType::Decision => {
                alts.push("Re-evaluate the decision".into());
                alts.push("Add a decision record (ADR)".into());
            }
            nexus_cog_core::causal::CausalNodeType::Bug => {
                alts.push("Add a regression test".into());
                alts.push("Fix the root cause".into());
            }
            _ => {
                alts.push("Add a guard / invariant".into());
                alts.push("Refactor for clarity".into());
            }
        }
        alts
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
        e.add_node(CausalNode {
            id: "invalid_input".into(),
            node_type: CausalNodeType::Assumption,
            name: "invalid input assumed valid".into(),
            description: "we assumed all input is valid".into(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        });
        e.add_node(CausalNode {
            id: "crash".into(),
            node_type: CausalNodeType::Bug,
            name: "crash on bad input".into(),
            description: "app crashes".into(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        });
        e.add_edge(CausalEdge {
            from: "invalid_input".into(),
            to: "crash".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        e
    }

    #[test]
    fn propose_generates_multiple() {
        let e = build();
        let r = CounterfactualReasoner::new(e);
        let cfs = r.propose_counterfactuals("crash");
        assert!(cfs.len() >= 3);
    }

    #[test]
    fn most_plausible_selects_max() {
        let e = build();
        let r = CounterfactualReasoner::new(e);
        let cf = r.most_plausible("crash");
        assert!(cf.is_some());
    }
}
