//! `ReconTargetsPort` — recon `targets` lookups + domain writes as a service
//! port.
//!
//! The in-proc adapter mirrors `golish_db::repo::targets` exactly (same SQL /
//! IDOR project-scope semantics). It is the ONLY place the consuming pentest /
//! agent / vuln services reach the recon `targets` repo; it lives under the
//! recon port domain so the ownership guard treats it as recon-owned.
//!
//! Reads return serializable scalars; the domain writes (S1-3, mirroring the
//! former `golish_recon_app::targets::db_*` helpers) return the shared
//! [`Target`] DTO from [`crate::domain::targets`]. The `sqlx::FromRow` row
//! adapter (`TargetRow`) is a DB-decode detail private to this adapter.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use golish_core::time::ts_from_dt;

use crate::domain::targets::{detect_type, ReconUpdate, Scope, Target, TargetStatus, TargetType};

fn is_trusted_scoping_intake_source(source: &str) -> bool {
    matches!(
        source.trim().to_ascii_lowercase().as_str(),
        "manual" | "imported" | "customer_provided" | "stage-run-seed" | "seed" | "cli"
    )
}

fn promote_existing_target_to_trusted_intake_sql() -> &'static str {
    r#"UPDATE targets
       SET organization_id = COALESCE(organization_id, $1),
           source = $2,
           updated_at = NOW()
       WHERE id = $3
         AND project_path IS NOT DISTINCT FROM $4
         AND (organization_id = $1 OR organization_id IS NULL)"#
}

/// Outbound port for recon target reads + domain writes.
#[async_trait]
pub trait ReconTargetsPort: Send + Sync {
    async fn targets_find_id_by_value_pair(
        &self,
        value_a: &str,
        value_b: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<Uuid>>;

    async fn targets_find_id_by_value_or_name(
        &self,
        value_or_name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<Uuid>>;

    async fn targets_exists_by_value_exact(
        &self,
        value: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool>;

    async fn targets_match_rows_legacy(
        &self,
        project_path: Option<&str>,
    ) -> anyhow::Result<Vec<(String, serde_json::Value)>>;

    /// Add a target (dedup by value within legacy visibility). Explicit trusted
    /// UI/CLI intake upgrades an exact same-org discovery row in place instead
    /// of creating a duplicate Target. Returns the existing/upgraded or freshly
    /// created [`Target`].
    #[allow(clippy::too_many_arguments)]
    async fn target_add(
        &self,
        name: &str,
        value: &str,
        target_type: Option<&str>,
        grp: Option<&str>,
        owner: Option<&str>,
        time_window_start: Option<chrono::DateTime<chrono::Utc>>,
        time_window_end: Option<chrono::DateTime<chrono::Utc>>,
        organization_id: Option<Uuid>,
        project_path: Option<&str>,
        source: &str,
        parent_id: Option<Uuid>,
    ) -> anyhow::Result<Target>;

    /// List targets for an exact `project_path`. Mirrors `db_target_list`.
    async fn target_list(&self, project_path: Option<&str>) -> anyhow::Result<Vec<Target>>;

    /// Distinct in-scope (`scope='in'`) target `value`s within legacy
    /// visibility. The authoritative in-scope asset set the harness coverage
    /// gate injects (populated by organization recon / manual target-add).
    /// `None` project_path = all visible targets (single-workspace default).
    /// `org_id` narrows to one organization's targets (coverage asset-axis
    /// isolation, design 2026-06-09); `None` = legacy whole-DB set.
    async fn in_scope_values(
        &self,
        project_path: Option<&str>,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>>;

    /// Same as [`ReconTargetsPort::in_scope_values`], but only returns target
    /// rows that existed at or before `cutoff`. Used by stage expansion wave
    /// barrier Phase 1 to freeze the active coverage denominator while newly
    /// discovered targets wait for the next wave.
    async fn in_scope_values_created_before(
        &self,
        project_path: Option<&str>,
        org_id: Option<Uuid>,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<String>>;

    /// In-scope (`scope='in'`) targets as full [`Target`] rows (id + value +
    /// type) within legacy visibility. Lets an agent enumerate recon-collected
    /// assets, then drill into each via `query_target_data(target_id)`. `None`
    /// project_path = all visible targets (single-workspace default).
    async fn in_scope_targets(&self, project_path: Option<&str>) -> anyhow::Result<Vec<Target>>;

    /// Update a target's status by id. Mirrors `db_target_update_status`.
    async fn target_update_status(&self, id: Uuid, status: &str) -> anyhow::Result<()>;

    /// Overwrite a target's `ports` JSON by id. Mirrors `db_target_update_recon`.
    async fn target_update_recon(&self, id: Uuid, ports: &serde_json::Value) -> anyhow::Result<()>;

    /// Apply an extended recon update by id. Mirrors
    /// `db_target_update_recon_extended`.
    async fn target_update_recon_extended(
        &self,
        id: Uuid,
        update: &ReconUpdate,
    ) -> anyhow::Result<()>;

    /// Set a target's `scope` ("in"/"out") by id, restricted to `project_path`
    /// (IDOR guard, AGENTS.md I2). Returns `true` when a target in this project
    /// was updated, `false` when missing / cross-project. Mirrors the
    /// `target_update` command's scoped scope write.
    async fn target_set_scope(
        &self,
        id: Uuid,
        scope: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgReconTargetsAdapter {
    pool: Arc<PgPool>,
}

impl PgReconTargetsAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

/// DB-row decode for the recon `targets` projection (`TARGET_ROW_COLS`). Private
/// to the adapter — the boundary type is the [`Target`] DTO.
#[derive(sqlx::FromRow)]
struct TargetRow {
    id: Uuid,
    name: String,
    target_type: String,
    value: String,
    tags: serde_json::Value,
    notes: String,
    scope: String,
    status: String,
    grp: String,
    owner: String,
    time_window_start: Option<chrono::DateTime<chrono::Utc>>,
    time_window_end: Option<chrono::DateTime<chrono::Utc>>,
    organization_id: Option<Uuid>,
    source: String,
    parent_id: Option<Uuid>,
    ports: serde_json::Value,
    real_ip: String,
    cdn_waf: String,
    http_title: String,
    http_status: Option<i32>,
    webserver: String,
    os_info: String,
    content_type: String,
    liveness_state: Option<String>,
    liveness_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<TargetRow> for Target {
    fn from(r: TargetRow) -> Self {
        Target {
            id: r.id.to_string(),
            name: r.name,
            target_type: TargetType::from_str(&r.target_type),
            value: r.value,
            tags: serde_json::from_value(r.tags).unwrap_or_default(),
            notes: r.notes,
            scope: Scope::from_str(&r.scope),
            status: TargetStatus::from_str(&r.status),
            grp: r.grp,
            owner: r.owner,
            time_window_start: r.time_window_start.map(ts_from_dt),
            time_window_end: r.time_window_end.map(ts_from_dt),
            organization_id: r.organization_id.map(|u| u.to_string()),
            source: r.source,
            parent_id: r.parent_id.map(|u| u.to_string()),
            ports: serde_json::from_value(r.ports).unwrap_or_default(),
            real_ip: r.real_ip,
            cdn_waf: r.cdn_waf,
            http_title: r.http_title,
            http_status: r.http_status,
            webserver: r.webserver,
            os_info: r.os_info,
            content_type: r.content_type,
            liveness_state: r.liveness_state,
            liveness_reason: r.liveness_reason,
            created_at: ts_from_dt(r.created_at),
            updated_at: ts_from_dt(r.updated_at),
        }
    }
}

#[async_trait]
impl ReconTargetsPort for PgReconTargetsAdapter {
    async fn targets_find_id_by_value_pair(
        &self,
        value_a: &str,
        value_b: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<Uuid>> {
        Ok(golish_db::repo::targets::find_id_by_value_pair(
            self.pool.as_ref(),
            value_a,
            value_b,
            project_path,
        )
        .await?)
    }

    async fn targets_find_id_by_value_or_name(
        &self,
        value_or_name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<Uuid>> {
        Ok(golish_db::repo::targets::find_id_by_value_or_name(
            self.pool.as_ref(),
            value_or_name,
            project_path,
        )
        .await?)
    }

    async fn targets_exists_by_value_exact(
        &self,
        value: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool> {
        Ok(
            golish_db::repo::targets::exists_by_value_exact(
                self.pool.as_ref(),
                value,
                project_path,
            )
            .await?,
        )
    }

    async fn targets_match_rows_legacy(
        &self,
        project_path: Option<&str>,
    ) -> anyhow::Result<Vec<(String, serde_json::Value)>> {
        Ok(golish_db::repo::targets::match_rows_legacy(self.pool.as_ref(), project_path).await?)
    }

    async fn target_add(
        &self,
        name: &str,
        value: &str,
        target_type: Option<&str>,
        grp: Option<&str>,
        owner: Option<&str>,
        time_window_start: Option<chrono::DateTime<chrono::Utc>>,
        time_window_end: Option<chrono::DateTime<chrono::Utc>>,
        organization_id: Option<Uuid>,
        project_path: Option<&str>,
        source: &str,
        parent_id: Option<Uuid>,
    ) -> anyhow::Result<Target> {
        let tt = target_type
            .map(TargetType::from_str)
            .unwrap_or_else(|| detect_type(value));
        let n = if name.is_empty() { value } else { name };
        let g = grp
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("default");
        let own = owner.map(str::trim).unwrap_or("");

        if let Some(mut r) = golish_db::repo::targets::find_row_by_value_legacy::<TargetRow>(
            self.pool.as_ref(),
            value,
            project_path,
        )
        .await?
        {
            if !is_trusted_scoping_intake_source(source) {
                return Ok(Target::from(r));
            }

            let same_identity = r.target_type == tt.as_str()
                && (r.organization_id == organization_id || r.organization_id.is_none());
            if same_identity {
                let promoted = sqlx::query(promote_existing_target_to_trusted_intake_sql())
                    .bind(organization_id)
                    .bind(source)
                    .bind(r.id)
                    .bind(project_path)
                    .execute(self.pool.as_ref())
                    .await?;
                if promoted.rows_affected() == 1 {
                    r.organization_id = r.organization_id.or(organization_id);
                    r.source = source.to_string();
                    return Ok(Target::from(r));
                }
            }
            // The legacy visibility probe may have returned a global or
            // different-org row. Trusted intake must not claim it; create an
            // exact row in the caller's own project/org scope below.
        }

        let row: TargetRow = golish_db::repo::targets::insert_full(
            self.pool.as_ref(),
            n,
            tt.as_str(),
            value,
            g,
            own,
            time_window_start,
            time_window_end,
            organization_id,
            project_path,
            source,
            parent_id,
        )
        .await?;
        Ok(Target::from(row))
    }

    async fn target_list(&self, project_path: Option<&str>) -> anyhow::Result<Vec<Target>> {
        let rows: Vec<TargetRow> =
            golish_db::repo::targets::list_rows_by_project_exact(self.pool.as_ref(), project_path)
                .await?;
        Ok(rows.into_iter().map(Target::from).collect())
    }

    async fn in_scope_values(
        &self,
        project_path: Option<&str>,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        Ok(
            golish_db::repo::targets::list_in_scope_values(
                self.pool.as_ref(),
                project_path,
                org_id,
            )
            .await?,
        )
    }

    async fn in_scope_values_created_before(
        &self,
        project_path: Option<&str>,
        org_id: Option<Uuid>,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<String>> {
        Ok(
            golish_db::repo::targets::list_in_scope_values_created_before(
                self.pool.as_ref(),
                project_path,
                org_id,
                cutoff,
            )
            .await?,
        )
    }

    async fn in_scope_targets(&self, project_path: Option<&str>) -> anyhow::Result<Vec<Target>> {
        let rows: Vec<TargetRow> =
            golish_db::repo::targets::list_rows_legacy(self.pool.as_ref(), project_path).await?;
        Ok(rows
            .into_iter()
            .filter(|r| r.scope == "in")
            .map(Target::from)
            .collect())
    }

    async fn target_update_status(&self, id: Uuid, status: &str) -> anyhow::Result<()> {
        Ok(golish_db::repo::targets::update_status_by_id(self.pool.as_ref(), id, status).await?)
    }

    async fn target_update_recon(&self, id: Uuid, ports: &serde_json::Value) -> anyhow::Result<()> {
        Ok(golish_db::repo::targets::update_ports_by_id(self.pool.as_ref(), id, ports).await?)
    }

    async fn target_set_scope(
        &self,
        id: Uuid,
        scope: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool> {
        let updated = golish_db::repo::targets::update_scope_scoped_legacy(
            self.pool.as_ref(),
            id,
            scope,
            project_path,
        )
        .await?;
        Ok(updated > 0)
    }

    async fn target_update_recon_extended(
        &self,
        id: Uuid,
        update: &ReconUpdate,
    ) -> anyhow::Result<()> {
        Ok(golish_db::repo::targets::update_recon_extended_by_id(
            self.pool.as_ref(),
            id,
            &update.real_ip,
            &update.cdn_waf,
            &update.http_title,
            update.http_status,
            &update.webserver,
            &update.os_info,
            &update.content_type,
            &update.ports,
        )
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recon_targets_port_is_object_safe() {
        fn _assert(_: &dyn ReconTargetsPort) {}
    }

    #[test]
    fn trusted_scoping_sources_match_the_active_recon_barrier_contract() {
        for source in [
            "manual",
            "imported",
            "customer_provided",
            "stage-run-seed",
            "seed",
            "cli",
        ] {
            assert!(is_trusted_scoping_intake_source(source), "{source}");
        }
        for source in [
            "asset_intel",
            "active_discovered",
            "external_attack_surface",
        ] {
            assert!(!is_trusted_scoping_intake_source(source), "{source}");
        }
    }

    #[test]
    fn trusted_intake_promotion_is_exact_project_and_org_scoped() {
        let sql = promote_existing_target_to_trusted_intake_sql();
        assert!(sql.contains("project_path IS NOT DISTINCT FROM $4"));
        assert!(sql.contains("organization_id = $1 OR organization_id IS NULL"));
        assert!(sql.contains("source = $2"));
        assert!(
            !sql.contains("scope ="),
            "trusted re-add must not reactivate scope=out"
        );
    }
}
