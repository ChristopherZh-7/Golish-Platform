//! Session persistence trait and DB handle.
//!
//! The trait decouples `golish-session` from any specific database library.
//! The application layer provides a concrete implementation (e.g. backed by
//! `sqlx` + PostgreSQL).

use std::sync::Arc;
use uuid::Uuid;

use crate::{GolishSessionSnapshot, SessionListingInfo};

/// Backend for session persistence (save, finalize, list, find, load).
///
/// Implementations handle the actual database operations. The
/// `GolishSessionManager` holds an optional `Arc<dyn SessionPersistence>`
/// for dual-write support alongside file-based archiving.
#[async_trait::async_trait]
pub trait SessionPersistence: Send + Sync {
    async fn save_session(
        &self,
        snapshot: &GolishSessionSnapshot,
        session_uuid: &Uuid,
    ) -> anyhow::Result<()>;

    async fn finalize_session(
        &self,
        snapshot: &GolishSessionSnapshot,
        session_uuid: &Uuid,
    ) -> anyhow::Result<()>;

    async fn list_sessions(&self, limit: usize) -> anyhow::Result<Vec<SessionListingInfo>>;

    async fn find_session(
        &self,
        identifier: &str,
    ) -> anyhow::Result<Option<SessionListingInfo>>;

    async fn load_session(
        &self,
        identifier: &str,
    ) -> anyhow::Result<Option<GolishSessionSnapshot>>;
}

/// Handle stored in `GolishSessionManager` for dual-write.
pub struct DbSessionHandle {
    pub backend: Arc<dyn SessionPersistence>,
    pub session_uuid: Uuid,
}
