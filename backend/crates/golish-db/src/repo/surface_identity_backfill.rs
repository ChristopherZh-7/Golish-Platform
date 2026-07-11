//! Conservative backfill for first-class surface identity rows.
//!
//! Phase 2.2 scope:
//! - read legacy surface data only;
//! - write only `network_endpoints`, `web_origins`, and
//!   `web_origin_observations` through the Phase 2.1 repos;
//! - never mutate legacy collection tables or command/collector paths.

use crate::models::{NetworkEndpoint, WebOrigin};
use crate::repo::surface_identity::{
    normalize_network_endpoint, normalize_web_origin, NormalizedNetworkEndpoint,
    NormalizedWebOrigin,
};
use crate::repo::{network_endpoints, web_origin_observations, web_origins};
use crate::Result;
use serde_json::{json, Value};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default)]
pub struct SurfaceIdentityBackfillOptions<'a> {
    /// Optional exact project filter. `None` scans all projects and preserves each
    /// row's own `project_path` when writing the new identity tables.
    pub project_path: Option<&'a str>,
    /// Optional organization filter. `None` scans rows regardless of
    /// organization. Rows with `organization_id = NULL` are written with their
    /// effective `project_path` fallback, never collapsed through a caller-level
    /// empty string.
    pub organization_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SurfaceIdentityBackfillSummary {
    pub scanned_targets: u64,
    pub scanned_target_assets: u64,
    pub scanned_api_endpoints: u64,
    pub scanned_js_results: u64,
    pub scanned_directory_entries: u64,
    pub scanned_passive_logs: u64,
    pub created_or_updated_network_endpoints: u64,
    pub created_or_updated_web_origins: u64,
    pub created_or_updated_observations: u64,
    pub skipped_relative_urls: u64,
    pub skipped_malformed_urls: u64,
    pub skipped_missing_endpoint: u64,
    pub skipped_unsupported_scheme: u64,
    pub inferred_observations: u64,
    pub confirmed_observations: u64,
}

/// Backfill first-class surface identity rows from legacy Golish data.
///
/// The function is safe to run repeatedly. Endpoint/origin writes use their
/// identity upserts, and observation writes use a stable backfill synthetic
/// `capture_path` (`backfill:<source_table>:<source_id>`) plus
/// `source = backfill:<source_table>` so the existing observation dedupe index
/// updates the same historical projection instead of appending duplicates.
pub async fn backfill_surface_identity(
    pool: &PgPool,
    options: SurfaceIdentityBackfillOptions<'_>,
) -> Result<SurfaceIdentityBackfillSummary> {
    let mut summary = SurfaceIdentityBackfillSummary::default();

    backfill_targets_ports(pool, options, &mut summary).await?;
    backfill_target_assets(pool, options, &mut summary).await?;
    backfill_url_table(pool, options, UrlSourceTable::ApiEndpoints, &mut summary).await?;
    backfill_url_table(
        pool,
        options,
        UrlSourceTable::JsAnalysisResults,
        &mut summary,
    )
    .await?;
    backfill_url_table(
        pool,
        options,
        UrlSourceTable::DirectoryEntries,
        &mut summary,
    )
    .await?;
    backfill_url_table(pool, options, UrlSourceTable::PassiveScanLogs, &mut summary).await?;

    Ok(summary)
}

const LIST_TARGET_PORT_ROWS_SQL: &str = r#"
SELECT
    target_type::text AS target_type,
    value,
    COALESCE(project_path, '') AS project_path,
    organization_id,
    ports,
    COALESCE(real_ip, '') AS real_ip
FROM targets
WHERE ($1::text IS NULL OR COALESCE(project_path, '') = $1)
  AND ($2::uuid IS NULL OR organization_id = $2)
ORDER BY created_at, id
"#;

const LIST_TARGET_ASSET_ROWS_SQL: &str = r#"
SELECT
    COALESCE(ta.project_path, '') AS source_project_path,
    COALESCE(t.project_path, '') AS target_project_path,
    t.organization_id,
    ta.value,
    ta.port,
    ta.protocol,
    ta.service,
    ta.version,
    ta.metadata,
    COALESCE(t.real_ip, '') AS target_real_ip
FROM target_assets ta
JOIN targets t ON t.id = ta.target_id
WHERE ($1::text IS NULL OR COALESCE(NULLIF(ta.project_path, ''), NULLIF(t.project_path, ''), '') = $1)
  AND ($2::uuid IS NULL OR t.organization_id = $2)
  AND (
      NULLIF(ta.project_path, '') IS NULL
      OR NULLIF(t.project_path, '') IS NULL
      OR ta.project_path = t.project_path
  )
ORDER BY ta.discovered_at, ta.id
"#;

const LIST_API_URL_ROWS_SQL: &str = r#"
SELECT
    ae.id,
    ae.target_id,
    COALESCE(ae.project_path, '') AS source_project_path,
    COALESCE(t.project_path, '') AS target_project_path,
    t.organization_id,
    ae.url,
    ae.status_code,
    COALESCE(ae.source, 'unknown') AS source_label,
    ae.capture_path AS legacy_capture_path,
    t.target_type::text AS target_type,
    t.value AS target_value,
    COALESCE(t.real_ip, '') AS target_real_ip,
    t.ports AS target_ports
FROM api_endpoints ae
JOIN targets t ON t.id = ae.target_id
WHERE ($1::text IS NULL OR COALESCE(NULLIF(ae.project_path, ''), NULLIF(t.project_path, ''), '') = $1)
  AND ($2::uuid IS NULL OR t.organization_id = $2)
  AND (
      NULLIF(ae.project_path, '') IS NULL
      OR NULLIF(t.project_path, '') IS NULL
      OR ae.project_path = t.project_path
  )
ORDER BY ae.discovered_at, ae.id
"#;

const LIST_JS_URL_ROWS_SQL: &str = r#"
SELECT
    ja.id,
    ja.target_id,
    COALESCE(ja.project_path, '') AS source_project_path,
    COALESCE(t.project_path, '') AS target_project_path,
    t.organization_id,
    ja.url,
    NULL::integer AS status_code,
    COALESCE(ja.source_tool, 'unknown') AS source_label,
    ja.file_path AS legacy_capture_path,
    t.target_type::text AS target_type,
    t.value AS target_value,
    COALESCE(t.real_ip, '') AS target_real_ip,
    t.ports AS target_ports
FROM js_analysis_results ja
JOIN targets t ON t.id = ja.target_id
WHERE ($1::text IS NULL OR COALESCE(NULLIF(ja.project_path, ''), NULLIF(t.project_path, ''), '') = $1)
  AND ($2::uuid IS NULL OR t.organization_id = $2)
  AND (
      NULLIF(ja.project_path, '') IS NULL
      OR NULLIF(t.project_path, '') IS NULL
      OR ja.project_path = t.project_path
  )
ORDER BY ja.analyzed_at, ja.id
"#;

const LIST_DIRECTORY_URL_ROWS_SQL: &str = r#"
SELECT
    de.id,
    de.target_id,
    COALESCE(de.project_path, '') AS source_project_path,
    COALESCE(t.project_path, '') AS target_project_path,
    t.organization_id,
    de.url,
    de.status_code,
    COALESCE(de.tool, 'unknown') AS source_label,
    NULL::text AS legacy_capture_path,
    t.target_type::text AS target_type,
    t.value AS target_value,
    COALESCE(t.real_ip, '') AS target_real_ip,
    t.ports AS target_ports
FROM directory_entries de
JOIN targets t ON t.id = de.target_id
WHERE ($1::text IS NULL OR COALESCE(NULLIF(de.project_path, ''), NULLIF(t.project_path, ''), '') = $1)
  AND ($2::uuid IS NULL OR t.organization_id = $2)
  AND (
      NULLIF(de.project_path, '') IS NULL
      OR NULLIF(t.project_path, '') IS NULL
      OR de.project_path = t.project_path
  )
ORDER BY de.created_at, de.id
"#;

const LIST_PASSIVE_URL_ROWS_SQL: &str = r#"
SELECT
    ps.id,
    ps.target_id,
    COALESCE(ps.project_path, '') AS source_project_path,
    COALESCE(t.project_path, '') AS target_project_path,
    t.organization_id,
    ps.url,
    NULL::integer AS status_code,
    COALESCE(ps.tool_used, ps.tester, 'unknown') AS source_label,
    NULL::text AS legacy_capture_path,
    t.target_type::text AS target_type,
    t.value AS target_value,
    COALESCE(t.real_ip, '') AS target_real_ip,
    t.ports AS target_ports
FROM passive_scan_logs ps
JOIN targets t ON t.id = ps.target_id
WHERE ($1::text IS NULL OR COALESCE(NULLIF(ps.project_path, ''), NULLIF(t.project_path, ''), '') = $1)
  AND ($2::uuid IS NULL OR t.organization_id = $2)
  AND (
      NULLIF(ps.project_path, '') IS NULL
      OR NULLIF(t.project_path, '') IS NULL
      OR ps.project_path = t.project_path
  )
ORDER BY ps.tested_at, ps.id
"#;

#[derive(Debug, Clone, FromRow)]
struct LegacyTargetPortRow {
    target_type: String,
    value: String,
    project_path: String,
    organization_id: Option<Uuid>,
    ports: Value,
    real_ip: String,
}

#[derive(Debug, Clone, FromRow)]
struct LegacyTargetAssetRow {
    source_project_path: String,
    target_project_path: String,
    organization_id: Option<Uuid>,
    value: String,
    port: Option<i32>,
    protocol: Option<String>,
    service: Option<String>,
    version: Option<String>,
    metadata: Value,
    /// Resolved IP of the owning target (`targets.real_ip`). Passive service
    /// assets store `value = "<port>/<proto>"` on a domain target, so the only
    /// IP available for `network_endpoints` (which are IP:port keyed) is the
    /// domain's resolved IP.
    target_real_ip: String,
}

#[derive(Debug, Clone, FromRow)]
struct LegacyUrlRow {
    id: Uuid,
    target_id: Uuid,
    source_project_path: String,
    target_project_path: String,
    organization_id: Option<Uuid>,
    url: String,
    status_code: Option<i32>,
    source_label: String,
    legacy_capture_path: Option<String>,
    target_type: String,
    target_value: String,
    target_real_ip: String,
    target_ports: Value,
}

#[derive(Debug, Clone)]
struct EndpointBackfill {
    identity: NormalizedNetworkEndpoint,
    state: Option<String>,
    service_name: Option<String>,
    service_product: Option<String>,
    service_version: Option<String>,
    banner: Option<String>,
    tls_detected: Option<bool>,
    source: String,
    confidence: f32,
    last_confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservationKind {
    Confirmed,
    Inferred,
}

impl ObservationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Inferred => "inferred",
        }
    }

    fn confidence(self) -> f32 {
        match self {
            Self::Confirmed => 0.9,
            Self::Inferred => 0.6,
        }
    }
}

#[derive(Debug, Clone)]
struct EndpointResolution {
    endpoint: EndpointBackfill,
    observation_kind: ObservationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UrlBackfillSkip {
    Relative,
    Malformed,
    UnsupportedScheme,
}

#[derive(Debug, Clone)]
struct UrlBackfillPlan {
    origin: Option<NormalizedWebOrigin>,
    endpoint_resolution: Option<EndpointResolution>,
    skip: Option<UrlBackfillSkip>,
    missing_endpoint: bool,
}

#[derive(Debug, Clone, Copy)]
enum UrlSourceTable {
    ApiEndpoints,
    JsAnalysisResults,
    DirectoryEntries,
    PassiveScanLogs,
}

impl UrlSourceTable {
    fn table_name(self) -> &'static str {
        match self {
            Self::ApiEndpoints => "api_endpoints",
            Self::JsAnalysisResults => "js_analysis_results",
            Self::DirectoryEntries => "directory_entries",
            Self::PassiveScanLogs => "passive_scan_logs",
        }
    }

    fn list_sql(self) -> &'static str {
        match self {
            Self::ApiEndpoints => LIST_API_URL_ROWS_SQL,
            Self::JsAnalysisResults => LIST_JS_URL_ROWS_SQL,
            Self::DirectoryEntries => LIST_DIRECTORY_URL_ROWS_SQL,
            Self::PassiveScanLogs => LIST_PASSIVE_URL_ROWS_SQL,
        }
    }

    fn backfill_source(self) -> String {
        format!("backfill:{}", self.table_name())
    }

    fn increment_scanned(self, summary: &mut SurfaceIdentityBackfillSummary) {
        match self {
            Self::ApiEndpoints => summary.scanned_api_endpoints += 1,
            Self::JsAnalysisResults => summary.scanned_js_results += 1,
            Self::DirectoryEntries => summary.scanned_directory_entries += 1,
            Self::PassiveScanLogs => summary.scanned_passive_logs += 1,
        }
    }
}

async fn backfill_targets_ports(
    pool: &PgPool,
    options: SurfaceIdentityBackfillOptions<'_>,
    summary: &mut SurfaceIdentityBackfillSummary,
) -> Result<()> {
    let rows = sqlx::query_as::<_, LegacyTargetPortRow>(LIST_TARGET_PORT_ROWS_SQL)
        .bind(options.project_path)
        .bind(options.organization_id)
        .fetch_all(pool)
        .await?;

    for row in rows {
        summary.scanned_targets += 1;
        let project_path = row.project_path.clone();
        for endpoint in endpoint_candidates_from_target_ports(&row) {
            upsert_endpoint(pool, row.organization_id, &project_path, &endpoint).await?;
            summary.created_or_updated_network_endpoints += 1;
        }
    }

    Ok(())
}

async fn backfill_target_assets(
    pool: &PgPool,
    options: SurfaceIdentityBackfillOptions<'_>,
    summary: &mut SurfaceIdentityBackfillSummary,
) -> Result<()> {
    let rows = sqlx::query_as::<_, LegacyTargetAssetRow>(LIST_TARGET_ASSET_ROWS_SQL)
        .bind(options.project_path)
        .bind(options.organization_id)
        .fetch_all(pool)
        .await?;

    for row in rows {
        summary.scanned_target_assets += 1;
        let Some(endpoint) = endpoint_candidate_from_target_asset(&row) else {
            continue;
        };
        let project_path =
            effective_project_path(&row.source_project_path, &row.target_project_path);
        upsert_endpoint(pool, row.organization_id, &project_path, &endpoint).await?;
        summary.created_or_updated_network_endpoints += 1;
    }

    Ok(())
}

async fn backfill_url_table(
    pool: &PgPool,
    options: SurfaceIdentityBackfillOptions<'_>,
    table: UrlSourceTable,
    summary: &mut SurfaceIdentityBackfillSummary,
) -> Result<()> {
    let rows = sqlx::query_as::<_, LegacyUrlRow>(table.list_sql())
        .bind(options.project_path)
        .bind(options.organization_id)
        .fetch_all(pool)
        .await?;

    for row in rows {
        table.increment_scanned(summary);
        let project_path =
            effective_project_path(&row.source_project_path, &row.target_project_path);
        let plan = plan_url_backfill(&row, table);

        if let Some(skip) = plan.skip {
            increment_url_skip(summary, skip);
            continue;
        }

        let Some(origin_identity) = plan.origin else {
            summary.skipped_malformed_urls += 1;
            continue;
        };

        let web_origin = web_origins::upsert_by_identity(
            pool,
            row.organization_id,
            Some(project_path.as_str()),
            &origin_identity,
            Some(table.backfill_source().as_str()),
            Some(0.8),
            true,
        )
        .await?;
        summary.created_or_updated_web_origins += 1;

        let Some(endpoint_resolution) = plan.endpoint_resolution else {
            if plan.missing_endpoint {
                summary.skipped_missing_endpoint += 1;
            }
            continue;
        };

        let network_endpoint = upsert_endpoint(
            pool,
            row.organization_id,
            &project_path,
            &endpoint_resolution.endpoint,
        )
        .await?;
        summary.created_or_updated_network_endpoints += 1;

        upsert_backfill_observation(
            pool,
            &row,
            table,
            &project_path,
            &origin_identity,
            &web_origin,
            &network_endpoint,
            endpoint_resolution.observation_kind,
        )
        .await?;
        summary.created_or_updated_observations += 1;
        match endpoint_resolution.observation_kind {
            ObservationKind::Confirmed => summary.confirmed_observations += 1,
            ObservationKind::Inferred => summary.inferred_observations += 1,
        }
    }

    Ok(())
}

async fn upsert_endpoint(
    pool: &PgPool,
    organization_id: Option<Uuid>,
    project_path: &str,
    endpoint: &EndpointBackfill,
) -> Result<NetworkEndpoint> {
    network_endpoints::upsert_by_identity(
        pool,
        organization_id,
        Some(project_path),
        &endpoint.identity,
        endpoint.state.as_deref(),
        endpoint.service_name.as_deref(),
        endpoint.service_product.as_deref(),
        endpoint.service_version.as_deref(),
        endpoint.banner.as_deref(),
        endpoint.tls_detected,
        Some(endpoint.source.as_str()),
        Some(endpoint.confidence),
        endpoint.last_confirmed,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn upsert_backfill_observation(
    pool: &PgPool,
    row: &LegacyUrlRow,
    table: UrlSourceTable,
    project_path: &str,
    origin_identity: &NormalizedWebOrigin,
    web_origin: &WebOrigin,
    network_endpoint: &NetworkEndpoint,
    observation_kind: ObservationKind,
) -> Result<()> {
    let source = table.backfill_source();
    let synthetic_capture_path = synthetic_backfill_capture_path(table, row.id);
    let raw = json!({
        "backfill": true,
        "source_table": table.table_name(),
        "source_id": row.id.to_string(),
        "dedupe_key": synthetic_capture_path,
        "observation_kind": observation_kind.as_str(),
        "legacy_url": row.url,
        "legacy_source_label": row.source_label,
        "legacy_capture_path": row.legacy_capture_path,
    });
    let sni = (origin_identity.host_type == "domain").then_some(origin_identity.host.as_str());
    let input = web_origin_observations::NewWebOriginObservation {
        organization_id: row.organization_id,
        project_path: Some(project_path),
        web_origin_id: web_origin.id,
        network_endpoint_id: Some(network_endpoint.id),
        target_id: Some(row.target_id),
        observed_ip: Some(network_endpoint.ip.as_str()),
        sni,
        host_header: Some(origin_identity.host.as_str()),
        status_code: row.status_code,
        title: None,
        final_url: Some(row.url.as_str()),
        redirect_chain: None,
        body_hash: None,
        favicon_hash: None,
        screenshot_path: None,
        capture_path: Some(synthetic_capture_path.as_str()),
        confidence: Some(observation_kind.confidence()),
        source: Some(source.as_str()),
        raw: Some(&raw),
    };

    web_origin_observations::upsert_observation_dedupe(pool, &input).await?;
    Ok(())
}

fn increment_url_skip(summary: &mut SurfaceIdentityBackfillSummary, skip: UrlBackfillSkip) {
    match skip {
        UrlBackfillSkip::Relative => summary.skipped_relative_urls += 1,
        UrlBackfillSkip::Malformed => summary.skipped_malformed_urls += 1,
        UrlBackfillSkip::UnsupportedScheme => summary.skipped_unsupported_scheme += 1,
    }
}

fn endpoint_candidates_from_target_ports(row: &LegacyTargetPortRow) -> Vec<EndpointBackfill> {
    // An IP target keys endpoints on its own value; a domain/URL target keys them
    // on its resolved `real_ip` (its ports belong to the IP it resolves to).
    let Some(endpoint_ip) = target_port_endpoint_ip(row) else {
        return Vec::new();
    };

    let Some(entries) = row.ports.as_array() else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| endpoint_candidate_from_target_port_entry(row, &endpoint_ip, entry))
        .collect()
}

/// The IP a target's `ports` should be attributed to: the value itself for IP
/// targets, else the resolved `real_ip`. Domain ports without a resolved IP have
/// no IP:port identity and are skipped (they still show via legacy fallback).
fn target_port_endpoint_ip(row: &LegacyTargetPortRow) -> Option<String> {
    if is_ip_target_type(&row.target_type) {
        return Some(row.value.clone());
    }
    let real_ip = row.real_ip.trim();
    if real_ip.is_empty() {
        return None;
    }
    Some(real_ip.to_string())
}

fn endpoint_candidate_from_target_port_entry(
    row: &LegacyTargetPortRow,
    endpoint_ip: &str,
    entry: &Value,
) -> Option<EndpointBackfill> {
    let port = port_from_json(entry)?;
    let transport = transport_from_json(entry);
    let identity = normalize_network_endpoint(endpoint_ip, port, &transport)?;
    let confirmed = is_ip_target_type(&row.target_type);
    let service_name = string_field(entry, &["service", "service_name", "name"]);
    let service_product = string_field(entry, &["product", "service_product"]);
    let service_version = string_field(entry, &["version", "service_version"]);
    let banner = string_field(entry, &["banner"]);
    let state = string_field(entry, &["state"]).or(Some("open".to_string()));
    let tls_detected = bool_field(entry, &["tls_detected", "tls", "ssl"])
        .or_else(|| service_name.as_deref().map(service_name_implies_tls))
        .or(Some(port == 443));

    Some(EndpointBackfill {
        identity,
        state,
        service_name,
        service_product,
        service_version,
        banner,
        tls_detected,
        // Ports on an IP target are confirmed IP:port facts; ports carried by a
        // domain/URL target are attributed to its resolved IP, so they are an
        // inferred (real_ip-derived) endpoint.
        source: if confirmed {
            "backfill:targets.ports".to_string()
        } else {
            "backfill:targets.ports.real_ip".to_string()
        },
        confidence: if confirmed { 0.85 } else { 0.6 },
        last_confirmed: confirmed,
    })
}

fn endpoint_candidate_from_target_asset(row: &LegacyTargetAssetRow) -> Option<EndpointBackfill> {
    let port = row.port.filter(|port| (1..=65535).contains(port))?;
    let (ip, ip_is_explicit) = ip_for_target_asset(row)?;
    let transport = transport_from_protocol(row.protocol.as_deref());
    let identity = normalize_network_endpoint(&ip, port, &transport)?;
    let service_name = row
        .service
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| string_field(&row.metadata, &["service", "service_name", "name"]));
    let service_product = string_field(&row.metadata, &["product", "service_product"]);
    let service_version = row
        .version
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| string_field(&row.metadata, &["version", "service_version"]));
    let banner = string_field(&row.metadata, &["banner"]);
    let tls_detected = bool_field(&row.metadata, &["tls_detected", "tls", "ssl"])
        .or_else(|| service_name.as_deref().map(service_name_implies_tls));

    Some(EndpointBackfill {
        identity,
        state: Some("open".to_string()),
        service_name,
        service_product,
        service_version,
        banner,
        tls_detected,
        // Explicit IP asset value → confirmed; a passive `<port>/<proto>` asset
        // on a domain target resolved through `real_ip` → inferred.
        source: if ip_is_explicit {
            "backfill:target_assets".to_string()
        } else {
            "backfill:target_assets.real_ip".to_string()
        },
        confidence: if ip_is_explicit { 0.8 } else { 0.6 },
        last_confirmed: ip_is_explicit,
    })
}

fn plan_url_backfill(row: &LegacyUrlRow, table: UrlSourceTable) -> UrlBackfillPlan {
    let Some(origin) = normalize_web_origin(&row.url) else {
        return UrlBackfillPlan {
            origin: None,
            endpoint_resolution: None,
            skip: Some(classify_url_skip(&row.url)),
            missing_endpoint: false,
        };
    };

    let endpoint_resolution = endpoint_resolution_for_url(row, table, &origin);
    let missing_endpoint = endpoint_resolution.is_none();
    UrlBackfillPlan {
        origin: Some(origin),
        endpoint_resolution,
        skip: None,
        missing_endpoint,
    }
}

fn endpoint_resolution_for_url(
    row: &LegacyUrlRow,
    table: UrlSourceTable,
    origin: &NormalizedWebOrigin,
) -> Option<EndpointResolution> {
    if origin.host_type == "ip" {
        let identity = normalize_network_endpoint(&origin.host, origin.port, "tcp")?;
        return Some(EndpointResolution {
            endpoint: EndpointBackfill {
                identity,
                state: Some("open".to_string()),
                service_name: Some(origin.scheme.clone()),
                service_product: None,
                service_version: None,
                banner: None,
                tls_detected: Some(origin.scheme == "https"),
                source: table.backfill_source(),
                confidence: 0.9,
                last_confirmed: true,
            },
            observation_kind: ObservationKind::Confirmed,
        });
    }

    if is_ip_target_type(&row.target_type)
        && target_ports_contain(&row.target_ports, origin.port, "tcp")
    {
        let identity = normalize_network_endpoint(&row.target_value, origin.port, "tcp")?;
        return Some(EndpointResolution {
            endpoint: EndpointBackfill {
                identity,
                state: Some("open".to_string()),
                service_name: Some(origin.scheme.clone()),
                service_product: None,
                service_version: None,
                banner: None,
                tls_detected: Some(origin.scheme == "https"),
                source: "backfill:targets.ports".to_string(),
                confidence: 0.85,
                last_confirmed: true,
            },
            observation_kind: ObservationKind::Confirmed,
        });
    }

    if !row.target_real_ip.trim().is_empty() {
        let identity = normalize_network_endpoint(&row.target_real_ip, origin.port, "tcp")?;
        return Some(EndpointResolution {
            endpoint: EndpointBackfill {
                identity,
                state: Some("unknown".to_string()),
                service_name: Some(origin.scheme.clone()),
                service_product: None,
                service_version: None,
                banner: None,
                tls_detected: Some(origin.scheme == "https"),
                source: "backfill:target.real_ip".to_string(),
                confidence: 0.6,
                last_confirmed: false,
            },
            observation_kind: ObservationKind::Inferred,
        });
    }

    None
}

fn classify_url_skip(url: &str) -> UrlBackfillSkip {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return UrlBackfillSkip::Malformed;
    }

    if trimmed.starts_with("//") || trimmed.starts_with('/') || !trimmed.contains("://") {
        if let Some((scheme, _)) = trimmed.split_once(':') {
            if looks_like_scheme(scheme) && scheme != "http" && scheme != "https" {
                return UrlBackfillSkip::UnsupportedScheme;
            }
        }
        return UrlBackfillSkip::Relative;
    }

    let scheme = trimmed
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
        .unwrap_or_default();
    if scheme != "http" && scheme != "https" {
        UrlBackfillSkip::UnsupportedScheme
    } else {
        UrlBackfillSkip::Malformed
    }
}

fn looks_like_scheme(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn synthetic_backfill_capture_path(table: UrlSourceTable, source_id: Uuid) -> String {
    format!("backfill:{}:{source_id}", table.table_name())
}

fn effective_project_path(source_project_path: &str, target_project_path: &str) -> String {
    let source = source_project_path.trim();
    if !source.is_empty() {
        return source.to_string();
    }

    let target = target_project_path.trim();
    if !target.is_empty() {
        return target.to_string();
    }

    String::new()
}

fn is_ip_target_type(target_type: &str) -> bool {
    matches!(
        target_type.trim().to_ascii_lowercase().as_str(),
        "ip" | "ipv4" | "ip_address"
    )
}

fn target_ports_contain(ports: &Value, port: i32, transport: &str) -> bool {
    let Some(entries) = ports.as_array() else {
        return false;
    };

    entries.iter().any(|entry| {
        port_from_json(entry) == Some(port)
            && transport_from_json(entry) == transport_from_protocol(Some(transport))
    })
}

/// Resolve the IP a `target_assets` row should key its endpoint on, plus whether
/// that IP was explicit (asset value / metadata ip) versus inferred from the
/// owning target's `real_ip`. Passive `land_service_assets` rows store
/// `value = "<port>/<proto>"` with no IP, so the resolved `real_ip` is the only
/// way they become an IP:port `network_endpoint`.
fn ip_for_target_asset(row: &LegacyTargetAssetRow) -> Option<(String, bool)> {
    if let Some(ip) = explicit_ip_from_target_asset(row) {
        return Some((ip, true));
    }
    let real_ip = row.target_real_ip.trim();
    if real_ip.is_empty() {
        return None;
    }
    normalize_network_endpoint(real_ip, row.port?, "tcp").map(|identity| (identity.ip, false))
}

fn explicit_ip_from_target_asset(row: &LegacyTargetAssetRow) -> Option<String> {
    normalize_network_endpoint(&row.value, row.port?, "tcp")
        .map(|identity| identity.ip)
        .or_else(|| {
            string_field(&row.metadata, &["ip", "observed_ip", "real_ip"]).and_then(|ip| {
                normalize_network_endpoint(&ip, row.port?, "tcp").map(|identity| identity.ip)
            })
        })
}

fn port_from_json(value: &Value) -> Option<i32> {
    if let Some(port) = value.as_i64() {
        return i32::try_from(port)
            .ok()
            .filter(|port| (1..=65535).contains(port));
    }

    int_field(value, &["port", "number", "port_number"]).filter(|port| (1..=65535).contains(port))
}

fn transport_from_json(value: &Value) -> String {
    let raw = string_field(value, &["transport", "proto", "protocol"]);
    transport_from_protocol(raw.as_deref())
}

fn transport_from_protocol(raw: Option<&str>) -> String {
    match raw.unwrap_or("tcp").trim().to_ascii_lowercase().as_str() {
        "udp" => "udp".to_string(),
        "unknown" => "unknown".to_string(),
        // Legacy rows often store application protocols (`http`, `https`,
        // `ssl`, `tls`) in `protocol`. NetworkEndpoint transport should still be
        // TCP for these web observations.
        _ => "tcp".to_string(),
    }
}

fn service_name_implies_tls(service_name: &str) -> bool {
    matches!(
        service_name.trim().to_ascii_lowercase().as_str(),
        "https" | "ssl" | "tls" | "https-alt"
    )
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|raw| match raw {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    })
}

fn int_field(value: &Value, keys: &[&str]) -> Option<i32> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|raw| match raw {
            Value::Number(n) => n.as_i64().and_then(|n| i32::try_from(n).ok()),
            Value::String(s) => s.trim().parse::<i32>().ok(),
            _ => None,
        })
    })
}

fn bool_field(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|raw| match raw {
            Value::Bool(value) => Some(*value),
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => Some(true),
                "false" | "no" | "0" => Some(false),
                _ => None,
            },
            _ => None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    type EndpointKey = (String, i32, String);
    type OriginKey = (String, String, i32);
    type ObservationKey = (String, String, String);
    type PlanKeys = (
        HashSet<EndpointKey>,
        HashSet<OriginKey>,
        HashSet<ObservationKey>,
    );

    fn target_row(target_type: &str, value: &str, ports: Value) -> LegacyTargetPortRow {
        LegacyTargetPortRow {
            target_type: target_type.to_string(),
            value: value.to_string(),
            project_path: "proj-a".to_string(),
            organization_id: None,
            ports,
            real_ip: String::new(),
        }
    }

    fn target_row_with_real_ip(
        target_type: &str,
        value: &str,
        real_ip: &str,
        ports: Value,
    ) -> LegacyTargetPortRow {
        LegacyTargetPortRow {
            target_type: target_type.to_string(),
            value: value.to_string(),
            project_path: "proj-a".to_string(),
            organization_id: None,
            ports,
            real_ip: real_ip.to_string(),
        }
    }

    fn asset_row(
        value: &str,
        port: Option<i32>,
        protocol: Option<&str>,
        service: Option<&str>,
        target_real_ip: &str,
    ) -> LegacyTargetAssetRow {
        LegacyTargetAssetRow {
            source_project_path: String::new(),
            target_project_path: "proj-a".to_string(),
            organization_id: None,
            value: value.to_string(),
            port,
            protocol: protocol.map(str::to_string),
            service: service.map(str::to_string),
            version: None,
            metadata: json!({ "source": "asset_intel" }),
            target_real_ip: target_real_ip.to_string(),
        }
    }

    fn url_row(
        url: &str,
        target_type: &str,
        target_value: &str,
        target_real_ip: &str,
        ports: Value,
    ) -> LegacyUrlRow {
        LegacyUrlRow {
            id: Uuid::new_v4(),
            target_id: Uuid::new_v4(),
            source_project_path: String::new(),
            target_project_path: "proj-a".to_string(),
            organization_id: None,
            url: url.to_string(),
            status_code: Some(200),
            source_label: "test".to_string(),
            legacy_capture_path: None,
            target_type: target_type.to_string(),
            target_value: target_value.to_string(),
            target_real_ip: target_real_ip.to_string(),
            target_ports: ports,
        }
    }

    fn collect_plan_keys(
        target_rows: &[LegacyTargetPortRow],
        url_rows: &[LegacyUrlRow],
    ) -> PlanKeys {
        let mut endpoints = HashSet::new();
        let mut origins = HashSet::new();
        let mut observations = HashSet::new();

        for row in target_rows {
            for endpoint in endpoint_candidates_from_target_ports(row) {
                endpoints.insert((
                    endpoint.identity.ip,
                    endpoint.identity.port,
                    endpoint.identity.transport,
                ));
            }
        }

        for row in url_rows {
            let table = UrlSourceTable::DirectoryEntries;
            let plan = plan_url_backfill(row, table);
            let Some(origin) = plan.origin else {
                continue;
            };
            origins.insert((origin.scheme.clone(), origin.host.clone(), origin.port));
            if let Some(endpoint_resolution) = plan.endpoint_resolution {
                endpoints.insert((
                    endpoint_resolution.endpoint.identity.ip.clone(),
                    endpoint_resolution.endpoint.identity.port,
                    endpoint_resolution.endpoint.identity.transport.clone(),
                ));
                observations.insert((
                    origin.origin,
                    format!(
                        "{}:{}:{}",
                        endpoint_resolution.endpoint.identity.ip,
                        endpoint_resolution.endpoint.identity.transport,
                        endpoint_resolution.endpoint.identity.port
                    ),
                    synthetic_backfill_capture_path(table, row.id),
                ));
            }
        }

        (endpoints, origins, observations)
    }

    #[test]
    fn domain_target_ports_backfill_endpoint_via_real_ip_as_inferred() {
        // Gap B: EAS may write targets.ports on a domain target; its ports belong
        // to the resolved IP, so backfill keys the endpoint on real_ip (inferred).
        let row = target_row_with_real_ip(
            "domain",
            "a.example.com",
            "1.2.3.4",
            json!([{ "port": 443, "proto": "tcp", "service": "https" }]),
        );

        let endpoints = endpoint_candidates_from_target_ports(&row);

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].identity.ip, "1.2.3.4");
        assert_eq!(endpoints[0].identity.port, 443);
        assert!(!endpoints[0].last_confirmed);
        assert_eq!(endpoints[0].source, "backfill:targets.ports.real_ip");
    }

    #[test]
    fn domain_target_ports_without_real_ip_create_no_endpoint() {
        let row = target_row(
            "domain",
            "a.example.com",
            json!([{ "port": 443, "proto": "tcp" }]),
        );
        assert!(endpoint_candidates_from_target_ports(&row).is_empty());
    }

    #[test]
    fn passive_service_asset_backfills_endpoint_via_real_ip() {
        // Gap A: land_service_assets stores value="443/tcp" on a domain target with
        // no IP; the only IP is the target's real_ip, and it lands as inferred.
        let row = asset_row("443/tcp", Some(443), Some("tcp"), Some("https"), "1.2.3.4");
        let endpoint = endpoint_candidate_from_target_asset(&row).expect("endpoint via real_ip");
        assert_eq!(endpoint.identity.ip, "1.2.3.4");
        assert_eq!(endpoint.identity.port, 443);
        assert_eq!(endpoint.service_name.as_deref(), Some("https"));
        assert!(!endpoint.last_confirmed);
        assert_eq!(endpoint.source, "backfill:target_assets.real_ip");
    }

    #[test]
    fn explicit_ip_asset_stays_confirmed() {
        let row = asset_row("1.2.3.4", Some(443), Some("tcp"), Some("https"), "");
        let endpoint = endpoint_candidate_from_target_asset(&row).expect("endpoint via value ip");
        assert_eq!(endpoint.identity.ip, "1.2.3.4");
        assert!(endpoint.last_confirmed);
        assert_eq!(endpoint.source, "backfill:target_assets");
    }

    #[test]
    fn service_asset_without_ip_or_real_ip_is_skipped() {
        let row = asset_row("443/tcp", Some(443), Some("tcp"), Some("https"), "");
        assert!(endpoint_candidate_from_target_asset(&row).is_none());
    }

    #[test]
    fn targets_ports_create_network_endpoints_for_ip_targets() {
        let row = target_row(
            "ip",
            "1.1.1.1",
            json!([{ "port": 443, "proto": "tcp", "service": "https" }]),
        );

        let endpoints = endpoint_candidates_from_target_ports(&row);

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].identity.ip, "1.1.1.1");
        assert_eq!(endpoints[0].identity.port, 443);
        assert_eq!(endpoints[0].identity.transport, "tcp");
        assert_eq!(endpoints[0].service_name.as_deref(), Some("https"));
    }

    #[test]
    fn api_endpoint_url_creates_default_https_web_origin() {
        let row = url_row(
            "https://a.example.com/login",
            "domain",
            "a.example.com",
            "",
            json!([]),
        );

        let plan = plan_url_backfill(&row, UrlSourceTable::ApiEndpoints);

        let origin = plan.origin.unwrap();
        assert_eq!(origin.origin, "https://a.example.com:443");
        assert_eq!(origin.host_type, "domain");
        assert!(plan.missing_endpoint);
    }

    #[test]
    fn explicit_https_ports_do_not_merge_with_default_443() {
        let default_row = url_row(
            "https://a.example.com/login",
            "domain",
            "a.example.com",
            "",
            json!([]),
        );
        let explicit_row = url_row(
            "https://a.example.com:8443/login",
            "domain",
            "a.example.com",
            "",
            json!([]),
        );

        let default_origin = plan_url_backfill(&default_row, UrlSourceTable::ApiEndpoints)
            .origin
            .unwrap();
        let explicit_origin = plan_url_backfill(&explicit_row, UrlSourceTable::ApiEndpoints)
            .origin
            .unwrap();

        assert_eq!(default_origin.origin, "https://a.example.com:443");
        assert_eq!(explicit_origin.origin, "https://a.example.com:8443");
        assert_ne!(default_origin.origin, explicit_origin.origin);
    }

    #[test]
    fn http_urls_default_to_port_80() {
        let row = url_row(
            "http://a.example.com/login",
            "domain",
            "a.example.com",
            "",
            json!([]),
        );

        let origin = plan_url_backfill(&row, UrlSourceTable::ApiEndpoints)
            .origin
            .unwrap();

        assert_eq!(origin.origin, "http://a.example.com:80");
        assert_eq!(origin.port, 80);
    }

    #[test]
    fn ip_literal_url_creates_ip_web_origin_and_confirmed_observation_endpoint() {
        let row = url_row("https://1.1.1.1/login", "ip", "1.1.1.1", "", json!([]));

        let plan = plan_url_backfill(&row, UrlSourceTable::DirectoryEntries);

        let origin = plan.origin.unwrap();
        let endpoint = plan.endpoint_resolution.unwrap();
        assert_eq!(origin.origin, "https://1.1.1.1:443");
        assert_eq!(origin.host_type, "ip");
        assert_eq!(endpoint.endpoint.identity.ip, "1.1.1.1");
        assert_eq!(endpoint.endpoint.identity.port, 443);
        assert_eq!(endpoint.observation_kind, ObservationKind::Confirmed);
    }

    #[test]
    fn relative_urls_are_skipped_without_origin_assignment() {
        for raw in [
            "/login",
            "/api/user",
            "app.js",
            "static/main.js",
            "//cdn.example.com/app.js",
        ] {
            let row = url_row(raw, "ip", "1.1.1.1", "", json!([{ "port": 443 }]));
            let plan = plan_url_backfill(&row, UrlSourceTable::ApiEndpoints);
            assert!(plan.origin.is_none(), "{raw} must not produce a WebOrigin");
            assert_eq!(plan.skip, Some(UrlBackfillSkip::Relative));
        }
    }

    #[test]
    fn same_ip_with_multiple_domains_keeps_one_endpoint_two_origins_two_observations() {
        let target = target_row("ip", "1.1.1.1", json!([{ "port": 443, "proto": "tcp" }]));
        let rows = vec![
            url_row(
                "https://a.example.com/login",
                "ip",
                "1.1.1.1",
                "",
                json!([{ "port": 443, "proto": "tcp" }]),
            ),
            url_row(
                "https://b.example.com/login",
                "ip",
                "1.1.1.1",
                "",
                json!([{ "port": 443, "proto": "tcp" }]),
            ),
        ];

        let (endpoints, origins, observations) = collect_plan_keys(&[target], &rows);

        assert_eq!(endpoints.len(), 1);
        assert!(endpoints.contains(&("1.1.1.1".to_string(), 443, "tcp".to_string())));
        assert_eq!(origins.len(), 2);
        assert!(origins.contains(&("https".to_string(), "a.example.com".to_string(), 443)));
        assert!(origins.contains(&("https".to_string(), "b.example.com".to_string(), 443)));
        assert_eq!(observations.len(), 2);
    }

    #[test]
    fn same_domain_with_multiple_real_ips_keeps_one_origin_two_endpoints_two_observations() {
        let rows = vec![
            url_row(
                "https://a.example.com/login",
                "domain",
                "a.example.com",
                "1.1.1.1",
                json!([]),
            ),
            url_row(
                "https://a.example.com/login",
                "domain",
                "a.example.com",
                "2.2.2.2",
                json!([]),
            ),
        ];

        let (endpoints, origins, observations) = collect_plan_keys(&[], &rows);

        assert_eq!(origins.len(), 1);
        assert_eq!(endpoints.len(), 2);
        assert!(endpoints.contains(&("1.1.1.1".to_string(), 443, "tcp".to_string())));
        assert!(endpoints.contains(&("2.2.2.2".to_string(), 443, "tcp".to_string())));
        assert_eq!(observations.len(), 2);
    }

    #[test]
    fn backfill_identity_keys_are_stable_across_repeated_runs() {
        let target = target_row("ip", "1.1.1.1", json!([{ "port": 443, "proto": "tcp" }]));
        let rows = vec![url_row(
            "https://a.example.com/login",
            "ip",
            "1.1.1.1",
            "",
            json!([{ "port": 443, "proto": "tcp" }]),
        )];

        let once = collect_plan_keys(std::slice::from_ref(&target), &rows);
        let twice = collect_plan_keys(&[target], &rows);

        assert_eq!(once.0, twice.0, "network endpoint identity must be stable");
        assert_eq!(once.1, twice.1, "web origin identity must be stable");
        assert_eq!(once.2, twice.2, "observation dedupe key must be stable");
    }

    #[test]
    fn unsupported_and_malformed_urls_have_separate_skip_classes() {
        let ftp = url_row(
            "ftp://a.example.com/file",
            "domain",
            "a.example.com",
            "",
            json!([]),
        );
        let bad_http = url_row("https://", "domain", "a.example.com", "", json!([]));

        assert_eq!(
            plan_url_backfill(&ftp, UrlSourceTable::ApiEndpoints).skip,
            Some(UrlBackfillSkip::UnsupportedScheme)
        );
        assert_eq!(
            plan_url_backfill(&bad_http, UrlSourceTable::ApiEndpoints).skip,
            Some(UrlBackfillSkip::Malformed)
        );
    }

    #[test]
    fn organization_null_scope_uses_row_project_path_fallback() {
        assert_eq!(effective_project_path("", "proj-target"), "proj-target");
        assert_eq!(
            effective_project_path("proj-source", "proj-target"),
            "proj-source"
        );
        assert_eq!(effective_project_path("", ""), "");
    }

    #[test]
    fn legacy_child_queries_skip_known_project_conflicts() {
        for (sql, alias) in [
            (LIST_TARGET_ASSET_ROWS_SQL, "ta"),
            (LIST_API_URL_ROWS_SQL, "ae"),
            (LIST_JS_URL_ROWS_SQL, "ja"),
            (LIST_DIRECTORY_URL_ROWS_SQL, "de"),
            (LIST_PASSIVE_URL_ROWS_SQL, "ps"),
        ] {
            assert!(sql.contains(&format!("NULLIF({alias}.project_path, '') IS NULL")));
            assert!(sql.contains("NULLIF(t.project_path, '') IS NULL"));
            assert!(sql.contains(&format!("{alias}.project_path = t.project_path")));
        }
    }
}
