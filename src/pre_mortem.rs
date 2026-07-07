//! Pre-mortem analysis: imagine the project failed, work backward to why.

use nexus_cog_core::causal::{CausalNodeType, FailureScenario, PreMortemReport};
use nexus_cog_core::common::Severity;
use indexmap::IndexMap;

use crate::graph::CausalGraphEngine;

/// Pre-mortem engine.
#[derive(Debug, Clone)]
pub struct PreMortemEngine {
    engine: CausalGraphEngine,
}

impl PreMortemEngine {
    /// Construct a pre-mortem engine.
    #[must_use]
    pub fn new(engine: CausalGraphEngine) -> Self {
        Self { engine }
    }

    /// Run a pre-mortem on `subject` (e.g. "merging PR #1234", "deploying v2.0").
    #[must_use]
    pub fn run(&self, subject: &str) -> PreMortemReport {
        let scenarios = self.generate_scenarios(subject);
        let mut sorted = scenarios.clone();
        sorted.sort_by(|a, b| {
            let sa = a.likelihood * a.impact.score();
            let sb = b.likelihood * b.impact.score();
            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal).reverse()
        });
        let top_risks = sorted.iter().take(5).cloned().collect();
        let overall_risk = if scenarios.is_empty() {
            0.0
        } else {
            scenarios.iter().map(|s| s.likelihood * s.impact.score()).sum::<f32>() / scenarios.len() as f32
        };
        let recommendations = self.build_recommendations(&scenarios);
        PreMortemReport {
            id: format!("pmortem-{}", uuid::Uuid::new_v4()),
            subject: subject.to_string(),
            scenarios,
            top_risks,
            overall_risk,
            recommendations,
        }
    }

    fn generate_scenarios(&self, _subject: &str) -> Vec<FailureScenario> {
        let mut scenarios = Vec::new();
        let assumptions = self.engine.nodes_of_type(CausalNodeType::Assumption);
        let invariants = self.engine.nodes_of_type(CausalNodeType::Invariant);
        let constraints = self.engine.nodes_of_type(CausalNodeType::Constraint);
        let bugs = self.engine.nodes_of_type(CausalNodeType::Bug);

        for n in &assumptions {
            scenarios.push(FailureScenario {
                id: format!("fs-assumption-{}", n.id),
                title: format!("`{}` proves wrong", n.name),
                description: format!(
                    "The assumption `{}` was violated in production, causing cascading failures.",
                    n.name
                ),
                likelihood: 0.6,
                impact: Severity::High,
                time_horizon_days: 90,
                warning_signs: vec![format!("Violation of: {}", n.description)],
                mitigations: vec![
                    "Validate the assumption at runtime".into(),
                    "Add monitoring for related metrics".into(),
                ],
                causal_chain: vec![n.id.clone()],
            });
        }
        for n in &invariants {
            scenarios.push(FailureScenario {
                id: format!("fs-invariant-{}", n.id),
                title: format!("Invariant `{}` violated", n.name),
                description: format!(
                    "An invariant thought to always hold (`{}`) was broken, exposing a hidden bug.",
                    n.name
                ),
                likelihood: 0.4,
                impact: Severity::Critical,
                time_horizon_days: 30,
                warning_signs: vec!["Test failures".to_string(), "Anomalous metric values".to_string()],
                mitigations: vec!["Add invariant checks at startup".into(), "Property-based tests".into()],
                causal_chain: vec![n.id.clone()],
            });
        }
        for n in &constraints {
            scenarios.push(FailureScenario {
                id: format!("fs-constraint-{}", n.id),
                title: format!("Constraint `{}` exceeded", n.name),
                description: format!(
                    "A constraint we depended on (`{}`) was exceeded under load.",
                    n.name
                ),
                likelihood: 0.5,
                impact: Severity::High,
                time_horizon_days: 180,
                warning_signs: vec!["Resource saturation".to_string()],
                mitigations: vec!["Backpressure".into(), "Auto-scaling".into()],
                causal_chain: vec![n.id.clone()],
            });
        }
        for n in &bugs {
            scenarios.push(FailureScenario {
                id: format!("fs-bug-{}", n.id),
                title: format!("Bug `{}` resurfaces", n.name),
                description: format!(
                    "Known bug `{}` reappears after a partial fix.",
                    n.name
                ),
                likelihood: 0.3,
                impact: Severity::Medium,
                time_horizon_days: 60,
                warning_signs: vec!["Regressions in related metrics".to_string()],
                mitigations: vec!["Add regression test".into()],
                causal_chain: vec![n.id.clone()],
            });
        }

        // Add generic scenarios.
        scenarios.push(FailureScenario {
            id: "fs-generic-load".into(),
            title: "Traffic spike exposes unmeasured hot path".into(),
            description: "A previously-unseen request pattern triggers a slow code path.".into(),
            likelihood: 0.7,
            impact: Severity::High,
            time_horizon_days: 30,
            warning_signs: vec!["p99 latency drift".to_string()],
            mitigations: vec!["Add load tests covering the hot path".into()],
            causal_chain: Vec::new(),
        });
        scenarios.push(FailureScenario {
            id: "fs-generic-dep".into(),
            title: "Upstream dependency breaks the contract".into(),
            description: "An external library or service changes behavior in an incompatible way.".into(),
            likelihood: 0.5,
            impact: Severity::Critical,
            time_horizon_days: 90,
            warning_signs: vec!["Version bump".to_string(), "Breaking change notice".to_string()],
            mitigations: vec!["Pin versions".into(), "Add contract tests".into()],
            causal_chain: Vec::new(),
        });

        // Shuffle deterministically using a simple hash-based permutation.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut seed_state = DefaultHasher::new();
        scenarios.len().hash(&mut seed_state);
        let mut seed = seed_state.finish();
        for i in (1..scenarios.len()).rev() {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let j = (seed as usize) % (i + 1);
            scenarios.swap(i, j);
        }

        scenarios
    }

    fn build_recommendations(&self, scenarios: &[FailureScenario]) -> Vec<String> {
        let mut by_type: IndexMap<&'static str, usize> = IndexMap::new();
        for s in scenarios {
            for m in &s.mitigations {
                *by_type.entry(box_str(m)).or_insert(0) += 1;
            }
        }
        let mut recs: Vec<(String, usize)> = by_type
            .into_iter()
            .map(|(s, c)| (s.to_string(), c))
            .collect();
        recs.sort_by_key(|x| std::cmp::Reverse(x.1));
        recs.into_iter().take(5).map(|(s, _)| s).collect()
    }
}

fn box_str(s: &str) -> &'static str {
    match s {
        "Add monitoring for related metrics" => "monitoring",
        "Validate the assumption at runtime" => "runtime_validation",
        "Add invariant checks at startup" => "invariant_checks",
        "Property-based tests" => "property_tests",
        "Backpressure" => "backpressure",
        "Auto-scaling" => "auto_scaling",
        "Add regression test" => "regression_tests",
        "Add load tests covering the hot path" => "load_tests",
        "Pin versions" => "version_pinning",
        "Add contract tests" => "contract_tests",
        _ => "other",
    }
}

/// Re-export of [`FailureScenario`] for callers.
pub use nexus_cog_core::causal::FailureScenario as PreMortemScenario;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::CausalGraphEngine;
    use nexus_cog_core::causal::{CausalEdge, CausalEdgeType, CausalNode, CausalNodeType};
    use nexus_cog_core::common::Confidence;

    #[test]
    fn empty_graph_yields_only_generic_scenarios() {
        let e = CausalGraphEngine::new();
        let r = PreMortemEngine::new(e).run("deploy v1");
        assert!(r.scenarios.len() >= 2);
    }

    #[test]
    fn assumption_produces_scenario() {
        let mut e = CausalGraphEngine::new();
        e.add_node(CausalNode {
            id: "a1".into(),
            node_type: CausalNodeType::Assumption,
            name: "users are authenticated".into(),
            description: String::new(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        });
        let r = PreMortemEngine::new(e).run("deploy");
        assert!(r.scenarios.iter().any(|s| s.title.contains("users are authenticated")));
    }

    #[test]
    fn top_risks_bounded_to_five() {
        let e = CausalGraphEngine::new();
        let r = PreMortemEngine::new(e).run("x");
        assert!(r.top_risks.len() <= 5);
    }

    #[allow(dead_code)]
    fn _unused() -> CausalEdge {
        CausalEdge {
            from: String::new(),
            to: String::new(),
            edge_type: CausalEdgeType::Causes,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: Vec::new(),
        }
    }
}
