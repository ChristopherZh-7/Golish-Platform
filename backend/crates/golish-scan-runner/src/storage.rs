//! Storage callback for scan-runner results.
//!
//! The main crate implements this trait to delegate directory entry
//! storage to its existing `tools::targets` helpers, keeping the
//! scan-runner crate independent of the application's data model.

use async_trait::async_trait;
use golish_db::repo::scoped::TargetWriteGuard;
use sqlx::PgPool;

use crate::error::ScanRunnerResult;

#[async_trait]
pub trait ScanStorage: Send + Sync {
    /// Store a discovered directory entry from feroxbuster.
    async fn store_directory_entry(
        &self,
        pool: &PgPool,
        guard: &TargetWriteGuard,
        url: &str,
        status_code: Option<i32>,
        content_length: Option<i32>,
        lines: Option<i32>,
        words: Option<i32>,
        tool: &str,
    ) -> ScanRunnerResult<()>;
}
