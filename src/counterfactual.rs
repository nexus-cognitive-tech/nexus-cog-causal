//! Counterfactual reasoning: "what change would have prevented this outcome?"
//!
//! Counterfactuals are derived **only from the causal graph**. If the graph has
//! no ancestors for `outcome`, no counterfactuals are returned — fabrication
//! would be a disservice.

use nexus_cog_core::causal::{Counterfactual, CausalEdgeType, CausalNode};

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
    ///
    /// Returns an empty vector when the graph has no ancestors of `outcome`
    /// or when `outcome` itself is not a known node.
    #[must_use]
    pub fn propose_counterfactuals(&self, outcome: &str) -> Vec<Counterfactual> {
        if self.engine.node(outcome).is_none() {
            return Vec::new();
        }
        let immediate = self.immediate_causes(outcome);
        if immediate.is_empty() {
            return Vec::new();
        }
        immediate
            .into_iter()
            .map(|cause| self.build_counterfactual(outcome, &cause))
            .collect()
    }

    /// The single most plausible counterfactual.
    #[must_use]
    pub fn most_plausible(&self, outcome: &str) -> Option<Counterfactual> {
        self.propose_counterfactuals(outcome)
            .into_iter()
            .max_by(|a, b| a.plausibility.partial_cmp(&b.plausibility).unwrap_or(std::cmp::Ordering::Equal))
    }

    fn immediate_causes(&self, outcome: &str) -> Vec<CausalNode> {
        let Some(idx) = self.engine.index_of(outcome) else {
            return Vec::new();
        };
        self.engine
            .immediate_parents(idx)
            .into_iter()
            .filter_map(|i| self.engine.graph_weight(i))
            .filter(|n| self.edge_to_is_causal(outcome, &n.id))
            .collect()
    }

    /// Keep only causes that arrive via a positive (Causes / Enables / Mitigates) edge.
    fn edge_to_is_causal(&self, _to: &str, from: &str) -> bool {
        // We don't have a direct way to read the edge type from petgraph via
        // the public API, but all edges stored by add_edge default to Causes,
        // and the engine treats Causes/Enables/Mitigates as positive via
        // CausalEdgeType::is_positive(). Treat the mere existence of an edge
        // from `from` to `to` as causal for the purpose of counterfactuals —
        // explicitly negative edges (Prevents / Correlates) would invert
        // intuition, so conservatively filter them out using a node-level
        // proxy: Bug + Assumption + ExternalDep nodes are treated as positive
        // sources; Decision + Constraint are filtered out (they are usually
        // mitigations or enablers rather than direct causes).
        if let Some(node) = self.engine.node(from) {
            !matches!(
                node.node_type,
                nexus_cog_core::causal::CausalNodeType::Decision
                    | nexus_cog_core::causal::CausalNodeType::Constraint
            )
        } else {
            false
        }
    }

    fn build_counterfactual(&self, outcome: &str, cause: &CausalNode) -> Counterfactual {
        let (proposed_change, reasoning, alternatives) = specific_intervention(cause);
        let plausibility = plausibility_for(cause);
        Counterfactual {
            id: format!("cf-{}", uuid::Uuid::new_v4()),
            outcome: outcome.to_string(),
            proposed_change,
            reasoning,
            plausibility,
            evidence: vec![format!(
                "Direct causal edge from `{}` ({:?}) to `{}`.",
                cause.name,
                cause.node_type,
                outcome
            )],
            alternatives,
        }
    }
}

fn specific_intervention(cause: &CausalNode) -> (String, String, Vec<String>) {
    use nexus_cog_core::causal::CausalNodeType::*;
    match cause.node_type {
        Assumption => (
            format!("Validate or constrain the assumption `{}` at runtime", cause.name),
            format!(
                "`{}` is a recorded assumption. Wrapping it with a runtime check converts a silent failure into a hard error before `{}` can occur.",
                cause.name, cause.id
            ),
            vec![
                "Encode the assumption as a precondition check".into(),
                "Add a property-based test".into(),
            ],
        ),
        Bug => (
            format!("Fix the root cause of `{}`", cause.name),
            format!(
                "`{}` is a known bug in the causal chain. A targeted fix removes the edge entirely.",
                cause.name
            ),
            vec!["Add a regression test".into(), "Add an invariant that fails loudly on the buggy state".into()],
        ),
        Behavior => (
            format!("Guard or rate-limit the behavior `{}`", cause.name),
            format!(
                "`{}` is an observable behavior. Wrapping it with a guard converts the chain into a non-event.",
                cause.name
            ),
            vec!["Add circuit breaker".into(), "Add backpressure".into()],
        ),
        Feature => (
            format!("Make `{}` opt-in behind a flag", cause.name),
            format!(
                "`{}` is a feature on the path. Disabling it (or moving it behind a flag) severs the causal chain.",
                cause.name
            ),
            vec!["Add a kill-switch".into(), "Default the flag to off".into()],
        ),
        Invariant => (
            format!("Reinforce the invariant `{}` with a runtime check", cause.name),
            format!(
                "`{}` is supposed to hold. A check at the boundary turns violations into immediate, debuggable failures.",
                cause.name
            ),
            vec!["Add an assertion".into(), "Add a property test".into()],
        ),
        CodeEntity => (
            format!("Refactor or guard `{}`", cause.name),
            format!(
                "`{}` is a code-level cause. Refactoring or adding a precondition breaks the chain.",
                cause.name
            ),
            vec!["Extract behind a trait".into(), "Add an explicit guard".into()],
        ),
        ExternalDep => (
            format!("Add a fallback for `{}`", cause.name),
            format!(
                "`{}` is an external dependency. A fallback (cache, alternate provider) reduces the chance it triggers the outcome.",
                cause.name
            ),
            vec!["Cache the result".into(), "Add a secondary provider".into()],
        ),
        Decision | Constraint => (
            format!("Reconsider `{}`", cause.name),
            format!(
                "`{}` is a design choice on the chain. Documenting it as an ADR makes the trade-off explicit and reviewable.",
                cause.name
            ),
            vec!["Add an ADR".into(), "Review with stakeholders".into()],
        ),
    }
}

fn plausibility_for(cause: &CausalNode) -> f32 {
    use nexus_cog_core::causal::CausalNodeType::*;
    match cause.node_type {
        Bug => 0.9,
        Assumption => 0.8,
        Invariant => 0.8,
        CodeEntity => 0.7,
        Behavior => 0.7,
        ExternalDep => 0.6,
        Feature => 0.5,
        Decision | Constraint => 0.4,
    }
}

// Suppress unused-import warning when the public surface is exercised.
#[allow(dead_code)]
const _: Option<CausalEdgeType> = None;

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
    fn propose_generates_only_graph_derived() {
        let e = build();
        let r = CounterfactualReasoner::new(e);
        let cfs = r.propose_counterfactuals("crash");
        // Only the immediate ancestor → exactly one CF, no template noise.
        assert_eq!(cfs.len(), 1);
        assert!(cfs[0].proposed_change.contains("Validate or constrain"));
    }

    #[test]
    fn most_plausible_selects_max() {
        let e = build();
        let r = CounterfactualReasoner::new(e);
        let cf = r.most_plausible("crash");
        assert!(cf.is_some());
    }
}
