//! golish-graphiti: PostgreSQL-backed graph knowledge base.
//!
//! Provides a typed entity/relation graph for storing the security findings
//! that AI agents accumulate during a pentest engagement (hosts, services,
//! vulnerabilities, credentials, techniques, endpoints) together with the
//! directed edges between them (runs_service, has_vulnerability, exploited_by,
//! lateral_move, ...).
//!
//! The graph backs onto the same embedded PostgreSQL instance managed by the
//! `golish-db` crate; consumers pass in an existing [`sqlx::PgPool`] when
//! constructing a [`client::GraphClient`].
//!
//! # Quick Start
//! ```rust,ignore
//! use golish_graphiti::client::GraphClient;
//! use serde_json::json;
//!
//! let graph = GraphClient::new(pool.clone());
//! let host = graph
//!     .upsert_entity("host", "10.0.0.5", json!({"os": "linux"}), Some("proj-1"))
//!     .await?;
//! let svc = graph
//!     .upsert_entity("service", "http/8080", json!({"banner": "nginx"}), Some("proj-1"))
//!     .await?;
//! graph
//!     .upsert_relation(host.id, svc.id, "runs_service", json!({}))
//!     .await?;
//! ```

pub mod client;
pub mod error;
pub mod types;

pub use client::GraphClient;
pub use error::GraphError;
pub use types::{
    EntityType, GraphEntity, GraphQueryResult, GraphRelation, RelationType,
};
