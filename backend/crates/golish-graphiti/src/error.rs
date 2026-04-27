//! Error types for the graph knowledge base.

use thiserror::Error;

/// All fallible operations on the graph return [`GraphError`].
#[derive(Debug, Error)]
pub enum GraphError {
    /// Underlying SQL/database failure.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// JSON (de)serialization failure (e.g. invalid `properties` payload).
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// The supplied `entity_type` string is not one of the supported variants.
    #[error("unknown entity type: {0}")]
    UnknownEntityType(String),

    /// The supplied `relation_type` string is not one of the supported variants.
    #[error("unknown relation type: {0}")]
    UnknownRelationType(String),

    /// Lookup target (entity, relation, ...) was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// The caller passed an argument that violates an invariant (e.g. `max_depth <= 0`).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}
