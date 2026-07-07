//! Causal reasoning engine.
//!
//! The causal engine lets the agent ask real questions about code:
//!
//! - **Forward**: "If I change `X`, what else breaks?"
//! - **Backward**: "Why does this bug exist?"
//! - **Counterfactual**: "What change would have prevented this?"
//! - **Pre-mortem**: "Imagine this fails in 6 months — why?"
//! - **Blast radius**: "How much of the system does this affect?"
//!
//! This is a research-grade capability, packaged as an MCP tool.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod algorithms;
pub mod backward;
pub mod blast_radius;
pub mod counterfactual;
pub mod error;
pub mod forward;
pub mod graph;
pub mod pre_mortem;

pub use algorithms::{
    all_cycles, condensation, shortest_path, strongly_connected_components, topological_sort, Scc,
    SccResult, TopoResult,
};
pub use backward::BackwardReasoner;
pub use blast_radius::BlastRadiusCalculator;
pub use counterfactual::CounterfactualReasoner;
pub use error::{CausalError, CausalResult};
pub use forward::ForwardReasoner;
pub use graph::CausalGraphEngine;
pub use pre_mortem::{PreMortemEngine, PreMortemScenario};
