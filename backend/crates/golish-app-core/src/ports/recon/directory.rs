//! `ReconDirectoryPort` — recon `directory_entries` existence checks + inserts
//! as a port.
//!
//! The in-proc adapter mirrors `golish_db::repo::directory_entries` exactly. It
//! is the ONLY place the consuming pentest service reaches the recon
//! `directory_entries` repo; it lives under the recon port domain so the
//! ownership guard treats it as recon-owned. The insert (S1-3, mirroring the
//! former `golish_recon_app::targets::db_directory_entry_add`) returns the
//! shared [`DirectoryEntry`] DTO; `DirEntryRow` is a private DB-decode detail.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use golish_core::time::ts_from_dt;
use golish_db::repo::scoped::TargetWriteGuard;
use golish_db::repo::technique_outcomes::TechniqueOutcomeAttemptGuard;

use crate::domain::targets::DirectoryEntry;

#[derive(Debug)]
pub enum ConditionalDirectoryEntryWrite {
    Applied(DirectoryEntry),
    Superseded,
}

/// Outbound port for recon directory-entry existence checks + inserts.
#[async_trait]
pub trait ReconDirectoryPort: Send + Sync {
    async fn directory_entries_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<DirectoryEntry>>;

    async fn directory_entries_list_by_target_project(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
    ) -> anyhow::Result<Vec<DirectoryEntry>>;

    async fn directory_entries_exists_by_url_project(
        &self,
        url: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool>;

    /// Insert (or upsert by `url`+`tool` when `target_id` is set) a directory
    /// entry, returning the row. Mirrors `db_directory_entry_add`.
    #[allow(clippy::too_many_arguments)]
    async fn directory_entry_add(
        &self,
        target_id: Option<Uuid>,
        url: &str,
        status_code: Option<i32>,
        content_length: Option<i32>,
        lines: Option<i32>,
        words: Option<i32>,
        tool: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<DirectoryEntry>;

    /// Target-owned counterpart used by active producers. The adapter locks
    /// and validates the immutable target witness in the same short
    /// transaction as the directory-entry write.
    #[allow(clippy::too_many_arguments)]
    async fn directory_entry_add_guarded(
        &self,
        guard: &TargetWriteGuard,
        url: &str,
        status_code: Option<i32>,
        content_length: Option<i32>,
        lines: Option<i32>,
        words: Option<i32>,
        tool: &str,
    ) -> anyhow::Result<DirectoryEntry>;

    /// Generation-CAS counterpart for long-running route producers. The
    /// adapter writes only while the exact operation epoch and DIR attempt
    /// marker are still current.
    #[allow(clippy::too_many_arguments)]
    async fn directory_entry_add_guarded_if_attempt_current(
        &self,
        guard: &TargetWriteGuard,
        attempt_guard: &TechniqueOutcomeAttemptGuard,
        run_id: &str,
        asset: &str,
        technique: &str,
        url: &str,
        status_code: Option<i32>,
        content_length: Option<i32>,
        lines: Option<i32>,
        words: Option<i32>,
        tool: &str,
    ) -> anyhow::Result<ConditionalDirectoryEntryWrite>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgReconDirectoryAdapter {
    pool: Arc<PgPool>,
}

impl PgReconDirectoryAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

/// DB-row decode for the `directory_entries` projection (`DIR_ENTRY_COLS`).
/// Private to the adapter — the boundary type is the [`DirectoryEntry`] DTO.
#[derive(sqlx::FromRow)]
struct DirEntryRow {
    id: Uuid,
    target_id: Option<Uuid>,
    url: String,
    status_code: Option<i32>,
    content_length: Option<i32>,
    lines: Option<i32>,
    words: Option<i32>,
    content_type: String,
    tool: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<DirEntryRow> for DirectoryEntry {
    fn from(r: DirEntryRow) -> Self {
        DirectoryEntry {
            id: r.id.to_string(),
            target_id: r.target_id.map(|u| u.to_string()),
            url: r.url,
            status_code: r.status_code,
            content_length: r.content_length,
            lines: r.lines,
            words: r.words,
            content_type: r.content_type,
            tool: r.tool,
            created_at: ts_from_dt(r.created_at),
        }
    }
}

#[async_trait]
impl ReconDirectoryPort for PgReconDirectoryAdapter {
    async fn directory_entries_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<DirectoryEntry>> {
        let rows: Vec<DirEntryRow> =
            golish_db::repo::directory_entries::list_by_current_target_owner(
                self.pool.as_ref(),
                target_id,
            )
            .await?;
        Ok(rows.into_iter().map(DirectoryEntry::from).collect())
    }

    async fn directory_entries_list_by_target_project(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
    ) -> anyhow::Result<Vec<DirectoryEntry>> {
        let rows: Vec<DirEntryRow> = golish_db::repo::directory_entries::list_by_target_project(
            self.pool.as_ref(),
            target_id,
            project_path,
        )
        .await?;
        Ok(rows.into_iter().map(DirectoryEntry::from).collect())
    }

    async fn directory_entries_exists_by_url_project(
        &self,
        url: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool> {
        Ok(golish_db::repo::directory_entries::exists_by_url_project(
            self.pool.as_ref(),
            url,
            project_path,
        )
        .await?)
    }

    async fn directory_entry_add(
        &self,
        target_id: Option<Uuid>,
        url: &str,
        status_code: Option<i32>,
        content_length: Option<i32>,
        lines: Option<i32>,
        words: Option<i32>,
        tool: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<DirectoryEntry> {
        let row: DirEntryRow = golish_db::repo::directory_entries::insert_entry(
            self.pool.as_ref(),
            target_id,
            url,
            status_code,
            content_length,
            lines,
            words,
            tool,
            project_path,
        )
        .await?;
        Ok(DirectoryEntry::from(row))
    }

    async fn directory_entry_add_guarded(
        &self,
        guard: &TargetWriteGuard,
        url: &str,
        status_code: Option<i32>,
        content_length: Option<i32>,
        lines: Option<i32>,
        words: Option<i32>,
        tool: &str,
    ) -> anyhow::Result<DirectoryEntry> {
        let row: DirEntryRow = golish_db::repo::directory_entries::insert_entry_guarded(
            self.pool.as_ref(),
            guard,
            url,
            status_code,
            content_length,
            lines,
            words,
            tool,
        )
        .await?;
        Ok(DirectoryEntry::from(row))
    }

    async fn directory_entry_add_guarded_if_attempt_current(
        &self,
        guard: &TargetWriteGuard,
        attempt_guard: &TechniqueOutcomeAttemptGuard,
        run_id: &str,
        asset: &str,
        technique: &str,
        url: &str,
        status_code: Option<i32>,
        content_length: Option<i32>,
        lines: Option<i32>,
        words: Option<i32>,
        tool: &str,
    ) -> anyhow::Result<ConditionalDirectoryEntryWrite> {
        let result: golish_db::repo::directory_entries::ConditionalDirectoryEntryWrite<
            DirEntryRow,
        > = golish_db::repo::directory_entries::insert_entry_guarded_if_attempt_current(
            self.pool.as_ref(),
            guard,
            attempt_guard,
            run_id,
            asset,
            technique,
            url,
            status_code,
            content_length,
            lines,
            words,
            tool,
        )
        .await?;
        Ok(match result {
            golish_db::repo::directory_entries::ConditionalDirectoryEntryWrite::Applied(row) => {
                ConditionalDirectoryEntryWrite::Applied(DirectoryEntry::from(row))
            }
            golish_db::repo::directory_entries::ConditionalDirectoryEntryWrite::Superseded => {
                ConditionalDirectoryEntryWrite::Superseded
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recon_directory_port_is_object_safe() {
        fn _assert(_: &dyn ReconDirectoryPort) {}
    }
}
