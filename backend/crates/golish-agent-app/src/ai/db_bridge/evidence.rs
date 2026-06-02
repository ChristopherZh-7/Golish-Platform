//! `GolishDbRepoProvider` evidence-ledger methods (P0 · OpenFang hash chain).
//!
//! `evidence_append_impl` orchestrates the golish-pentest hash-chain append over
//! the shared `PgPool`; `evidence_existing_ids_impl` backs the harness gate's
//! fabricated-ref check via a golish-db query. Both are surfaced on
//! `DbRepoProvider` (see `mod.rs`) so the orchestrator/runtime reach the ledger
//! without holding a raw pool.

use std::collections::HashSet;

use uuid::Uuid;

use golish_pentest::evidence_ledger::append::{append, EvidenceInput};
use golish_pentest::evidence_ledger::{InMemoryScopeService, ScopeVersion};

use super::GolishDbRepoProvider;

impl GolishDbRepoProvider {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn evidence_append_impl(
        &self,
        operation_id: Uuid,
        stage_run_id: Option<Uuid>,
        session_id: Option<&str>,
        project_path: Option<&str>,
        tool_name: &str,
        kind: &str,
        subject: &str,
        raw_output: &str,
    ) -> anyhow::Result<i64> {
        // MVP scope service: InMemory default-InScope. The production
        // `organizations.scope_rules` lookup is the deferred Task 7 of the P0
        // plan; swapping it in later does not change this call site.
        let scope = InMemoryScopeService::new(ScopeVersion::new(1));
        let input = EvidenceInput {
            kind,
            subject,
            raw_output,
            tool_name,
            operation_id,
            stage_run_id,
            project_path,
            session_id,
        };
        let eid = append(&self.pool, &scope, input)
            .await
            .map_err(|e| anyhow::anyhow!("evidence append failed: {e}"))?;
        Ok(eid.as_i64())
    }

    pub(crate) async fn evidence_existing_ids_impl(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<HashSet<i64>> {
        let found = golish_db::repo::audit::existing_evidence_ids(&self.pool, ids).await?;
        Ok(found.into_iter().collect())
    }
}
