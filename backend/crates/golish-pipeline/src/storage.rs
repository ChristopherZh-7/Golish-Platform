//! Storage trait: the pipeline engine calls these methods to persist
//! tool output to the application database.
//!
//! The main crate implements this trait by delegating to its own
//! `tools::targets::*` + `output_parser` helpers. Tests and headless
//! tools can use [`NoopStorage`] to skip persistence entirely.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::PipelineResult;
use crate::parser::ParsedItem;

/// Callback interface for storing pipeline step output.
///
/// Every method returns `PipelineResult<bool>` where `bool` means
/// "was this a newly-created row" — the orchestrator uses it to compute
/// the `new_count` statistic reported to the frontend.
#[async_trait]
pub trait PipelineStorage: Send + Sync {
    /// Store a freshly discovered target. Returns `Ok(true)` if this
    /// target did not previously exist.
    async fn store_target_from_item(
        &self,
        pool: &PgPool,
        item: &ParsedItem,
        project_path: Option<&str>,
        parent_id: Option<Uuid>,
    ) -> PipelineResult<bool>;

    /// Store recon results (httpx/nmap-style fields) and per-port metadata.
    /// Returns `Ok(true)` if a port row was added.
    async fn store_recon_from_item(
        &self,
        pool: &PgPool,
        item: &ParsedItem,
        project_path: Option<&str>,
    ) -> PipelineResult<bool>;

    /// Store a directory-discovery entry (ffuf / feroxbuster).
    /// Returns `Ok(true)` if this URL was not previously stored for `tool_name`.
    async fn store_dirent_from_item(
        &self,
        pool: &PgPool,
        item: &ParsedItem,
        tool_name: &str,
        project_path: Option<&str>,
    ) -> PipelineResult<bool>;

    /// Store a finding (nuclei template hit, etc.).
    /// Returns `Ok(true)` if the finding is new (not a duplicate).
    async fn store_finding_from_item(
        &self,
        pool: &PgPool,
        item: &ParsedItem,
        tool_name: &str,
        project_path: Option<&str>,
    ) -> PipelineResult<bool>;

    /// Merge crawler-discovered URLs (e.g. from `katana`) into the
    /// project's sitemap store. Errors are logged and swallowed to avoid
    /// aborting a pipeline on a best-effort side-effect.
    async fn merge_urls_into_sitemap(
        &self,
        pool: &PgPool,
        urls: &[String],
        project_path: Option<&str>,
    );
}

/// A storage implementation that silently discards every call.
///
/// Useful for unit tests and any CLI path that only cares about stdout.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopStorage;

#[async_trait]
impl PipelineStorage for NoopStorage {
    async fn store_target_from_item(
        &self,
        _pool: &PgPool,
        _item: &ParsedItem,
        _project_path: Option<&str>,
        _parent_id: Option<Uuid>,
    ) -> PipelineResult<bool> {
        Ok(false)
    }

    async fn store_recon_from_item(
        &self,
        _pool: &PgPool,
        _item: &ParsedItem,
        _project_path: Option<&str>,
    ) -> PipelineResult<bool> {
        Ok(false)
    }

    async fn store_dirent_from_item(
        &self,
        _pool: &PgPool,
        _item: &ParsedItem,
        _tool_name: &str,
        _project_path: Option<&str>,
    ) -> PipelineResult<bool> {
        Ok(false)
    }

    async fn store_finding_from_item(
        &self,
        _pool: &PgPool,
        _item: &ParsedItem,
        _tool_name: &str,
        _project_path: Option<&str>,
    ) -> PipelineResult<bool> {
        Ok(false)
    }

    async fn merge_urls_into_sitemap(
        &self,
        _pool: &PgPool,
        _urls: &[String],
        _project_path: Option<&str>,
    ) {
    }
}
