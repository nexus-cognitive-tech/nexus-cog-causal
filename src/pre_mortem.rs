//! Pre-mortem analysis: imagine the project failed, work backward to why.
//!
//! All scenarios are derived from the live causal graph. The graph's ancestors
//! of `subject` form the `causal_chain` of each scenario — no template
//! fabrication.

use nexus_cog_core::causal::{CausalNodeType, FailureScenario, PreMortemReport};
use nexus_cog_core::common::Severity;
use indexmap::IndexMap;
use std::collections::HashSet;

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

    fn generate_scenarios(&self, subject: &str) -> Vec<FailureScenario> {
        let mut scenarios = Vec::new();
        // `backward_closure` already includes `subject` plus every ancestor;
        // we don't need to push the subject a second time and we don't want
        // duplicate entries in the rendered chain.
        let subject_chain: Vec<String> = if self.engine.node(subject).is_some() {
            self.engine.backward_closure(subject).into_iter().collect()
        } else {
            Vec::new()
        };

        // Assumptions are the most likely failure mode: they were never
        // validated.
        for n in &self.engine.nodes_of_type(CausalNodeType::Assumption) {
            let chain = append_unique(&subject_chain, &n.id);
            scenarios.push(FailureScenario {
                id: format!("fs-assumption-{}", n.id),
                title: format!("Assumption `{}` is violated", n.name),
                description: format!(
                    "`{}` was never validated. In production it turns out to be false, and the failure propagates to `{}`.",
                    n.name,
                    subject
                ),
                likelihood: 0.6,
                impact: Severity::High,
                time_horizon_days: 90,
                warning_signs: vec![format!("Counter-example to `{}` observed", n.description)],
                mitigations: vec![
                    "Validate the assumption at runtime".into(),
                    "Add monitoring for related metrics".into(),
                ],
                causal_chain: chain,
            });
        }

        // Invariants that break are usually catastrophic.
        for n in &self.engine.nodes_of_type(CausalNodeType::Invariant) {
            let chain = append_unique(&subject_chain, &n.id);
            scenarios.push(FailureScenario {
                id: format!("fs-invariant-{}", n.id),
                title: format!("Invariant `{}` is broken", n.name),
                description: format!(
                    "The invariant `{}` was thought to always hold but breaks under `{}`, exposing a hidden bug.",
                    n.name,
                    subject
                ),
                likelihood: 0.4,
                impact: Severity::Critical,
                time_horizon_days: 30,
                warning_signs: vec![
                    "Test failures".to_string(),
                    "Anomalous metric values".to_string(),
                ],
                mitigations: vec![
                    "Add invariant checks at startup".into(),
                    "Property-based tests".into(),
                ],
                causal_chain: chain,
            });
        }

        // Constraints under load are the next big class.
        for n in &self.engine.nodes_of_type(CausalNodeType::Constraint) {
            let chain = append_unique(&subject_chain, &n.id);
            scenarios.push(FailureScenario {
                id: format!("fs-constraint-{}", n.id),
                title: format!("Constraint `{}` is exceeded", n.name),
                description: format!(
                    "We depended on `{}`, but under load (or growth) it is exceeded and `{}` fails.",
                    n.name,
                    subject
                ),
                likelihood: 0.5,
                impact: Severity::High,
                time_horizon_days: 180,
                warning_signs: vec!["Resource saturation".to_string()],
                mitigations: vec!["Backpressure".into(), "Auto-scaling".into()],
                causal_chain: chain,
            });
        }

        // Known bugs almost always come back.
        for n in &self.engine.nodes_of_type(CausalNodeType::Bug) {
            let chain = append_unique(&subject_chain, &n.id);
            scenarios.push(FailureScenario {
                id: format!("fs-bug-{}", n.id),
                title: format!("Bug `{}` resurfaces", n.name),
                description: format!(
                    "Known bug `{}` reappears after a partial fix, contributing to the `{}` failure.",
                    n.name,
                    subject
                ),
                likelihood: 0.3,
                impact: Severity::Medium,
                time_horizon_days: 60,
                warning_signs: vec!["Regressions in related metrics".to_string()],
                mitigations: vec!["Add regression test".into()],
                causal_chain: chain,
            });
        }

        // If we discovered any scenarios, expose the unique node IDs they touch.
        let _touched: HashSet<String> = scenarios
            .iter()
            .flat_map(|s| s.causal_chain.iter().cloned())
            .collect();

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

/// Clone `chain` and append `extra` only if it isn't already present.
///
/// `backward_closure(subject)` already contains `subject`, so the previous
/// implementation produced `causal_chain`s with the subject id appearing
/// twice. This helper keeps each chain a strict linear order — no duplicate
/// ids, no accidental cycles from re-injecting an ancestor at the tail.
fn append_unique(chain: &[String], extra: &str) -> Vec<String> {
    let mut out = chain.to_vec();
    if !out.iter().any(|s| s == extra) {
        out.push(extra.to_string());
    }
    out
}

#[cfg(test)]
mod chain_tests {
    use super::append_unique;

    #[test]
    fn appends_when_missing() {
        let v = vec!["a".into(), "b".into()];
        let r = append_unique(&v, "c");
        assert_eq!(r, vec!["a", "b", "c"]);
    }

    #[test]
    fn skips_when_present() {
        let v = vec!["a".into(), "b".into()];
        let r = append_unique(&v, "a");
        assert_eq!(r, vec!["a", "b"]);
    }

    #[test]
    fn works_on_empty_chain() {
        let r = append_unique(&[], "a");
        assert_eq!(r, vec!["a"]);
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
    fn empty_graph_yields_zero_scenarios() {
        let e = CausalGraphEngine::in_memory().unwrap();
        let r = PreMortemEngine::new(e).run("deploy v1");
        assert_eq!(r.scenarios.len(), 0);
        assert_eq!(r.overall_risk, 0.0);
    }

    #[test]
    fn assumption_produces_scenario_with_causal_chain() {
        let mut e = CausalGraphEngine::in_memory().unwrap();
        e.add_node(CausalNode {
            id: "subject".into(),
            node_type: CausalNodeType::Feature,
            name: "checkout flow".into(),
            description: String::new(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        });
        e.add_node(CausalNode {
            id: "a1".into(),
            node_type: CausalNodeType::Assumption,
            name: "users are authenticated".into(),
            description: "session cookie is valid".into(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        });
        e.add_edge(CausalEdge {
            from: "a1".into(),
            to: "subject".into(),
            edge_type: CausalEdgeType::Enables,
            strength: 1.0,
            confidence: Confidence::new(1.0),
            evidence: vec![],
        });
        let r = PreMortemEngine::new(e).run("subject");
        assert!(r.scenarios.iter().any(|s| s.title.contains("users are authenticated")));
        let scenario = r.scenarios.iter().find(|s| s.title.contains("users are authenticated")).unwrap();
        assert!(scenario.causal_chain.contains(&"a1".to_string()));
        assert!(scenario.causal_chain.contains(&"subject".to_string()));
    }

    #[test]
    fn causal_chain_has_no_duplicates() {
        // Regression: previously `causal_chain` contained `subject` twice
        // because `backward_closure` already includes it and the generator
        // pushed it again at the tail.
        let mut e = CausalGraphEngine::in_memory().unwrap();
        for id in ["subject", "a1", "a2"] {
            e.add_node(CausalNode {
                id: id.into(),
                node_type: if id == "subject" { CausalNodeType::Feature } else { CausalNodeType::Assumption },
                name: id.into(),
                description: String::new(),
                file: None,
                line: None,
                confidence: Confidence::new(1.0),
                tags: vec![],
            });
        }
        for tail in ["a1", "a2"] {
            e.add_edge(CausalEdge {
                from: tail.into(),
                to: "subject".into(),
                edge_type: CausalEdgeType::Enables,
                strength: 1.0,
                confidence: Confidence::new(1.0),
                evidence: vec![],
            });
        }
        let r = PreMortemEngine::new(e).run("subject");
        for s in &r.scenarios {
            let mut sorted = s.causal_chain.clone();
            sorted.sort();
            let orig_len = sorted.len();
            sorted.dedup();
            assert_eq!(sorted.len(), orig_len, "chain has dup ids: {:?}", s.causal_chain);
        }
    }

    #[test]
    fn top_risks_bounded_to_five() {
        let e = CausalGraphEngine::in_memory().unwrap();
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
