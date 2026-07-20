//! Destructive stage-reset purges for the dev "reset this stage to its start"
//! control (design `docs/design/2026-06-30-stage-reset-full-purge.md`).
//!
//! Every statement here is scoped to the operation's frozen organization units
//! (or the exact operation id for operation-owned artifacts) — there is no bare
//! table wipe. This centralizes the destructive SQL so it is auditable and
//! SQL-shape unit-tested in one place.
//!
//! Layering: this module exposes **data-domain** purges (recon / eas / enumeration
//! / vuln) plus the cross-stage ledger deletes. The harness `StageKind → domain`
//! mapping lives in the command layer (`golish-agent-app`), so this crate stays
//! free of harness stage semantics. Executors accept one `PgConnection`; the
//! runtime checkpoint compound owns that connection so runtime, facts, ledgers,
//! status, and cursor commit or roll back as one unit.
//!
//! Not touched here (by design / user instruction):
//! - `audit_log` (== the evidence ledger AND the audit/run log the user keeps);
//!   coverage re-evaluates from the fact tables below, so the ledger can stay.
//! - `vuln_feeds` / `vuln_entries` (global CVE KB, no engagement scope).
//! - `targets` / `organizations` row deletes (the spine is preserved; we only roll
//!   back status + null per-stage freshness and delete the per-target fact rows).

use crate::Result;
use sqlx::PgConnection;
use uuid::Uuid;

/// Per-statement directly affected row counts for one purge invocation. Rows
/// removed by FK cascade are intentionally not double-counted.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StagePurgeCounts {
    pub target_assets: u64,
    pub dns_records: u64,
    pub passive_scans: u64,
    pub source_query_log: u64,
    pub org_intel_freshness: u64,
    pub fingerprints: u64,
    pub expansion_queue: u64,
    pub eas_target_columns: u64,
    pub network_endpoints: u64,
    pub web_origins: u64,
    pub web_origin_observations: u64,
    pub screenshots: u64,
    pub api_endpoints: u64,
    pub js_analysis: u64,
    pub directory_entries: u64,
    pub crawl_observations: u64,
    pub endpoint_tests: u64,
    pub findings: u64,
    pub vuln_scan_history: u64,
    pub sensitive_scan: u64,
    pub technique_outcomes: u64,
    pub org_stage_completions: u64,
    pub stage_asset_waves: u64,
    pub target_status_rolled_back: u64,
}

impl StagePurgeCounts {
    /// Total rows directly affected by the explicit statements.
    pub fn total(&self) -> u64 {
        self.target_assets
            + self.dns_records
            + self.passive_scans
            + self.source_query_log
            + self.org_intel_freshness
            + self.fingerprints
            + self.expansion_queue
            + self.eas_target_columns
            + self.network_endpoints
            + self.web_origins
            + self.web_origin_observations
            + self.screenshots
            + self.api_endpoints
            + self.js_analysis
            + self.directory_entries
            + self.crawl_observations
            + self.endpoint_tests
            + self.findings
            + self.vuln_scan_history
            + self.sensitive_scan
            + self.technique_outcomes
            + self.org_stage_completions
            + self.stage_asset_waves
            + self.target_status_rolled_back
    }
}

/// Data-only mutable fact domains. Harness `StageKind` mapping intentionally
/// remains in the application layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagePurgeDomain {
    TargetIntel,
    Eas,
    Enumeration,
    Vuln,
}

/// Server-derived plan attached to one compound checkpoint reset. Scope is not
/// caller supplied: the transaction derives exact org ids from the operation's
/// sealed snapshot after locking the operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageCheckpointPurgePlan {
    pub domains: Vec<StagePurgeDomain>,
    pub stage_kinds: Vec<String>,
    pub techniques: Vec<String>,
    pub target_status_floor: Option<String>,
}

// ── SQL builders (table names are compile-time constants, never user input) ──

/// `DELETE` a per-target child table, scoped to the engagement org subtree via a
/// `targets` sub-select on `organization_id`.
fn build_delete_by_target_org_sql(table: &str) -> String {
    format!(
        "DELETE FROM {table} \
         WHERE target_id IN (SELECT id FROM targets WHERE organization_id = ANY($1))"
    )
}

/// `DELETE` a run-owned org table only for the current operation's accepted
/// run aliases (modern operation UUID plus the parent Task session UUID).
fn build_delete_by_org_run_sql(table: &str) -> String {
    format!("DELETE FROM {table} WHERE organization_id = ANY($1) AND run_id = ANY($2)")
}

/// `DELETE` an organization-owned mutable stage fact. These tables have no
/// operation/run key, so the compound reset first rejects every active
/// operation whose frozen scope overlaps the selected operation.
fn build_delete_mutable_org_fact_sql(table: &str) -> String {
    format!("DELETE FROM {table} WHERE organization_id = ANY($1)")
}

fn build_delete_crawl_observations_by_target_org_sql() -> String {
    "DELETE FROM crawl_observations \
      WHERE origin_target_id IN (SELECT id FROM targets WHERE organization_id = ANY($1))"
        .to_string()
}

/// Screenshots carry the exact Task/operation id; project-wide deletion would
/// erase artifacts belonging to sibling operations.
fn build_delete_screenshots_by_operation_sql() -> &'static str {
    "DELETE FROM screenshots WHERE task_id = $1"
}

fn build_reset_org_intel_freshness_sql() -> String {
    "UPDATE organizations \
        SET asns_collected_at = NULL, \
            certificates_collected_at = NULL, \
            whois_collected_at = NULL, \
            osint_collected_at = NULL \
      WHERE id = ANY($1)"
        .to_string()
}

fn build_reset_targets_eas_sql() -> String {
    "UPDATE targets \
        SET real_ip = '', \
            cdn_waf = '', \
            http_title = '', \
            http_status = NULL, \
            webserver = '', \
            os_info = '', \
            content_type = '', \
            ports = '[]'::jsonb, \
            ip_whois = NULL, \
            ports_scanned_at = NULL, \
            liveness_checked_at = NULL, \
            ip_whois_collected_at = NULL, \
            updated_at = NOW() \
      WHERE organization_id = ANY($1)"
        .to_string()
}

fn build_rollback_target_status_sql() -> String {
    // Only downgrade targets that progressed past the floor; enum order is
    // new < passive < active < enumerated < vuln_scan < verified.
    "UPDATE targets \
        SET status = $2::target_status, updated_at = NOW() \
      WHERE organization_id = ANY($1) \
        AND status > $2::target_status"
        .to_string()
}

fn build_delete_stage_asset_waves_sql() -> String {
    // stage_asset_wave_items cascade via FK ON DELETE CASCADE.
    "DELETE FROM stage_asset_waves \
      WHERE operation_id = $1 \
        AND organization_id = ANY($2) \
        AND stage_kind = ANY($3)"
        .to_string()
}

fn build_delete_org_stage_completions_sql() -> String {
    "DELETE FROM org_stage_completions \
      WHERE organization_id = ANY($1) \
        AND stage_kind = ANY($2)"
        .to_string()
}

fn build_delete_technique_outcomes_sql() -> String {
    "DELETE FROM technique_outcomes \
      WHERE organization_id = ANY($1) \
        AND run_id = ANY($2) \
        AND technique = ANY($3)"
        .to_string()
}

// ── Primitive executors ──

async fn delete_by_target_org(
    conn: &mut PgConnection,
    table: &str,
    org_ids: &[Uuid],
) -> Result<u64> {
    if org_ids.is_empty() {
        return Ok(0);
    }
    let res = sqlx::query(&build_delete_by_target_org_sql(table))
        .bind(org_ids)
        .execute(&mut *conn)
        .await?;
    Ok(res.rows_affected())
}

async fn delete_by_org_run(
    conn: &mut PgConnection,
    table: &str,
    org_ids: &[Uuid],
    run_ids: &[String],
) -> Result<u64> {
    if org_ids.is_empty() || run_ids.is_empty() {
        return Ok(0);
    }
    let res = sqlx::query(&build_delete_by_org_run_sql(table))
        .bind(org_ids)
        .bind(run_ids)
        .execute(&mut *conn)
        .await?;
    Ok(res.rows_affected())
}

async fn delete_mutable_org_fact(
    conn: &mut PgConnection,
    table: &str,
    org_ids: &[Uuid],
) -> Result<u64> {
    if org_ids.is_empty() {
        return Ok(0);
    }
    let res = sqlx::query(&build_delete_mutable_org_fact_sql(table))
        .bind(org_ids)
        .execute(&mut *conn)
        .await?;
    Ok(res.rows_affected())
}

// ── Domain purges ──

/// target_intel domain: discovered assets, DNS, passive scans, source-query log,
/// and the org-level intel freshness stamps (so re-collection re-runs).
pub async fn purge_target_intel_domain(
    conn: &mut PgConnection,
    org_ids: &[Uuid],
    run_ids: &[String],
    counts: &mut StagePurgeCounts,
) -> Result<()> {
    counts.target_assets += delete_by_target_org(conn, "target_assets", org_ids).await?;
    counts.dns_records += delete_by_target_org(conn, "dns_records", org_ids).await?;
    counts.passive_scans += delete_by_target_org(conn, "passive_scan_logs", org_ids).await?;
    counts.source_query_log +=
        delete_by_org_run(conn, "source_query_log", org_ids, run_ids).await?;
    if !org_ids.is_empty() {
        let res = sqlx::query(&build_reset_org_intel_freshness_sql())
            .bind(org_ids)
            .execute(&mut *conn)
            .await?;
        counts.org_intel_freshness += res.rows_affected();
    }
    Ok(())
}

/// external_attack_surface domain: reset per-target probe columns + freshness,
/// service fingerprints, the expansion queue, and exact operation screenshots.
pub async fn purge_eas_domain(
    conn: &mut PgConnection,
    operation_id: Uuid,
    org_ids: &[Uuid],
    run_ids: &[String],
    counts: &mut StagePurgeCounts,
) -> Result<()> {
    if !org_ids.is_empty() {
        let res = sqlx::query(&build_reset_targets_eas_sql())
            .bind(org_ids)
            .execute(&mut *conn)
            .await?;
        counts.eas_target_columns += res.rows_affected();
    }
    counts.fingerprints += delete_by_target_org(conn, "fingerprints", org_ids).await?;
    counts.expansion_queue += delete_by_org_run(conn, "expansion_queue", org_ids, run_ids).await?;
    // Delete observations first so their count is explicit; deleting the
    // origin identity then cascades any remaining origin-owned provenance.
    counts.web_origin_observations +=
        delete_mutable_org_fact(conn, "web_origin_observations", org_ids).await?;
    counts.web_origins += delete_mutable_org_fact(conn, "web_origins", org_ids).await?;
    counts.network_endpoints += delete_mutable_org_fact(conn, "network_endpoints", org_ids).await?;
    counts.screenshots += sqlx::query(build_delete_screenshots_by_operation_sql())
        .bind(operation_id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    Ok(())
}

/// enumeration domain: API endpoints (endpoint_tests cascade), JS analysis,
/// directory entries, and any target-scoped endpoint tests left behind.
pub async fn purge_enumeration_domain(
    conn: &mut PgConnection,
    org_ids: &[Uuid],
    counts: &mut StagePurgeCounts,
) -> Result<()> {
    counts.api_endpoints += delete_by_target_org(conn, "api_endpoints", org_ids).await?;
    counts.js_analysis += delete_by_target_org(conn, "js_analysis_results", org_ids).await?;
    counts.directory_entries += delete_by_target_org(conn, "directory_entries", org_ids).await?;
    if !org_ids.is_empty() {
        counts.crawl_observations +=
            sqlx::query(&build_delete_crawl_observations_by_target_org_sql())
                .bind(org_ids)
                .execute(&mut *conn)
                .await?
                .rows_affected();
    }
    counts.endpoint_tests += delete_by_target_org(conn, "endpoint_tests", org_ids).await?;
    Ok(())
}

/// V2 Company Vuln owns no independently purgeable fact table. `findings` are
/// manual or immutable Candidate/Verification truth, while `vuln_scan_history`
/// and legacy sensitive-scan tables have no trustworthy operation/org owner.
/// All are retained. Exact current-run technique outcomes are purged separately.
pub async fn purge_vuln_domain(
    _conn: &mut PgConnection,
    _org_ids: &[Uuid],
    _counts: &mut StagePurgeCounts,
) -> Result<()> {
    Ok(())
}

/// Execute the mutable fact/ledger/status portion of a reset on the
/// caller-owned transaction. `org_ids` are the exact sealed scope units.
pub async fn purge_stage_checkpoint_facts(
    conn: &mut PgConnection,
    operation_id: Uuid,
    org_ids: &[Uuid],
    run_ids: &[String],
    plan: &StageCheckpointPurgePlan,
) -> Result<StagePurgeCounts> {
    let mut counts = StagePurgeCounts::default();
    for domain in [
        StagePurgeDomain::TargetIntel,
        StagePurgeDomain::Eas,
        StagePurgeDomain::Enumeration,
        StagePurgeDomain::Vuln,
    ] {
        if !plan.domains.contains(&domain) {
            continue;
        }
        match domain {
            StagePurgeDomain::TargetIntel => {
                purge_target_intel_domain(conn, org_ids, run_ids, &mut counts).await?
            }
            StagePurgeDomain::Eas => {
                purge_eas_domain(conn, operation_id, org_ids, run_ids, &mut counts).await?
            }
            StagePurgeDomain::Enumeration => {
                purge_enumeration_domain(conn, org_ids, &mut counts).await?
            }
            StagePurgeDomain::Vuln => purge_vuln_domain(conn, org_ids, &mut counts).await?,
        }
    }

    counts.technique_outcomes +=
        delete_technique_outcomes(conn, org_ids, run_ids, &plan.techniques).await?;
    counts.org_stage_completions +=
        delete_org_stage_completions(conn, org_ids, &plan.stage_kinds).await?;
    counts.stage_asset_waves +=
        delete_stage_asset_waves(conn, operation_id, org_ids, &plan.stage_kinds).await?;
    if let Some(floor) = plan.target_status_floor.as_deref() {
        counts.target_status_rolled_back += rollback_target_status(conn, org_ids, floor).await?;
    }
    Ok(counts)
}

// ── Cross-stage ledgers + status rollback ──

/// Delete the per-(org, stage) completion ledger rows for the affected stages so
/// the resume oracle stops skipping them.
pub async fn delete_org_stage_completions(
    conn: &mut PgConnection,
    org_ids: &[Uuid],
    stage_kinds: &[String],
) -> Result<u64> {
    if org_ids.is_empty() || stage_kinds.is_empty() {
        return Ok(0);
    }
    let res = sqlx::query(&build_delete_org_stage_completions_sql())
        .bind(org_ids)
        .bind(stage_kinds)
        .execute(&mut *conn)
        .await?;
    Ok(res.rows_affected())
}

/// Delete durable stage asset wave snapshots (items cascade) for the affected
/// stages of one operation.
pub async fn delete_stage_asset_waves(
    conn: &mut PgConnection,
    operation_id: Uuid,
    org_ids: &[Uuid],
    stage_kinds: &[String],
) -> Result<u64> {
    if org_ids.is_empty() || stage_kinds.is_empty() {
        return Ok(0);
    }
    let res = sqlx::query(&build_delete_stage_asset_waves_sql())
        .bind(operation_id)
        .bind(org_ids)
        .bind(stage_kinds)
        .execute(&mut *conn)
        .await?;
    Ok(res.rows_affected())
}

/// Roll back `targets.status` to `floor_status` for any target in the subtree that
/// progressed past it. Returns rows changed.
pub async fn rollback_target_status(
    conn: &mut PgConnection,
    org_ids: &[Uuid],
    floor_status: &str,
) -> Result<u64> {
    if org_ids.is_empty() {
        return Ok(0);
    }
    let res = sqlx::query(&build_rollback_target_status_sql())
        .bind(org_ids)
        .bind(floor_status)
        .execute(&mut *conn)
        .await?;
    Ok(res.rows_affected())
}

/// Delete only outcomes belonging to techniques declared by the affected stage
/// specs. Both scopes are mandatory: an empty org or technique set is a no-op.
pub async fn delete_technique_outcomes(
    conn: &mut PgConnection,
    org_ids: &[Uuid],
    run_ids: &[String],
    techniques: &[String],
) -> Result<u64> {
    if org_ids.is_empty() || run_ids.is_empty() || techniques.is_empty() {
        return Ok(0);
    }
    let res = sqlx::query(&build_delete_technique_outcomes_sql())
        .bind(org_ids)
        .bind(run_ids)
        .bind(techniques)
        .execute(&mut *conn)
        .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_scoped_delete_confines_to_org_subtree() {
        let sql = build_delete_by_target_org_sql("api_endpoints");
        assert!(sql.starts_with("DELETE FROM api_endpoints"));
        assert!(
            sql.contains("target_id IN (SELECT id FROM targets WHERE organization_id = ANY($1))")
        );
    }

    #[test]
    fn org_scoped_delete_uses_org_array() {
        let sql = build_delete_by_org_run_sql("source_query_log");
        assert!(sql.contains("organization_id = ANY($1)"));
        assert!(sql.contains("run_id = ANY($2)"));
    }

    #[test]
    fn screenshot_delete_is_exact_operation_scoped() {
        let sql = build_delete_screenshots_by_operation_sql();
        assert_eq!(sql, "DELETE FROM screenshots WHERE task_id = $1");
        assert!(!sql.contains("project_path"));
    }

    #[test]
    fn mutable_eas_surface_delete_is_exact_frozen_org_scoped() {
        for table in [
            "web_origin_observations",
            "web_origins",
            "network_endpoints",
        ] {
            assert_eq!(
                build_delete_mutable_org_fact_sql(table),
                format!("DELETE FROM {table} WHERE organization_id = ANY($1)")
            );
        }
    }

    #[test]
    fn crawl_delete_uses_origin_target_owner_in_frozen_org_scope() {
        let sql = build_delete_crawl_observations_by_target_org_sql();
        assert!(sql.starts_with("DELETE FROM crawl_observations"));
        assert!(sql.contains("origin_target_id IN (SELECT id FROM targets"));
        assert!(sql.contains("organization_id = ANY($1)"));
        assert!(!sql.contains("WHERE target_id"));
    }

    #[test]
    fn eas_reset_nulls_probe_and_freshness_columns() {
        let sql = build_reset_targets_eas_sql();
        for col in [
            "real_ip = ''",
            "ports = '[]'::jsonb",
            "ip_whois = NULL",
            "ports_scanned_at = NULL",
            "liveness_checked_at = NULL",
            "ip_whois_collected_at = NULL",
        ] {
            assert!(sql.contains(col), "missing `{col}` in EAS reset SQL");
        }
        assert!(sql.contains("organization_id = ANY($1)"));
    }

    #[test]
    fn org_intel_freshness_reset_nulls_four_dims() {
        let sql = build_reset_org_intel_freshness_sql();
        for col in [
            "asns_collected_at = NULL",
            "certificates_collected_at = NULL",
            "whois_collected_at = NULL",
            "osint_collected_at = NULL",
        ] {
            assert!(sql.contains(col), "missing `{col}` in org intel reset SQL");
        }
        assert!(sql.contains("id = ANY($1)"));
    }

    #[test]
    fn status_rollback_only_downgrades_above_floor() {
        let sql = build_rollback_target_status_sql();
        assert!(sql.contains("status = $2::target_status"));
        assert!(sql.contains("organization_id = ANY($1)"));
        assert!(sql.contains("status > $2::target_status"));
    }

    #[test]
    fn stage_asset_wave_delete_keys_operation_org_and_stage() {
        let sql = build_delete_stage_asset_waves_sql();
        assert!(sql.contains("operation_id = $1"));
        assert!(sql.contains("organization_id = ANY($2)"));
        assert!(sql.contains("stage_kind = ANY($3)"));
    }

    #[test]
    fn completion_delete_keys_org_and_stage() {
        let sql = build_delete_org_stage_completions_sql();
        assert!(sql.contains("organization_id = ANY($1)"));
        assert!(sql.contains("stage_kind = ANY($2)"));
    }

    #[test]
    fn technique_outcome_delete_keys_org_and_technique() {
        let sql = build_delete_technique_outcomes_sql();
        assert!(sql.starts_with("DELETE FROM technique_outcomes"));
        assert!(sql.contains("organization_id = ANY($1)"));
        assert!(sql.contains("run_id = ANY($2)"));
        assert!(sql.contains("technique = ANY($3)"));
    }

    #[test]
    fn company_vuln_reset_has_no_unowned_fact_delete_sql() {
        let implementation = include_str!("stage_purge.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production implementation section");
        assert!(!implementation.contains("DELETE FROM findings"));
        assert!(!implementation.contains("DELETE FROM vuln_scan_history"));
        assert!(!implementation.contains("DELETE FROM sensitive_scan"));
    }

    #[test]
    fn purge_counts_total_sums_every_field() {
        let counts = StagePurgeCounts {
            target_assets: 1,
            dns_records: 2,
            passive_scans: 3,
            source_query_log: 4,
            org_intel_freshness: 5,
            fingerprints: 6,
            expansion_queue: 7,
            eas_target_columns: 8,
            network_endpoints: 9,
            web_origins: 10,
            web_origin_observations: 11,
            screenshots: 12,
            api_endpoints: 13,
            js_analysis: 14,
            directory_entries: 15,
            crawl_observations: 16,
            endpoint_tests: 17,
            findings: 18,
            vuln_scan_history: 19,
            sensitive_scan: 20,
            technique_outcomes: 21,
            org_stage_completions: 22,
            stage_asset_waves: 23,
            target_status_rolled_back: 24,
        };
        assert_eq!(counts.total(), (1..=24u64).sum::<u64>());
    }
}
