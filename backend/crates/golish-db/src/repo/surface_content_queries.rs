//! Read-only legacy web-content aggregation for backend surface hierarchy.
//!
//! Phase 2.5A intentionally reads legacy content tables and maps rows to
//! already-existing backend WebOrigin keys. It never creates identity rows and
//! never mutates legacy rows.

use std::collections::{BTreeMap, BTreeSet};

use crate::repo::surface_identity::{normalize_network_endpoint, normalize_web_origin};
use crate::Result;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceContentQuery {
    pub organization_id: Option<Uuid>,
    pub project_path: String,
    pub root_target_id: Uuid,
    pub root_ip: Option<String>,
    pub origin_keys: Vec<String>,
    pub include_related: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceContentCounts {
    pub url_count: u64,
    pub api_count: u64,
    pub js_count: u64,
    pub param_count: u64,
    pub directory_entry_count: u64,
    pub passive_log_count: u64,
    pub evidence_count: u64,
}

/// Cap on how many lightweight refs we surface per WebOrigin (and for the
/// unassigned bucket). Counts stay exact; only the enumerated ref list is
/// bounded so a busy origin cannot bloat the hierarchy payload.
const MAX_REFS_PER_BUCKET: usize = 200;

/// A lightweight pointer back to a single legacy content row (api endpoint / JS
/// result / directory entry / passive log). This is intentionally NOT a full
/// row: it carries only enough metadata for the UI to render a compact list and
/// deep-link back to the legacy record. Counts remain the source of truth for
/// totals; refs are a bounded, best-effort enumeration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceContentRef {
    pub kind: String,
    pub id: String,
    pub url: String,
    pub method: Option<String>,
    pub status_code: Option<i32>,
    pub capture_path: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceUnassignedContentCounts {
    pub url_count: u64,
    pub api_count: u64,
    pub js_count: u64,
    pub param_count: u64,
    pub directory_entry_count: u64,
    pub passive_log_count: u64,
    pub evidence_count: u64,
    pub relative_url_count: u64,
    pub malformed_url_count: u64,
    pub unsupported_scheme_count: u64,
    pub missing_origin_count: u64,
    pub unmatched_origin_count: u64,
    pub unmatched_origin_item_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceContentSummary {
    pub url_count: u64,
    pub api_count: u64,
    pub js_count: u64,
    pub param_count: u64,
    pub directory_entry_count: u64,
    pub passive_log_count: u64,
    pub evidence_count: u64,
    pub content_unassigned_count: u64,
    pub content_unmatched_origin_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceContentAggregation {
    pub by_origin: BTreeMap<String, SurfaceContentCounts>,
    /// Bounded lightweight refs per matched origin key. Keys mirror `by_origin`.
    pub refs_by_origin: BTreeMap<String, Vec<SurfaceContentRef>>,
    pub unassigned: SurfaceUnassignedContentCounts,
    /// Bounded lightweight refs for content that could not be attributed to a
    /// backend WebOrigin (relative/malformed/unsupported/missing + unmatched).
    pub unassigned_refs: Vec<SurfaceContentRef>,
    pub summary: SurfaceContentSummary,
    pub candidate_target_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, FromRow)]
struct CandidateTargetRow {
    id: Uuid,
    target_type: String,
    value: String,
    real_ip: String,
}

#[derive(Debug, Clone, FromRow)]
struct LegacyContentRow {
    kind: String,
    id: Uuid,
    url: String,
    params: serde_json::Value,
    method: Option<String>,
    status_code: Option<i32>,
    capture_path: Option<String>,
    source_label: Option<String>,
}

impl LegacyContentRow {
    fn to_ref(&self) -> SurfaceContentRef {
        SurfaceContentRef {
            kind: self.kind.clone(),
            id: self.id.to_string(),
            url: self.url.clone(),
            method: self
                .method
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            status_code: self.status_code,
            capture_path: self
                .capture_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            source: self
                .source_label
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyContentKind {
    Api,
    Js,
    Directory,
    Passive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UrlUnassignedReason {
    Relative,
    Malformed,
    UnsupportedScheme,
    MissingOrigin,
}

enum UrlOriginAssignment {
    Origin(String),
    Unassigned(UrlUnassignedReason),
}

#[derive(Default)]
struct SurfaceContentAccumulator {
    urls: BTreeSet<String>,
    api_ids: BTreeSet<Uuid>,
    js_ids: BTreeSet<Uuid>,
    directory_entry_ids: BTreeSet<Uuid>,
    passive_log_ids: BTreeSet<Uuid>,
    param_count: u64,
    refs: Vec<SurfaceContentRef>,
}

pub const LIST_SURFACE_CONTENT_CANDIDATE_TARGETS_SQL: &str = r#"
SELECT
    t.id,
    t.target_type::text AS target_type,
    t.value,
    t.real_ip
FROM targets t
WHERE (
    ($1::uuid IS NOT NULL AND t.organization_id = $1)
    OR ($1::uuid IS NULL AND t.organization_id IS NULL AND t.project_path = $2)
)
  AND (
      t.id = $3
      OR (
          $4::boolean
          AND $5::text IS NOT NULL
          AND (
              (t.real_ip = $5 AND t.target_type::text IN ('domain', 'url', 'wildcard'))
              OR t.target_type::text = 'url'
          )
      )
  )
ORDER BY t.created_at ASC, t.id ASC
"#;

pub const LIST_SURFACE_LEGACY_CONTENT_ROWS_SQL: &str = r#"
SELECT
    'api'::text AS kind,
    ae.id,
    ae.url,
    ae.params,
    ae.method AS method,
    ae.status_code AS status_code,
    ae.capture_path AS capture_path,
    ae.source AS source_label
FROM api_endpoints ae
JOIN targets t ON t.id = ae.target_id
WHERE ae.target_id = ANY($3::uuid[])
  AND (
      ($1::uuid IS NOT NULL AND t.organization_id = $1)
      OR ($1::uuid IS NULL AND t.organization_id IS NULL AND t.project_path = $2)
  )
UNION ALL
SELECT
    'js'::text AS kind,
    ja.id,
    ja.url,
    'null'::jsonb AS params,
    NULL::text AS method,
    NULL::integer AS status_code,
    ja.file_path AS capture_path,
    ja.source_tool AS source_label
FROM js_analysis_results ja
JOIN targets t ON t.id = ja.target_id
WHERE ja.target_id = ANY($3::uuid[])
  AND (
      ($1::uuid IS NOT NULL AND t.organization_id = $1)
      OR ($1::uuid IS NULL AND t.organization_id IS NULL AND t.project_path = $2)
  )
UNION ALL
SELECT
    'directory'::text AS kind,
    de.id,
    de.url,
    'null'::jsonb AS params,
    NULL::text AS method,
    de.status_code AS status_code,
    NULL::text AS capture_path,
    de.tool AS source_label
FROM directory_entries de
JOIN targets t ON t.id = de.target_id
WHERE de.target_id = ANY($3::uuid[])
  AND (
      ($1::uuid IS NOT NULL AND t.organization_id = $1)
      OR ($1::uuid IS NULL AND t.organization_id IS NULL AND t.project_path = $2)
  )
UNION ALL
SELECT
    'passive'::text AS kind,
    ps.id,
    ps.url,
    'null'::jsonb AS params,
    NULL::text AS method,
    NULL::integer AS status_code,
    NULL::text AS capture_path,
    ps.tool_used AS source_label
FROM passive_scan_logs ps
JOIN targets t ON t.id = ps.target_id
WHERE ps.target_id = ANY($3::uuid[])
  AND (
      ($1::uuid IS NOT NULL AND t.organization_id = $1)
      OR ($1::uuid IS NULL AND t.organization_id IS NULL AND t.project_path = $2)
  )
ORDER BY kind ASC, id ASC
"#;

pub async fn aggregate_surface_content(
    pool: &PgPool,
    query: SurfaceContentQuery,
) -> Result<SurfaceContentAggregation> {
    let project_path = scoped_project_path(query.organization_id, &query.project_path)?;
    let root_ip = query
        .root_ip
        .as_deref()
        .map(normalize_required_ip)
        .transpose()?;
    let candidate_target_ids = list_candidate_target_ids(
        pool,
        query.organization_id,
        project_path.as_deref(),
        query.root_target_id,
        root_ip.as_deref(),
        query.include_related,
    )
    .await?;

    if candidate_target_ids.is_empty() {
        return Ok(SurfaceContentAggregation::default());
    }

    let rows = sqlx::query_as::<_, LegacyContentRow>(LIST_SURFACE_LEGACY_CONTENT_ROWS_SQL)
        .bind(query.organization_id)
        .bind(project_path.as_deref())
        .bind(&candidate_target_ids)
        .fetch_all(pool)
        .await?;

    Ok(aggregate_legacy_content_rows(
        rows,
        query.origin_keys,
        candidate_target_ids,
    ))
}

async fn list_candidate_target_ids(
    pool: &PgPool,
    organization_id: Option<Uuid>,
    project_path: Option<&str>,
    root_target_id: Uuid,
    root_ip: Option<&str>,
    include_related: bool,
) -> Result<Vec<Uuid>> {
    let rows = sqlx::query_as::<_, CandidateTargetRow>(LIST_SURFACE_CONTENT_CANDIDATE_TARGETS_SQL)
        .bind(organization_id)
        .bind(project_path)
        .bind(root_target_id)
        .bind(include_related)
        .bind(root_ip)
        .fetch_all(pool)
        .await?;

    Ok(candidate_target_ids_from_rows(
        rows,
        root_target_id,
        root_ip,
        include_related,
    ))
}

fn candidate_target_ids_from_rows(
    rows: Vec<CandidateTargetRow>,
    root_target_id: Uuid,
    root_ip: Option<&str>,
    include_related: bool,
) -> Vec<Uuid> {
    let mut ids = BTreeSet::new();

    for row in rows {
        if row.id == root_target_id {
            ids.insert(row.id);
            continue;
        }

        if !include_related {
            continue;
        }

        let Some(root_ip) = root_ip else {
            continue;
        };

        let target_type = row.target_type.trim().to_ascii_lowercase();
        let real_ip_matches = matches!(target_type.as_str(), "domain" | "url" | "wildcard")
            && row.real_ip.trim() == root_ip;
        let ip_literal_url_matches =
            target_type == "url" && url_value_host_is_ip(&row.value, root_ip);

        if real_ip_matches || ip_literal_url_matches {
            ids.insert(row.id);
        }
    }

    ids.into_iter().collect()
}

fn aggregate_legacy_content_rows(
    rows: Vec<LegacyContentRow>,
    origin_keys: Vec<String>,
    candidate_target_ids: Vec<Uuid>,
) -> SurfaceContentAggregation {
    let origin_key_set = origin_keys
        .into_iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<BTreeSet<_>>();
    let mut accumulators: BTreeMap<String, SurfaceContentAccumulator> = BTreeMap::new();
    let mut unassigned = SurfaceUnassignedContentCounts::default();
    let mut unassigned_refs: Vec<SurfaceContentRef> = Vec::new();
    let mut unmatched_origin_keys = BTreeSet::new();

    for row in rows {
        let kind = parse_content_kind(&row.kind);
        match classify_content_url(&row.url) {
            UrlOriginAssignment::Origin(origin_key) if origin_key_set.contains(&origin_key) => {
                let accumulator = accumulators.entry(origin_key).or_default();
                accumulator.add_row(kind, &row);
            }
            UrlOriginAssignment::Origin(origin_key) => {
                unmatched_origin_keys.insert(origin_key);
                unassigned.unmatched_origin_item_count += 1;
                unassigned.add_kind(kind, count_params(&row.params));
                push_capped_ref(&mut unassigned_refs, &row);
            }
            UrlOriginAssignment::Unassigned(reason) => {
                unassigned.add_reason(reason);
                unassigned.add_kind(kind, count_params(&row.params));
                push_capped_ref(&mut unassigned_refs, &row);
            }
        }
    }

    unassigned.unmatched_origin_count = unmatched_origin_keys.len() as u64;

    let mut by_origin: BTreeMap<String, SurfaceContentCounts> = BTreeMap::new();
    let mut refs_by_origin: BTreeMap<String, Vec<SurfaceContentRef>> = BTreeMap::new();
    for (origin, accumulator) in accumulators {
        let (counts, refs) = accumulator.into_counts_and_refs();
        by_origin.insert(origin.clone(), counts);
        if !refs.is_empty() {
            refs_by_origin.insert(origin, refs);
        }
    }

    let summary = summarize_content(&by_origin, &unassigned);

    SurfaceContentAggregation {
        by_origin,
        refs_by_origin,
        unassigned,
        unassigned_refs,
        summary,
        candidate_target_ids,
    }
}

fn push_capped_ref(refs: &mut Vec<SurfaceContentRef>, row: &LegacyContentRow) {
    if refs.len() < MAX_REFS_PER_BUCKET {
        refs.push(row.to_ref());
    }
}

impl SurfaceContentAccumulator {
    fn add_row(&mut self, kind: LegacyContentKind, row: &LegacyContentRow) {
        let id = row.id;
        if matches!(
            kind,
            LegacyContentKind::Api | LegacyContentKind::Js | LegacyContentKind::Directory
        ) {
            self.urls.insert(content_url_key(&row.url));
        }

        let is_new = match kind {
            LegacyContentKind::Api => {
                let inserted = self.api_ids.insert(id);
                if inserted {
                    self.param_count += count_params(&row.params);
                }
                inserted
            }
            LegacyContentKind::Js => self.js_ids.insert(id),
            LegacyContentKind::Directory => self.directory_entry_ids.insert(id),
            LegacyContentKind::Passive => self.passive_log_ids.insert(id),
        };

        if is_new {
            push_capped_ref(&mut self.refs, row);
        }
    }

    fn into_counts_and_refs(self) -> (SurfaceContentCounts, Vec<SurfaceContentRef>) {
        let counts = SurfaceContentCounts {
            url_count: self.urls.len() as u64,
            api_count: self.api_ids.len() as u64,
            js_count: self.js_ids.len() as u64,
            param_count: self.param_count,
            directory_entry_count: self.directory_entry_ids.len() as u64,
            passive_log_count: self.passive_log_ids.len() as u64,
            evidence_count: self.passive_log_ids.len() as u64,
        };
        (counts, self.refs)
    }
}

impl SurfaceUnassignedContentCounts {
    fn add_reason(&mut self, reason: UrlUnassignedReason) {
        match reason {
            UrlUnassignedReason::Relative => self.relative_url_count += 1,
            UrlUnassignedReason::Malformed => self.malformed_url_count += 1,
            UrlUnassignedReason::UnsupportedScheme => self.unsupported_scheme_count += 1,
            UrlUnassignedReason::MissingOrigin => self.missing_origin_count += 1,
        }
    }

    fn add_kind(&mut self, kind: LegacyContentKind, param_count: u64) {
        match kind {
            LegacyContentKind::Api => {
                self.api_count += 1;
                self.param_count += param_count;
                self.url_count += 1;
            }
            LegacyContentKind::Js => {
                self.js_count += 1;
                self.url_count += 1;
            }
            LegacyContentKind::Directory => {
                self.directory_entry_count += 1;
                self.url_count += 1;
            }
            LegacyContentKind::Passive => {
                self.passive_log_count += 1;
                self.evidence_count += 1;
            }
        }
    }
}

fn summarize_content(
    by_origin: &BTreeMap<String, SurfaceContentCounts>,
    unassigned: &SurfaceUnassignedContentCounts,
) -> SurfaceContentSummary {
    let mut summary = SurfaceContentSummary::default();
    for counts in by_origin.values() {
        summary.url_count += counts.url_count;
        summary.api_count += counts.api_count;
        summary.js_count += counts.js_count;
        summary.param_count += counts.param_count;
        summary.directory_entry_count += counts.directory_entry_count;
        summary.passive_log_count += counts.passive_log_count;
        summary.evidence_count += counts.evidence_count;
    }
    summary.content_unassigned_count = unassigned.relative_url_count
        + unassigned.malformed_url_count
        + unassigned.unsupported_scheme_count
        + unassigned.missing_origin_count;
    summary.content_unmatched_origin_count = unassigned.unmatched_origin_count;
    summary
}

fn classify_content_url(raw_url: &str) -> UrlOriginAssignment {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return UrlOriginAssignment::Unassigned(UrlUnassignedReason::MissingOrigin);
    }

    if trimmed.starts_with("//") || trimmed.starts_with('/') || !trimmed.contains("://") {
        return UrlOriginAssignment::Unassigned(UrlUnassignedReason::Relative);
    }

    let Some((scheme, _)) = trimmed.split_once("://") else {
        return UrlOriginAssignment::Unassigned(UrlUnassignedReason::Malformed);
    };

    let scheme = scheme.trim_end_matches(':').to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return UrlOriginAssignment::Unassigned(UrlUnassignedReason::UnsupportedScheme);
    }

    match normalize_web_origin(trimmed) {
        Some(origin) => UrlOriginAssignment::Origin(origin.origin),
        None => UrlOriginAssignment::Unassigned(UrlUnassignedReason::Malformed),
    }
}

fn parse_content_kind(kind: &str) -> LegacyContentKind {
    match kind {
        "api" => LegacyContentKind::Api,
        "js" => LegacyContentKind::Js,
        "directory" => LegacyContentKind::Directory,
        "passive" => LegacyContentKind::Passive,
        _ => LegacyContentKind::Passive,
    }
}

fn count_params(params: &serde_json::Value) -> u64 {
    match params {
        serde_json::Value::Array(values) => values.len() as u64,
        serde_json::Value::Object(values) => values.len() as u64,
        _ => 0,
    }
}

fn content_url_key(url: &str) -> String {
    url.trim().to_string()
}

fn url_value_host_is_ip(value: &str, root_ip: &str) -> bool {
    normalize_web_origin(value)
        .filter(|origin| origin.host_type == "ip" && origin.host == root_ip)
        .is_some()
}

fn scoped_project_path(
    organization_id: Option<Uuid>,
    project_path: &str,
) -> Result<Option<String>> {
    if organization_id.is_some() {
        return Ok(None);
    }

    let project_path = project_path.trim();
    if project_path.is_empty() {
        return Err(anyhow::anyhow!(
            "surface content query requires project_path when organization_id is None"
        )
        .into());
    }

    Ok(Some(project_path.to_string()))
}

fn normalize_required_ip(raw: &str) -> Result<String> {
    normalize_network_endpoint(raw, 1, "tcp")
        .map(|identity| identity.ip)
        .ok_or_else(|| anyhow::anyhow!("invalid surface content root ip query: {raw}").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn content(kind: &str, url: &str) -> LegacyContentRow {
        LegacyContentRow {
            kind: kind.to_string(),
            id: Uuid::new_v4(),
            url: url.to_string(),
            params: serde_json::Value::Null,
            method: None,
            status_code: None,
            capture_path: None,
            source_label: None,
        }
    }

    fn api(url: &str, params: serde_json::Value) -> LegacyContentRow {
        LegacyContentRow {
            kind: "api".to_string(),
            id: Uuid::new_v4(),
            url: url.to_string(),
            params,
            method: Some("GET".to_string()),
            status_code: Some(200),
            capture_path: Some("captures/a/443/api/x.json".to_string()),
            source_label: Some("api_endpoint".to_string()),
        }
    }

    #[test]
    fn api_endpoint_count_and_params_are_grouped_by_origin() {
        let aggregation = aggregate_legacy_content_rows(
            vec![api("https://a.example.com/api/login", json!(["u", "p"]))],
            vec!["https://a.example.com:443".to_string()],
            Vec::new(),
        );

        let counts = aggregation
            .by_origin
            .get("https://a.example.com:443")
            .unwrap();
        assert_eq!(counts.api_count, 1);
        assert_eq!(counts.param_count, 2);
        assert_eq!(counts.url_count, 1);
        assert_eq!(aggregation.summary.api_count, 1);
        assert_eq!(aggregation.summary.param_count, 2);
    }

    #[test]
    fn object_params_count_by_key() {
        let aggregation = aggregate_legacy_content_rows(
            vec![api(
                "https://a.example.com/api/login",
                json!({ "u": 1, "p": 2 }),
            )],
            vec!["https://a.example.com:443".to_string()],
            Vec::new(),
        );

        assert_eq!(
            aggregation
                .by_origin
                .get("https://a.example.com:443")
                .unwrap()
                .param_count,
            2
        );
    }

    #[test]
    fn js_directory_and_passive_counts_are_grouped_by_origin() {
        let aggregation = aggregate_legacy_content_rows(
            vec![
                content("js", "https://a.example.com/static/app.js"),
                content("directory", "https://a.example.com/admin"),
                content("passive", "https://a.example.com/login"),
            ],
            vec!["https://a.example.com:443".to_string()],
            Vec::new(),
        );

        let counts = aggregation
            .by_origin
            .get("https://a.example.com:443")
            .unwrap();
        assert_eq!(counts.js_count, 1);
        assert_eq!(counts.directory_entry_count, 1);
        assert_eq!(counts.passive_log_count, 1);
        assert_eq!(counts.evidence_count, 1);
        assert_eq!(counts.url_count, 2);
    }

    #[test]
    fn relative_urls_are_unassigned() {
        let aggregation = aggregate_legacy_content_rows(
            vec![api("/api/login", json!(["q"]))],
            vec!["https://a.example.com:443".to_string()],
            Vec::new(),
        );

        assert!(aggregation.by_origin.is_empty());
        assert_eq!(aggregation.unassigned.relative_url_count, 1);
        assert_eq!(aggregation.unassigned.api_count, 1);
        assert_eq!(aggregation.summary.content_unassigned_count, 1);
    }

    #[test]
    fn unmatched_origin_is_counted_without_creating_origin() {
        let aggregation = aggregate_legacy_content_rows(
            vec![api("https://c.example.com/api", json!([]))],
            vec!["https://a.example.com:443".to_string()],
            Vec::new(),
        );

        assert!(aggregation.by_origin.is_empty());
        assert_eq!(aggregation.unassigned.unmatched_origin_count, 1);
        assert_eq!(aggregation.unassigned.unmatched_origin_item_count, 1);
        assert_eq!(aggregation.summary.content_unmatched_origin_count, 1);
    }

    #[test]
    fn ip_literal_content_matches_ip_literal_origin() {
        let aggregation = aggregate_legacy_content_rows(
            vec![api("https://1.1.1.1/login", json!([]))],
            vec!["https://1.1.1.1:443".to_string()],
            Vec::new(),
        );

        assert_eq!(
            aggregation
                .by_origin
                .get("https://1.1.1.1:443")
                .unwrap()
                .api_count,
            1
        );
    }

    #[test]
    fn matched_origin_collects_lightweight_refs_not_full_rows() {
        let aggregation = aggregate_legacy_content_rows(
            vec![
                api("https://a.example.com/api/login", json!(["u", "p"])),
                content("js", "https://a.example.com/static/app.js"),
                content("directory", "https://a.example.com/admin"),
            ],
            vec!["https://a.example.com:443".to_string()],
            Vec::new(),
        );

        let refs = aggregation
            .refs_by_origin
            .get("https://a.example.com:443")
            .expect("refs should be attached to matched origin");
        assert_eq!(refs.len(), 3);
        let api_ref = refs.iter().find(|r| r.kind == "api").unwrap();
        assert_eq!(api_ref.url, "https://a.example.com/api/login");
        assert_eq!(api_ref.method.as_deref(), Some("GET"));
        assert_eq!(api_ref.status_code, Some(200));
        assert!(refs.iter().any(|r| r.kind == "js"));
        assert!(refs.iter().any(|r| r.kind == "directory"));
        // Counts stay the source of truth alongside the refs.
        let counts = aggregation
            .by_origin
            .get("https://a.example.com:443")
            .unwrap();
        assert_eq!(counts.api_count, 1);
        assert_eq!(counts.js_count, 1);
        assert_eq!(counts.directory_entry_count, 1);
    }

    #[test]
    fn unassigned_and_unmatched_rows_produce_unassigned_refs_only() {
        let aggregation = aggregate_legacy_content_rows(
            vec![
                api("/relative/login", json!([])),
                api("https://c.example.com/api", json!([])),
            ],
            vec!["https://a.example.com:443".to_string()],
            Vec::new(),
        );

        assert!(aggregation.refs_by_origin.is_empty());
        assert_eq!(aggregation.unassigned_refs.len(), 2);
        assert!(aggregation
            .unassigned_refs
            .iter()
            .any(|r| r.url == "https://c.example.com/api"));
    }

    #[test]
    fn refs_are_capped_per_bucket_while_counts_stay_exact() {
        let rows = (0..(MAX_REFS_PER_BUCKET + 25))
            .map(|i| api(&format!("https://a.example.com/api/{i}"), json!([])))
            .collect::<Vec<_>>();
        let aggregation = aggregate_legacy_content_rows(
            rows,
            vec!["https://a.example.com:443".to_string()],
            Vec::new(),
        );

        let refs = aggregation
            .refs_by_origin
            .get("https://a.example.com:443")
            .unwrap();
        assert_eq!(refs.len(), MAX_REFS_PER_BUCKET);
        let counts = aggregation
            .by_origin
            .get("https://a.example.com:443")
            .unwrap();
        assert_eq!(counts.api_count as usize, MAX_REFS_PER_BUCKET + 25);
    }

    #[test]
    fn explicit_ports_remain_split() {
        let aggregation = aggregate_legacy_content_rows(
            vec![
                api("https://a.example.com/api", json!([])),
                api("https://a.example.com:8443/api", json!([])),
            ],
            vec![
                "https://a.example.com:443".to_string(),
                "https://a.example.com:8443".to_string(),
            ],
            Vec::new(),
        );

        assert_eq!(
            aggregation
                .by_origin
                .get("https://a.example.com:443")
                .unwrap()
                .api_count,
            1
        );
        assert_eq!(
            aggregation
                .by_origin
                .get("https://a.example.com:8443")
                .unwrap()
                .api_count,
            1
        );
    }

    #[test]
    fn candidate_targets_include_root_same_real_ip_and_ip_literal_url_only() {
        let root_id = Uuid::new_v4();
        let same_real_ip_id = Uuid::new_v4();
        let other_real_ip_id = Uuid::new_v4();
        let ip_url_id = Uuid::new_v4();
        let other_url_id = Uuid::new_v4();
        let ids = candidate_target_ids_from_rows(
            vec![
                CandidateTargetRow {
                    id: root_id,
                    target_type: "ip".to_string(),
                    value: "1.1.1.1".to_string(),
                    real_ip: String::new(),
                },
                CandidateTargetRow {
                    id: same_real_ip_id,
                    target_type: "domain".to_string(),
                    value: "a.example.com".to_string(),
                    real_ip: "1.1.1.1".to_string(),
                },
                CandidateTargetRow {
                    id: other_real_ip_id,
                    target_type: "domain".to_string(),
                    value: "b.example.com".to_string(),
                    real_ip: "2.2.2.2".to_string(),
                },
                CandidateTargetRow {
                    id: ip_url_id,
                    target_type: "url".to_string(),
                    value: "https://1.1.1.1/login".to_string(),
                    real_ip: String::new(),
                },
                CandidateTargetRow {
                    id: other_url_id,
                    target_type: "url".to_string(),
                    value: "https://2.2.2.2/login".to_string(),
                    real_ip: String::new(),
                },
            ],
            root_id,
            Some("1.1.1.1"),
            true,
        );

        assert!(ids.contains(&root_id));
        assert!(ids.contains(&same_real_ip_id));
        assert!(ids.contains(&ip_url_id));
        assert!(!ids.contains(&other_real_ip_id));
        assert!(!ids.contains(&other_url_id));
    }

    #[test]
    fn include_related_false_keeps_only_root_target() {
        let root_id = Uuid::new_v4();
        let related_id = Uuid::new_v4();
        let ids = candidate_target_ids_from_rows(
            vec![
                CandidateTargetRow {
                    id: root_id,
                    target_type: "ip".to_string(),
                    value: "1.1.1.1".to_string(),
                    real_ip: String::new(),
                },
                CandidateTargetRow {
                    id: related_id,
                    target_type: "domain".to_string(),
                    value: "a.example.com".to_string(),
                    real_ip: "1.1.1.1".to_string(),
                },
            ],
            root_id,
            Some("1.1.1.1"),
            false,
        );

        assert_eq!(ids, vec![root_id]);
    }

    #[test]
    fn org_and_project_scope_never_full_scan() {
        assert!(scoped_project_path(Some(Uuid::new_v4()), "")
            .unwrap()
            .is_none());
        assert!(scoped_project_path(None, "").is_err());
        assert!(LIST_SURFACE_CONTENT_CANDIDATE_TARGETS_SQL.contains("t.organization_id = $1"));
        assert!(LIST_SURFACE_CONTENT_CANDIDATE_TARGETS_SQL.contains("t.project_path = $2"));
        assert!(LIST_SURFACE_LEGACY_CONTENT_ROWS_SQL.contains("ANY($3::uuid[])"));
        assert!(
            LIST_SURFACE_LEGACY_CONTENT_ROWS_SQL.contains("JOIN targets t ON t.id = ae.target_id")
        );
        assert!(LIST_SURFACE_LEGACY_CONTENT_ROWS_SQL.contains("t.organization_id = $1"));
        assert!(LIST_SURFACE_LEGACY_CONTENT_ROWS_SQL.contains("t.project_path = $2"));
    }

    #[test]
    fn query_sql_is_read_only() {
        let sql = format!(
            "{}\n{}",
            LIST_SURFACE_CONTENT_CANDIDATE_TARGETS_SQL, LIST_SURFACE_LEGACY_CONTENT_ROWS_SQL
        )
        .to_ascii_lowercase();
        assert!(!sql.contains("insert "));
        assert!(!sql.contains("update "));
        assert!(!sql.contains("delete "));
        assert!(!sql.contains("upsert"));
    }
}
