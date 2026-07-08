//! Causal reasoning: forward / backward / counterfactual / pre-mortem /
//! blast-radius over a directed graph whose source of truth is SQLite.
//!
//! Every stateful operation goes through `nexus-cog-storage`; the in-memory
//! `petgraph` is a derived cache that mirrors what is on disk.

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
pub mod schema;

pub use backward::BackwardReasoner;
pub use blast_radius::BlastRadiusCalculator;
pub use counterfactual::CounterfactualReasoner;
pub use forward::ForwardReasoner;
pub use graph::CausalGraphEngine;
pub use pre_mortem::PreMortemEngine;
