//! Unified error type (`thiserror`) shared by the crates in this workspace.

use thiserror::Error;

/// Core error.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

/// Convenience constructor.
impl CoreError {
    pub fn msg(s: impl Into<String>) -> Self {
        CoreError::Other(s.into())
    }
}

/// Result alias.
pub type CoreResult<T> = Result<T, CoreError>;
