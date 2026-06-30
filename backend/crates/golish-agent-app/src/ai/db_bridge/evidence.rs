//! `GolishDbRepoProvider` evidence-ledger methods (P0 · OpenFang hash chain).
//!
//! `evidence_append_impl` orchestrates the golish-pentest hash-chain append over
//! the shared `PgPool`; `evidence_existing_ids_impl` backs the harness gate's
//! fabricated-ref check via a golish-db query. Both are surfaced on
//! `DbRepoProvider` (see `mod.rs`) so the orchestrator/runtime reach the ledger
//! without holding a raw pool.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use golish_agent_kit::harness::SourceQueryFact;
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
        facts: Option<(&str, &str, &str)>,
    ) -> anyhow::Result<i64> {
        // MVP scope service: InMemory default-InScope. The production
        // `organizations.scope_rules` lookup is the deferred Task 7 of the P0
        // plan; swapping it in later does not change this call site.
        let scope = InMemoryScopeService::new(ScopeVersion::new(1));
        let (technique, asset, outcome) = match facts {
            Some((t, a, o)) => (Some(t), Some(a), Some(o)),
            None => (None, None, None),
        };
        let input = EvidenceInput {
            kind,
            subject,
            raw_output,
            tool_name,
            operation_id,
            stage_run_id,
            project_path,
            session_id,
            technique,
            asset,
            outcome,
        };
        let eid = append(&self.pool, &scope, input)
            .await
            .map_err(|e| anyhow::anyhow!("evidence append failed: {e}"))?;
        Ok(eid.as_i64())
    }

    /// PR2 任务 2.5 · 只读投影源: 本会话三列齐全的证据事实.
    pub(crate) async fn evidence_facts_for_session_impl(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<(String, String, String, i64)>> {
        let rows =
            golish_db::repo::audit::evidence_facts_for_session(&self.pool, session_id).await?;
        Ok(rows)
    }

    /// PR-C step2b（#4 / E3，设计 2026-06-23-technique-outcomes-provenance）：upsert 一条
    /// 覆盖结局 + provenance 进 `technique_outcomes`。EAS LIVENESS 使用 gate-compatible
    /// endpoint key（保留 URL port/path），其它 host-level technique 仍过
    /// `canonical_asset_key` 归一。`collected_at` 取当前时刻；`result_count`/`confidence`
    /// 暂留 None（落点暂无该信号）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn upsert_technique_outcome_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
        asset: &str,
        technique: &str,
        outcome: &str,
        source: Option<&str>,
        query: Option<&str>,
        evidence_ids: &[i64],
    ) -> anyhow::Result<()> {
        let canonical = if technique == golish_agent_kit::harness::evidence_facts::TECH_EAS_LIVENESS
        {
            golish_agent_kit::harness::evidence_facts::eas_liveness_asset_key(asset)
                .unwrap_or_else(|| asset.to_string())
        } else {
            golish_pentest_domain::canonical_asset_key(asset)
                .map(|k| k.key)
                .unwrap_or_else(|| asset.to_string())
        };
        let write = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
            organization_id,
            run_id: run_id.to_string(),
            asset: canonical,
            technique: technique.to_string(),
            outcome: outcome.to_string(),
            source: source.map(str::to_string),
            query: query.map(str::to_string),
            result_count: None,
            confidence: None,
            evidence_ids: evidence_ids.to_vec(),
            collected_at: Some(chrono::Utc::now()),
        };
        golish_db::repo::technique_outcomes::upsert(&self.pool, &write).await
    }

    /// #5（设计 2026-06-23-source-query-log）：upsert 一条被动情报「源查询」进
    /// `source_query_log`（逐源查询日志，比 `technique_outcomes` 更细）。非空 `target` 在此
    /// 过 `canonical_asset_key` 归一（E1）；org 级查询（空串 target）原样保留。`finished_at`
    /// 取当前时刻；`result_count` / `started_at` / `detail` 暂留 None（命令路径暂无该信号）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn upsert_source_query_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
        source: &str,
        query: &str,
        target: &str,
        technique: Option<&str>,
        status: &str,
        evidence_ids: &[i64],
    ) -> anyhow::Result<()> {
        let canonical_target = if target.is_empty() {
            String::new()
        } else {
            golish_pentest_domain::canonical_asset_key(target)
                .map(|k| k.key)
                .unwrap_or_else(|| target.to_string())
        };
        let write = golish_db::repo::source_query_log::SourceQueryLogWrite {
            organization_id,
            run_id: run_id.to_string(),
            source: source.to_string(),
            query: query.to_string(),
            target: canonical_target,
            technique: technique.map(str::to_string),
            status: status.to_string(),
            result_count: None,
            evidence_ids: evidence_ids.to_vec(),
            detail: None,
            started_at: None,
            finished_at: Some(chrono::Utc::now()),
        };
        golish_db::repo::source_query_log::upsert(&self.pool, &write).await
    }

    /// #6（设计 2026-06-23-expansion-queue）：enqueue 一条「待扩展线索」进
    /// `expansion_queue`。入队恒 `status="pending"`（冲突时 SQL 不重置 status）；
    /// `discovered_at` 取当前时刻；`detail` 暂留 None。`lead_value` 不过
    /// canonical_asset_key（子公司线索是公司名，非 in-scope 主机）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn enqueue_expansion_lead_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
        lead_type: &str,
        lead_value: &str,
        source: Option<&str>,
        confidence: Option<f32>,
        evidence_ids: &[i64],
    ) -> anyhow::Result<()> {
        let write = golish_db::repo::expansion_queue::ExpansionLeadWrite {
            organization_id,
            run_id: run_id.to_string(),
            lead_type: lead_type.to_string(),
            lead_value: lead_value.to_string(),
            source: source.map(str::to_string),
            confidence,
            status: "pending".to_string(),
            evidence_ids: evidence_ids.to_vec(),
            detail: None,
            discovered_at: Some(chrono::Utc::now()),
        };
        golish_db::repo::expansion_queue::enqueue(&self.pool, &write).await
    }

    /// PR-D（#4 / E3）：读某 `(org, run)` 的 technique_outcomes 行 → coverage 投影元组
    /// `(asset, technique, outcome, evidence_id)`。`evidence_id` 取 `evidence_ids` 首个
    /// （无则 0 哨兵）。fail-safe：读失败 → 空 + warn（gate 退回 coverage_truth/ledger）。
    pub(crate) async fn technique_outcome_facts_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
    ) -> Vec<(String, String, String, i64)> {
        match golish_db::repo::technique_outcomes::list_for_run(&self.pool, organization_id, run_id)
            .await
        {
            Ok(rows) => rows
                .into_iter()
                .map(|r| {
                    let eid = r.evidence_ids.first().copied().unwrap_or(0);
                    (r.asset, r.technique, r.outcome, eid)
                })
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    error = %e,
                    "technique_outcome_facts read failed; gate runs without technique_outcomes projection"
                );
                Vec::new()
            }
        }
    }

    /// #5 Phase 3（provider-source closure）：读 `source_query_log` 的 terminal source
    /// rows → gate/source-coverage 只读 facts。fail-safe：读失败返回空，让 coverage
    /// gate 退回其它证据路径；日志保留定位信息。
    pub(crate) async fn source_query_facts_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
    ) -> Vec<SourceQueryFact> {
        match golish_db::repo::source_query_log::list_for_run(&self.pool, organization_id, run_id)
            .await
        {
            Ok(rows) => rows
                .into_iter()
                .map(|r| SourceQueryFact {
                    source: r.source,
                    query: r.query,
                    target: r.target,
                    technique: r.technique,
                    status: r.status,
                    evidence_ids: r.evidence_ids,
                })
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    error = %e,
                    "source_query_facts read failed; source_coverage gate runs without source-query projection"
                );
                Vec::new()
            }
        }
    }

    pub(crate) async fn evidence_existing_ids_impl(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<HashSet<i64>> {
        let found = golish_db::repo::audit::existing_evidence_ids(&self.pool, ids).await?;
        Ok(found.into_iter().collect())
    }

    pub(crate) async fn recent_evidence_ids_impl(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<i64>> {
        let rows =
            golish_db::repo::audit::recent_evidence_ids_for_session(&self.pool, session_id, limit)
                .await?;
        Ok(rows)
    }

    pub(crate) async fn evidence_kinds_for_impl(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, String>> {
        let rows = golish_db::repo::audit::evidence_kinds_for(&self.pool, ids).await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, kind)| kind.map(|k| (id, k)))
            .collect())
    }

    pub(crate) async fn evidence_ages_for_impl(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, std::time::Duration>> {
        let rows = golish_db::repo::audit::evidence_ages_for(&self.pool, ids).await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, secs)| {
                // Negative age (clock skew) or NULL → drop; the gate treats a
                // missing age as "unknown" and does not block on it.
                secs.filter(|s| *s >= 0.0)
                    .map(|s| (id, std::time::Duration::from_secs_f64(s)))
            })
            .collect())
    }
}

/// P2 · expose the ledger existence check to the `submit_stage_deliverable`
/// tool via its narrow [`EvidenceLedgerQuery`] seam (no full `DbRepoProvider`
/// dependency from the tool).
#[async_trait::async_trait]
impl crate::ai::harness_submit_tool::EvidenceLedgerQuery for GolishDbRepoProvider {
    async fn existing_evidence_ids(&self, ids: &[i64]) -> anyhow::Result<HashSet<i64>> {
        self.evidence_existing_ids_impl(ids).await
    }

    async fn recent_evidence_ids(&self, session_id: &str, limit: i64) -> anyhow::Result<Vec<i64>> {
        self.recent_evidence_ids_impl(session_id, limit).await
    }

    async fn evidence_kinds_for(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<std::collections::HashMap<i64, String>> {
        self.evidence_kinds_for_impl(ids).await
    }

    async fn evidence_facts(
        &self,
        session_id: &str,
    ) -> Vec<golish_agent_kit::harness::EvidenceFact> {
        use golish_agent_kit::harness::{EvidenceFact, EvidenceOutcome};
        match self.evidence_facts_for_session_impl(session_id).await {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|(asset, technique, outcome, evidence_id)| {
                    let outcome = match outcome.as_str() {
                        "found" => EvidenceOutcome::Found,
                        "empty" => EvidenceOutcome::Empty,
                        // T2：失败检查（gray-switch GOLISH_FAILURE_OUTCOME_ERROR）记 error。
                        "error" => EvidenceOutcome::Error,
                        _ => return None,
                    };
                    Some(EvidenceFact {
                        asset,
                        technique,
                        outcome,
                        evidence_id,
                    })
                })
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    error = %e,
                    "evidence_facts_for_session failed; submit gate preview runs without projection"
                );
                Vec::new()
            }
        }
    }

    async fn db_truth_facts(
        &self,
        org_id: Option<uuid::Uuid>,
        assets: &[String],
    ) -> Vec<golish_agent_kit::harness::EvidenceFact> {
        self.db_truth_facts_with_run_start(org_id, assets, None)
            .await
    }

    async fn db_truth_facts_with_run_start(
        &self,
        org_id: Option<uuid::Uuid>,
        assets: &[String],
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<golish_agent_kit::harness::EvidenceFact> {
        use golish_agent_kit::harness::{EvidenceFact, EvidenceOutcome};
        match self.db_truth_facts_impl(org_id, assets, run_start).await {
            // coverage_truth is Found-only (it never infers checked_empty), and
            // the projection is evidence-id-agnostic, so the business-table truth
            // maps to Found facts with the sentinel id 0.
            Ok(rows) => rows
                .into_iter()
                .map(|(asset, technique)| EvidenceFact {
                    asset,
                    technique,
                    outcome: EvidenceOutcome::Found,
                    evidence_id: 0,
                })
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    error = %e,
                    "db_truth_facts failed; submit gate preview runs without the org-truth half"
                );
                Vec::new()
            }
        }
    }

    async fn in_scope_assets(&self, org_id: Option<uuid::Uuid>) -> Vec<String> {
        // org-isolated (`in_scope_values(None, org_id)`), unlike the whole-DB
        // `in_scope_targets`; keeps the submit preview's asset axis to THIS org.
        self.in_scope_assets_impl(org_id).await.unwrap_or_default()
    }

    async fn in_scope_assets_created_before(
        &self,
        org_id: Option<uuid::Uuid>,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Vec<String> {
        self.in_scope_assets_created_before_impl(org_id, cutoff)
            .await
            .unwrap_or_default()
    }

    async fn in_scope_typed_assets(&self, org_id: Option<uuid::Uuid>) -> Vec<(String, String)> {
        // T3 (设计 2026-06-23-submit-preview-authoritative-context): host-aware
        // asset_types for the submit preview (same source as the stage-close gate).
        self.in_scope_typed_assets_impl(org_id)
            .await
            .unwrap_or_default()
    }

    async fn eas_port_delegated_assets(&self, org_id: Option<uuid::Uuid>) -> Vec<String> {
        // 方案 A (设计 2026-06-30-eas-domain-port-delegation): EAS alias delegation
        // for the submit preview (same source as the stage-close gate).
        self.eas_port_delegated_assets_impl(org_id)
            .await
            .unwrap_or_default()
    }

    async fn in_scope_target_types(&self, org_id: Option<uuid::Uuid>) -> Vec<String> {
        // T3: distinct targets.type for the preview's dynamic expected_techniques.
        self.in_scope_target_types_impl(org_id)
            .await
            .unwrap_or_default()
    }

    async fn technique_outcome_facts(
        &self,
        org_id: uuid::Uuid,
        run_id: &str,
    ) -> Vec<(String, String, String, i64)> {
        // PR-D (#4/E3): submit 预检 dual-read 投影源（与 DbRepoProvider 同 impl）。
        self.technique_outcome_facts_impl(org_id, run_id).await
    }

    async fn source_query_facts(&self, org_id: uuid::Uuid, run_id: &str) -> Vec<SourceQueryFact> {
        self.source_query_facts_impl(org_id, run_id).await
    }

    async fn operation_stage_started_at(
        &self,
        operation_id: uuid::Uuid,
    ) -> Option<(
        golish_agent_kit::harness::StageKind,
        chrono::DateTime<chrono::Utc>,
    )> {
        let state = self.operation_state_get_impl(operation_id).await.ok()??;
        let stage = golish_agent_kit::harness::StageKind::try_parse(&state.current_stage)?;
        Some((stage, state.stage_started_at))
    }
}
