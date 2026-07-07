//! Error types for `cog-causal`.

use thiserror::Error;

/// Result alias for `cog-causal`.
pub type CausalResult<T> = Result<T, CausalError>;

/// Errors produced by the causal engines.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CausalError {
    /// Node not found.
    #[error("causal node `{0}` not found")]
    NodeNotFound(String),

    /// Edge not found.
    #[error("causal edge from `{from}` to `{to}` not found")]
    EdgeNotFound {
        /// Source node ID.
        from: String,
        /// Target node ID.
        to: String,
    },

    /// Invalid input.
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
