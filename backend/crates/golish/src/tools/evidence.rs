//! `evidence_read` Tauri command (Doc 2 §3 · Phase 1b Task 1b.2).
//!
//! Reads a sanitized evidence summary out of `audit_log` + `evidence_classifications`
//! and returns it to the LLM via `EvidenceSummary`. Raw output is NEVER inlined into
//! the LLM context here; the only path that returns raw text is `SummaryLevel::Raw`
//! and even then it is passed through [`EvidenceSanitizer`] (Doc 2 §4 pipeline).
//!
//! IDOR / scope_version guards:
//!   - Phase 1 MVP single-user desktop: IDOR check = "this evidence_audit_id exists,
//!     is `audit_role='evidence'`, and is not abandoned"
//!   - scope_version snapshot read from `evidence_classifications.scope_version`
//!     (no live `ScopeService` lookup; per Doc 1 §5.4 cursor.last_scope_version
//!     wins)
//!
//! Freshness compares `audit_log.created_at` against `evidence_kinds.json` (Doc 1
//! §6.1) with the 7-day fallback.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::error::GolishError;
use crate::state::DbState;
use golish_pentest::evidence_kinds::EvidenceKindRegistry;
use golish_pentest::evidence_ledger::EvidenceScopeLabel;
use golish_pentest::evidence_sanitizer::EvidenceSanitizer;

/// Request body for `evidence_read` (Doc 2 §3.1).
#[derive(Debug, Clone, Deserialize)]
pub struct ReadEvidenceRequest {
    pub evidence_audit_id: i64,
    #[serde(default)]
    pub summary_level: SummaryLevel,
}

/// 3 摘要档 (Doc 2 §3.1):
///   - `headline`   仅 subject + status + 一句话
///   - `structured` 解析后的字段 (默认 stage 用)
///   - `raw`        完整 sanitize 后 raw (仅 admin/debug)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryLevel {
    Headline,
    #[default]
    Structured,
    Raw,
}

/// 三态新鲜度 (Doc 2 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFreshness {
    Fresh,
    Stale,
    Expired,
}

impl EvidenceFreshness {
    /// `Fresh` = age < max_age; `Stale` = max_age ≤ age < 2 × max_age;
    /// `Expired` = age ≥ 2 × max_age.
    pub fn from_age(age: Duration, max_age: Duration) -> Self {
        if age < max_age {
            Self::Fresh
        } else if age < max_age * 2 {
            Self::Stale
        } else {
            Self::Expired
        }
    }
}

/// 返给 LLM 的 evidence 摘要 (Doc 2 §3.1).
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceSummary {
    pub evidence_audit_id: i64,
    pub kind: String,
    pub subject: String,
    pub as_of_timestamp: DateTime<Utc>,
    pub freshness: EvidenceFreshness,
    pub scope_label: EvidenceScopeLabel,
    /// `Some(...)` 当 kind 有 structural parser 时 (Doc 2 §4.1 parse_structured).
    pub structured: Option<serde_json::Value>,
    /// 一行人话摘要 (始终走 EvidenceSanitizer 全 pipeline).
    pub headline: String,
    /// 仅 `SummaryLevel::Raw` 时返回; 仍经 sanitize_for_llm pipeline.
    pub raw_truncated: Option<String>,
}

/// 取 audit_log 行 + 当前 classification 的合并 row (single SQL).
#[derive(Debug, FromRow)]
struct EvidenceRow {
    id: i64,
    /// audit_role 仅供 SQL WHERE 过滤; 已在查询里限定 = 'evidence'.
    /// 保留字段避免 sqlx FromRow 解析顺序漂移; 显式 allow dead_code.
    #[allow(dead_code)]
    audit_role: String,
    status: String,
    action: String,
    category: String,
    details: String,
    detail: serde_json::Value,
    entity_id: Option<String>,
    tool_name: Option<String>,
    created_at: DateTime<Utc>,
    classification: Option<String>,
    classification_scope_version: Option<i64>,
}

#[tauri::command]
pub async fn evidence_read(
    state: tauri::State<'_, DbState>,
    request: ReadEvidenceRequest,
) -> Result<EvidenceSummary, GolishError> {
    tracing::info!(
        target: "harness::evidence_read",
        evidence_audit_id = request.evidence_audit_id,
        summary_level = ?request.summary_level,
        "evidence_read command entered"
    );
    let pool = state.pool_ready().await?;

    // ── 1 + 2 + 3: 单 SQL 同时做 IDOR check + classification 拼接 ──────────────
    // - 验 audit_log row 存在 + audit_role='evidence'
    // - 排 abandoned (Doc 1 §5.3 不被 evidence_classifications 引用)
    // - LEFT JOIN evidence_classifications WHERE valid_to IS NULL
    let row: Option<EvidenceRow> = sqlx::query_as::<_, EvidenceRow>(
        r#"SELECT
               al.id,
               al.audit_role,
               al.status,
               al.action,
               al.category,
               al.details,
               al.detail,
               al.entity_id,
               al.tool_name,
               al.created_at,
               ec.classification    AS classification,
               ec.scope_version     AS classification_scope_version
           FROM audit_log al
           LEFT JOIN evidence_classifications ec
             ON ec.evidence_audit_id = al.id AND ec.valid_to IS NULL
           WHERE al.id = $1
             AND al.audit_role = 'evidence'
             AND al.status <> 'abandoned'
           LIMIT 1"#,
    )
    .bind(request.evidence_audit_id)
    .fetch_optional(pool)
    .await?;

    let row = row.ok_or_else(|| {
        tracing::warn!(
            target: "harness::evidence_read",
            evidence_audit_id = request.evidence_audit_id,
            "evidence_read NotFound: row missing, abandoned, or not audit_role='evidence'"
        );
        GolishError::NotFound(format!(
            "evidence_audit_id={} not found or not in evidence role",
            request.evidence_audit_id
        ))
    })?;

    // ── 4 + 5: scope_label parsing (Doc 1 §4.1) ─────────────────────────────
    let scope_label = parse_scope_label(row.classification.as_deref());

    // ── 6: freshness via evidence_kinds.json (Doc 1 §6.1) ───────────────────
    let kind = derive_kind(&row);
    let registry = EvidenceKindRegistry::instance();
    let max_age = registry.max_age_with_default(&kind);
    let max_age_chrono = Duration::from_std(max_age).unwrap_or_else(|_| Duration::days(7));
    let age = Utc::now() - row.created_at;
    let freshness = EvidenceFreshness::from_age(age, max_age_chrono);

    // ── subject / headline ──────────────────────────────────────────────────
    let subject = derive_subject(&row);
    let headline_raw = build_headline(&row, &kind, &subject);
    let headline = EvidenceSanitizer::sanitize_for_llm(&headline_raw, row.id.into());

    // ── structured (per-kind parser; Doc 2 §4.1 parse_structured) ───────────
    let raw_payload = extract_raw_payload(&row);
    let structured = match request.summary_level {
        SummaryLevel::Headline => None,
        SummaryLevel::Structured | SummaryLevel::Raw => {
            EvidenceSanitizer::parse_structured(&raw_payload, &kind)
        }
    };

    // ── raw_truncated (仅 SummaryLevel::Raw; 仍走 sanitize pipeline) ────────
    let raw_truncated = match request.summary_level {
        SummaryLevel::Raw => Some(EvidenceSanitizer::sanitize_for_llm(
            &raw_payload,
            row.id.into(),
        )),
        _ => None,
    };

    let _ = row.classification_scope_version; // 当前未直接用; 留作 Phase 2 scope_version snapshot validation 接入.

    tracing::info!(
        target: "harness::evidence_read",
        evidence_audit_id = row.id,
        kind = %kind,
        subject = %subject,
        scope_label = ?scope_label,
        freshness = ?freshness,
        headline_len = headline.len(),
        structured_present = structured.is_some(),
        raw_present = raw_truncated.is_some(),
        raw_len = raw_truncated.as_ref().map(|s| s.len()).unwrap_or(0),
        "evidence_read returning summary"
    );

    Ok(EvidenceSummary {
        evidence_audit_id: row.id,
        kind,
        subject,
        as_of_timestamp: row.created_at,
        freshness,
        scope_label,
        structured,
        headline,
        raw_truncated,
    })
}

/// 解析 `evidence_classifications.classification` TEXT → enum.
///
/// 未分类的 evidence (LEFT JOIN 返 NULL) 默认按 `InScope` 处理. 这是保守 fallback,
/// 实际生产环境 ScopeService 必须确保 evidence 创建时同步写 classification 行.
fn parse_scope_label(s: Option<&str>) -> EvidenceScopeLabel {
    match s.unwrap_or("in_scope") {
        "in_scope" => EvidenceScopeLabel::InScope,
        "out_of_scope" => EvidenceScopeLabel::OutOfScope,
        "derived_from_out_of_scope" => EvidenceScopeLabel::DerivedFromOutOfScope,
        // 未知值保守上推 OOS (deny by default)
        _ => EvidenceScopeLabel::OutOfScope,
    }
}

/// kind = detail.kind (preferred, by EvidenceLedger.append 写入) → tool_name → category.
fn derive_kind(row: &EvidenceRow) -> String {
    if let Some(k) = row.detail.get("kind").and_then(|v| v.as_str()) {
        return k.to_string();
    }
    if let Some(tool) = &row.tool_name {
        if !tool.is_empty() {
            return tool.clone();
        }
    }
    row.category.clone()
}

/// subject = detail.subject (preferred) → entity_id → action.
fn derive_subject(row: &EvidenceRow) -> String {
    if let Some(s) = row.detail.get("subject").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(eid) = &row.entity_id {
        if !eid.is_empty() {
            return eid.clone();
        }
    }
    row.action.clone()
}

/// 取 raw payload: detail.raw_output (preferred) → details (audit_log TEXT 列).
///
/// 始终是字符串 — 调用方负责 sanitize.
fn extract_raw_payload(row: &EvidenceRow) -> String {
    if let Some(raw) = row.detail.get("raw_output").and_then(|v| v.as_str()) {
        return raw.to_string();
    }
    row.details.clone()
}

/// Build a short headline before sanitize. 形如:
/// `[kind] subject (status=...)`
fn build_headline(row: &EvidenceRow, kind: &str, subject: &str) -> String {
    format!("[{}] {} (status={})", kind, subject, row.status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mock_row(detail: serde_json::Value, audit_role: &str) -> EvidenceRow {
        EvidenceRow {
            id: 100,
            audit_role: audit_role.to_string(),
            status: "completed".to_string(),
            action: "dns_resolve_completed".to_string(),
            category: "scan".to_string(),
            details: "example.com -> 93.184.216.34".to_string(),
            detail,
            entity_id: Some("example.com".to_string()),
            tool_name: Some("dns_a".to_string()),
            created_at: Utc::now() - Duration::hours(6),
            classification: Some("in_scope".to_string()),
            classification_scope_version: Some(3),
        }
    }

    #[test]
    fn freshness_fresh_when_age_below_max() {
        let f = EvidenceFreshness::from_age(Duration::hours(1), Duration::hours(24));
        assert_eq!(f, EvidenceFreshness::Fresh);
    }

    #[test]
    fn freshness_stale_when_age_between_max_and_2x() {
        let f = EvidenceFreshness::from_age(Duration::hours(30), Duration::hours(24));
        assert_eq!(f, EvidenceFreshness::Stale);
    }

    #[test]
    fn freshness_expired_when_age_exceeds_2x() {
        let f = EvidenceFreshness::from_age(Duration::hours(50), Duration::hours(24));
        assert_eq!(f, EvidenceFreshness::Expired);
    }

    #[test]
    fn parse_scope_label_three_known_variants() {
        assert_eq!(
            parse_scope_label(Some("in_scope")),
            EvidenceScopeLabel::InScope
        );
        assert_eq!(
            parse_scope_label(Some("out_of_scope")),
            EvidenceScopeLabel::OutOfScope
        );
        assert_eq!(
            parse_scope_label(Some("derived_from_out_of_scope")),
            EvidenceScopeLabel::DerivedFromOutOfScope
        );
    }

    #[test]
    fn parse_scope_label_null_defaults_in_scope() {
        assert_eq!(parse_scope_label(None), EvidenceScopeLabel::InScope);
    }

    #[test]
    fn parse_scope_label_unknown_deny_by_default() {
        // 未识别 classification 值 → 保守 deny (OOS)
        assert_eq!(
            parse_scope_label(Some("garbage_value")),
            EvidenceScopeLabel::OutOfScope
        );
    }

    #[test]
    fn derive_kind_prefers_detail_kind() {
        let row = mock_row(json!({"kind": "http_probe", "subject": "x"}), "evidence");
        assert_eq!(derive_kind(&row), "http_probe");
    }

    #[test]
    fn derive_kind_falls_back_to_tool_name() {
        let row = mock_row(json!({}), "evidence");
        // detail 无 kind, fallback tool_name='dns_a'
        assert_eq!(derive_kind(&row), "dns_a");
    }

    #[test]
    fn derive_kind_final_fallback_category() {
        let mut row = mock_row(json!({}), "evidence");
        row.tool_name = None;
        assert_eq!(derive_kind(&row), "scan");
    }

    #[test]
    fn derive_subject_prefers_detail_subject() {
        let row = mock_row(json!({"subject": "api.example.com"}), "evidence");
        assert_eq!(derive_subject(&row), "api.example.com");
    }

    #[test]
    fn derive_subject_falls_back_to_entity_id() {
        let row = mock_row(json!({}), "evidence");
        assert_eq!(derive_subject(&row), "example.com");
    }

    #[test]
    fn extract_raw_payload_prefers_detail_raw_output() {
        let row = mock_row(
            json!({"raw_output": "HTTP/1.1 200 OK\nServer: nginx"}),
            "evidence",
        );
        let payload = extract_raw_payload(&row);
        assert!(payload.contains("HTTP/1.1"));
    }

    #[test]
    fn extract_raw_payload_falls_back_to_details() {
        let row = mock_row(json!({}), "evidence");
        let payload = extract_raw_payload(&row);
        assert!(payload.contains("example.com"));
    }

    #[test]
    fn build_headline_includes_kind_subject_status() {
        let row = mock_row(json!({}), "evidence");
        let head = build_headline(&row, "dns_a", "example.com");
        assert!(head.contains("dns_a"));
        assert!(head.contains("example.com"));
        assert!(head.contains("completed"));
    }

    #[test]
    fn summary_level_default_is_structured() {
        assert_eq!(SummaryLevel::default(), SummaryLevel::Structured);
    }

    #[test]
    fn summary_level_serde_snake_case() {
        let s = serde_json::to_string(&SummaryLevel::Headline).unwrap();
        assert_eq!(s, "\"headline\"");
        let s = serde_json::to_string(&SummaryLevel::Structured).unwrap();
        assert_eq!(s, "\"structured\"");
        let s = serde_json::to_string(&SummaryLevel::Raw).unwrap();
        assert_eq!(s, "\"raw\"");
    }

    #[test]
    fn read_request_deserialize_with_default_summary_level() {
        let json = r#"{"evidence_audit_id": 42}"#;
        let req: ReadEvidenceRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.evidence_audit_id, 42);
        assert_eq!(req.summary_level, SummaryLevel::Structured);
    }
}
