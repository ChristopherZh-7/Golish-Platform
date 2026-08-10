//! Server-owned, bounded Application Understanding work-item projections.
//!
//! This module is intentionally separate from the provider bridge: it accepts
//! typed database rows, emits a closed redacted JSON projection, and never
//! reads capture files or forwards predecessor handoff payloads.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context as _;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

const MAX_ROUTES_PER_WORK_ITEM: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionManifestInput {
    pub input_key: String,
    pub input_kind: String,
    pub source_id: String,
    pub source_version: i64,
    pub content_hash: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionTarget {
    pub id: Uuid,
    pub target_type: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionOrigin {
    pub id: Uuid,
    pub origin: String,
    pub target_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionParameter {
    pub name: String,
    pub location: String,
    pub value_type: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionRoute {
    pub origin_id: Uuid,
    pub method: String,
    pub url: String,
    pub parameters: Vec<ProjectionParameter>,
    pub status_code: Option<i32>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectionFingerprint {
    pub origin_id: Uuid,
    pub category: String,
    pub name: String,
    pub version: Option<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionService {
    pub target_id: Uuid,
    pub web_origin_id: Option<Uuid>,
    pub host: String,
    pub port: i32,
    pub transport: String,
    pub service: Option<String>,
    pub product: Option<String>,
    pub version: Option<String>,
    pub tls: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionOutcome {
    pub asset: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApplicationProjectionSource {
    pub operation_id: Uuid,
    pub manifest_id: Uuid,
    pub manifest_hash: String,
    pub organization_id: Uuid,
    pub inputs: Vec<ProjectionManifestInput>,
    pub targets: Vec<ProjectionTarget>,
    pub origins: Vec<ProjectionOrigin>,
    pub routes: Vec<ProjectionRoute>,
    pub fingerprints: Vec<ProjectionFingerprint>,
    pub services: Vec<ProjectionService>,
    pub outcomes: Vec<ProjectionOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectedApplicationWorkItem {
    pub work_item_key: String,
    pub work_item_kind: String,
    pub projection_hash: String,
    pub evidence_ids: Vec<i64>,
    pub source_input_keys: Vec<String>,
    pub projection: Value,
}

pub(crate) fn build_application_work_item_projections(
    source: &ApplicationProjectionSource,
) -> anyhow::Result<Vec<ProjectedApplicationWorkItem>> {
    let authorized_evidence = source
        .inputs
        .iter()
        .flat_map(|input| input.evidence_ids.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut projections = Vec::new();

    let mut origins = source.origins.clone();
    origins.sort_by(|left, right| {
        sanitize_origin(&left.origin)
            .cmp(&sanitize_origin(&right.origin))
            .then_with(|| left.id.cmp(&right.id))
    });
    for origin in origins {
        let normalized_origin = sanitize_origin(&origin.origin);
        let origin_target_ids = origin.target_ids.iter().copied().collect::<BTreeSet<_>>();
        let origin_targets = source
            .targets
            .iter()
            .filter(|target| origin_target_ids.contains(&target.id))
            .collect::<Vec<_>>();
        let mut subjects = vec![json!({
            "kind": "web_origin",
            "value": normalized_origin,
        })];
        subjects.extend(
            origin_targets
                .iter()
                .filter_map(|target| project_target_subject(target)),
        );
        let mut routes = source
            .routes
            .iter()
            .filter(|route| route.origin_id == origin.id)
            .map(project_route)
            .collect::<anyhow::Result<Vec<_>>>()?;
        sort_values(&mut routes);
        routes.dedup();

        let mut fingerprints = source
            .fingerprints
            .iter()
            .filter(|fingerprint| fingerprint.origin_id == origin.id)
            .map(|fingerprint| {
                json!({
                    "category": fingerprint.category,
                    "confidence": (fingerprint.confidence.clamp(0.0, 1.0) * 100.0).round() as u8,
                    "name": fingerprint.name,
                    "version": fingerprint.version,
                })
            })
            .collect::<Vec<_>>();
        sort_values(&mut fingerprints);
        fingerprints.dedup();

        let mut services = source
            .services
            .iter()
            .filter(|service| service.web_origin_id == Some(origin.id))
            .map(project_service)
            .collect::<Vec<_>>();
        sort_values(&mut services);
        services.dedup();

        let mut origin_assets = vec![normalized_origin.clone()];
        origin_assets.extend(
            origin_targets
                .iter()
                .map(|target| sanitize_asset(&target.value)),
        );
        let evidence_ids = evidence_with_manifest_fallback(
            evidence_for_assets(&source.outcomes, origin_assets.iter(), &authorized_evidence),
            &authorized_evidence,
        );
        let source_input_keys = source_input_keys(source, &evidence_ids);
        let chunks = if routes.is_empty() {
            vec![Vec::new()]
        } else {
            routes
                .chunks(MAX_ROUTES_PER_WORK_ITEM)
                .map(<[Value]>::to_vec)
                .collect::<Vec<_>>()
        };
        let chunk_count = chunks.len();
        for (chunk_index, route_chunk) in chunks.into_iter().enumerate() {
            let key = if chunk_count == 1 {
                format!("web_origin:{}", origin.id)
            } else {
                format!("web_origin:{}:chunk:{chunk_index:04}", origin.id)
            };
            projections.push(make_projection(
                source,
                key,
                "web_origin",
                subjects.clone(),
                services.clone(),
                route_chunk,
                fingerprints.clone(),
                evidence_ids.clone(),
                source_input_keys.clone(),
            )?);
        }
    }

    let mut services_by_host = BTreeMap::<String, Vec<&ProjectionService>>::new();
    for service in source
        .services
        .iter()
        .filter(|service| service.web_origin_id.is_none())
    {
        services_by_host
            .entry(sanitize_asset(&service.host).to_ascii_lowercase())
            .or_default()
            .push(service);
    }
    for (host, mut services) in services_by_host {
        services
            .sort_by_key(|service| (service.port, service.transport.clone(), service.target_id));
        let projected_services = services
            .iter()
            .map(|service| project_service(service))
            .collect::<Vec<_>>();
        let asset_keys = services
            .iter()
            .flat_map(|service| {
                [
                    sanitize_asset(&service.host).to_ascii_lowercase(),
                    format!(
                        "{}:{}",
                        sanitize_asset(&service.host).to_ascii_lowercase(),
                        service.port
                    ),
                    format!(
                        "{}://{}:{}",
                        service
                            .service
                            .as_deref()
                            .unwrap_or(&service.transport)
                            .to_ascii_lowercase(),
                        sanitize_asset(&service.host).to_ascii_lowercase(),
                        service.port
                    ),
                ]
            })
            .collect::<BTreeSet<_>>();
        let evidence_ids = evidence_with_manifest_fallback(
            evidence_for_assets(&source.outcomes, asset_keys.iter(), &authorized_evidence),
            &authorized_evidence,
        );
        let source_input_keys = source_input_keys(source, &evidence_ids);
        projections.push(make_projection(
            source,
            format!("service_host:{}", stable_fragment(&host)),
            "service_host",
            vec![json!({
                "kind": if host.parse::<std::net::IpAddr>().is_ok() { "ip" } else { "host" },
                "value": host,
            })],
            projected_services,
            Vec::new(),
            Vec::new(),
            evidence_ids,
            source_input_keys,
        )?);
    }

    let known_assets = known_asset_keys(source);
    let mut unknown_assets = BTreeMap::<String, Vec<i64>>::new();
    for outcome in &source.outcomes {
        let asset = sanitize_asset(&outcome.asset);
        if asset.is_empty() || known_assets.contains(&asset.to_ascii_lowercase()) {
            continue;
        }
        let evidence = unknown_assets.entry(asset).or_default();
        evidence.extend(
            outcome
                .evidence_ids
                .iter()
                .copied()
                .filter(|id| authorized_evidence.contains(id)),
        );
    }
    for (asset, mut evidence_ids) in unknown_assets {
        evidence_ids.sort_unstable();
        evidence_ids.dedup();
        let source_input_keys = source_input_keys(source, &evidence_ids);
        projections.push(make_projection(
            source,
            format!("unknown_asset:{}", stable_fragment(&asset)),
            "unknown_asset",
            vec![json!({"kind": "unknown_asset", "value": asset})],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            evidence_ids,
            source_input_keys,
        )?);
    }

    if projections.is_empty() && !source.inputs.is_empty() {
        let evidence_ids = authorized_evidence.iter().copied().collect::<Vec<_>>();
        projections.push(make_projection(
            source,
            "unknown_asset:manifest-unmapped".to_string(),
            "unknown_asset",
            vec![json!({"kind": "unknown_asset", "value": "manifest-unmapped"})],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            evidence_ids.clone(),
            source_input_keys(source, &evidence_ids),
        )?);
    }

    projections.sort_by(|left, right| {
        left.work_item_kind
            .cmp(&right.work_item_kind)
            .then_with(|| left.work_item_key.cmp(&right.work_item_key))
    });
    Ok(projections)
}

/// Load only typed, allowlisted columns for one frozen company manifest. Raw
/// handoff payloads, audit details, banners, captures, request headers and
/// response bodies are deliberately absent from every query in this loader.
pub(crate) async fn load_application_projection_source(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    manifest: &golish_db::repo::application_models::ApplicationModelManifestRow,
    manifest_inputs: &[golish_db::repo::application_models::ApplicationModelManifestInputRow],
) -> anyhow::Result<ApplicationProjectionSource> {
    let targets = sqlx::query_as::<_, (Uuid, String, String)>(
        r#"SELECT id,target_type::TEXT,value
             FROM targets
            WHERE organization_id=$1 AND scope='in'
            ORDER BY target_type::TEXT,value,id"#,
    )
    .bind(manifest.organization_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, target_type, value)| ProjectionTarget {
        id,
        target_type,
        value,
    })
    .collect();

    let origins = sqlx::query_as::<_, (Uuid, String, Vec<Uuid>)>(
        r#"SELECT origin.id,origin.origin,
                  COALESCE(array_agg(DISTINCT observation.target_id)
                           FILTER (WHERE observation.target_id IS NOT NULL),'{}') AS target_ids
             FROM web_origins AS origin
             LEFT JOIN web_origin_observations AS observation
               ON observation.web_origin_id=origin.id
              AND observation.organization_id=origin.organization_id
            WHERE origin.organization_id=$1
            GROUP BY origin.id,origin.origin
            ORDER BY origin.origin,origin.id"#,
    )
    .bind(manifest.organization_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, origin, target_ids)| ProjectionOrigin {
        id,
        origin,
        target_ids,
    })
    .collect();

    let parameter_rows = sqlx::query_as::<_, (Uuid, String, String, String, bool)>(
        r#"SELECT parameter.endpoint_observation_id,parameter.name,parameter.location,
                  parameter.value_type,parameter.required
             FROM enumeration_endpoint_parameters AS parameter
             JOIN enumeration_endpoint_observations AS observation
               ON observation.id=parameter.endpoint_observation_id
            WHERE observation.operation_id=$1 AND observation.organization_id=$2
              AND parameter.location <> 'unknown'
            ORDER BY parameter.endpoint_observation_id,parameter.location,parameter.name"#,
    )
    .bind(operation_id)
    .bind(manifest.organization_id)
    .fetch_all(pool)
    .await?;
    let mut parameters_by_observation = BTreeMap::<Uuid, Vec<ProjectionParameter>>::new();
    for (observation_id, name, location, value_type, required) in parameter_rows {
        parameters_by_observation
            .entry(observation_id)
            .or_default()
            .push(ProjectionParameter {
                name,
                location,
                value_type,
                required,
            });
    }
    let routes = sqlx::query_as::<_, (Uuid, Uuid, String, String, Option<i32>, Option<String>)>(
        r#"SELECT observation.id,observation.web_origin_id,endpoint.method,endpoint.url,
                  endpoint.status_code,endpoint.response_type
             FROM enumeration_endpoint_observations AS observation
             JOIN api_endpoints AS endpoint ON endpoint.id=observation.endpoint_id
            WHERE observation.operation_id=$1 AND observation.organization_id=$2
            ORDER BY observation.web_origin_id,endpoint.method,endpoint.url,endpoint.id"#,
    )
    .bind(operation_id)
    .bind(manifest.organization_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(
        |(observation_id, origin_id, method, url, status_code, content_type)| ProjectionRoute {
            origin_id,
            method,
            url,
            parameters: parameters_by_observation
                .remove(&observation_id)
                .unwrap_or_default(),
            status_code,
            content_type,
        },
    )
    .collect();

    let fingerprints = sqlx::query_as::<_, (Uuid, String, String, Option<String>, f32)>(
        r#"SELECT observation.web_origin_id,fingerprint.category,fingerprint.name,
                  fingerprint.version,fingerprint.confidence
             FROM fingerprint_origin_observations AS observation
             JOIN fingerprints AS fingerprint ON fingerprint.id=observation.fingerprint_id
            WHERE observation.organization_id=$1
            ORDER BY observation.web_origin_id,fingerprint.category,fingerprint.name,
                     fingerprint.version NULLS FIRST,fingerprint.id"#,
    )
    .bind(manifest.organization_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(
        |(origin_id, category, name, version, confidence)| ProjectionFingerprint {
            origin_id,
            category,
            name,
            version,
            confidence,
        },
    )
    .collect();

    let services = sqlx::query_as::<
        _,
        (
            Uuid,
            Option<Uuid>,
            String,
            i32,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            bool,
        ),
    >(
        r#"SELECT endpoint.id,
                  (SELECT observation.web_origin_id
                     FROM web_origin_observations AS observation
                    WHERE observation.network_endpoint_id=endpoint.id
                      AND observation.organization_id=endpoint.organization_id
                    ORDER BY observation.web_origin_id LIMIT 1) AS web_origin_id,
                  endpoint.ip,endpoint.port,endpoint.transport,endpoint.service_name,
                  endpoint.service_product,endpoint.service_version,endpoint.tls_detected
             FROM network_endpoints AS endpoint
            WHERE endpoint.organization_id=$1 AND endpoint.state='open'
              AND endpoint.transport IN ('tcp','udp')
            ORDER BY endpoint.ip,endpoint.transport,endpoint.port,endpoint.id"#,
    )
    .bind(manifest.organization_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(
        |(target_id, web_origin_id, host, port, transport, service, product, version, tls)| {
            ProjectionService {
                target_id,
                web_origin_id,
                host,
                port,
                transport,
                service,
                product,
                version,
                tls,
            }
        },
    )
    .collect();

    let manifest_evidence_ids = manifest_inputs
        .iter()
        .flat_map(|input| input.evidence_ids.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let outcomes = if manifest_evidence_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, (String, Vec<i64>)>(
            r#"SELECT asset,evidence_ids
                 FROM technique_outcomes
                WHERE organization_id=$1 AND outcome='found'
                  AND evidence_ids && $2::BIGINT[]
                ORDER BY asset,technique,id"#,
        )
        .bind(manifest.organization_id)
        .bind(&manifest_evidence_ids)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(asset, evidence_ids)| ProjectionOutcome {
            asset,
            evidence_ids,
        })
        .collect()
    };

    Ok(ApplicationProjectionSource {
        operation_id,
        manifest_id: manifest.id,
        manifest_hash: manifest.manifest_hash.clone(),
        organization_id: manifest.organization_id,
        inputs: manifest_inputs
            .iter()
            .map(|input| {
                let content_hash = input
                    .source_payload_hash
                    .strip_prefix("sha256:")
                    .filter(|hash| {
                        hash.len() == 64
                            && hash
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    })
                    .context("manifest input content hash is not canonical sha256")?;
                Ok(ProjectionManifestInput {
                    input_key: input.input_key.clone(),
                    input_kind: input.input_kind.clone(),
                    source_id: input.source_id.clone(),
                    source_version: input.source_version,
                    content_hash: content_hash.to_string(),
                    evidence_ids: input.evidence_ids.clone(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        targets,
        origins,
        routes,
        fingerprints,
        services,
        outcomes,
    })
}

fn project_route(route: &ProjectionRoute) -> anyhow::Result<Value> {
    let parsed = Url::parse(&route.url).with_context(|| {
        format!(
            "invalid URL in typed route projection for origin {}",
            route.origin_id
        )
    })?;
    let mut parameters = route
        .parameters
        .iter()
        .map(|parameter| {
            let location = match parameter.location.as_str() {
                "body_or_form" | "form" | "graphql_variable" => "body",
                value => value,
            };
            json!({
                "location": location,
                "name": safe_parameter_token(&parameter.name, 128),
                "required": parameter.required,
                "value_type": safe_parameter_token(&parameter.value_type, 64),
            })
        })
        .collect::<Vec<_>>();
    sort_values(&mut parameters);
    parameters.dedup();
    let host = parsed
        .host_str()
        .context("typed route URL has no host")?
        .to_ascii_lowercase();
    let port = parsed
        .port_or_known_default()
        .context("typed route URL has no effective port")?;
    Ok(json!({
        "content_type": route.content_type,
        "host": host,
        "method": route.method.to_ascii_uppercase(),
        "parameters": parameters,
        "port": port,
        "route_shape": redact_route_shape(parsed.path()),
        "scheme": parsed.scheme(),
        "status_code": route.status_code,
    }))
}

fn project_target_subject(target: &ProjectionTarget) -> Option<Value> {
    let (kind, value) = match target.target_type.as_str() {
        "domain" => ("host", sanitize_asset(&target.value).to_ascii_lowercase()),
        "ip" => ("ip", sanitize_asset(&target.value)),
        "cidr" => ("cidr", sanitize_asset(&target.value)),
        "url" => ("web_origin", sanitize_origin(&target.value)),
        "wildcard" => (
            "unknown_asset",
            sanitize_asset(&target.value).to_ascii_lowercase(),
        ),
        _ => return None,
    };
    Some(json!({"kind": kind, "value": value}))
}

fn safe_parameter_token(value: &str, max: usize) -> String {
    let mut sanitized = value
        .trim()
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'[' | b']') {
                byte as char
            } else {
                '_'
            }
        })
        .take(max)
        .collect::<String>();
    if sanitized.is_empty() {
        sanitized.push_str("unknown");
    }
    sanitized
}

fn redact_route_shape(path: &str) -> String {
    let mut redacted = path
        .split('/')
        .map(|segment| {
            let parameter_marker = (segment.starts_with('{') && segment.ends_with('}'))
                || segment.starts_with(':')
                || segment == "*";
            let looks_dynamic = !parameter_marker
                && (segment.len() > 64
                    || segment.contains('%')
                    || segment.contains('@')
                    || segment.parse::<u128>().is_ok()
                    || Uuid::parse_str(segment).is_ok()
                    || (segment.len() >= 12
                        && segment.bytes().all(|byte| byte.is_ascii_hexdigit())));
            if looks_dynamic {
                "{value}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    if redacted.is_empty() {
        redacted.push('/');
    }
    redacted
}

fn project_service(service: &ProjectionService) -> Value {
    json!({
        "host": sanitize_asset(&service.host).to_ascii_lowercase(),
        "port": service.port,
        "product": service.product,
        "service_name": service.service,
        "tls": service.tls,
        "transport": service.transport.to_ascii_lowercase(),
        "version": service.version,
    })
}

#[allow(clippy::too_many_arguments)]
fn make_projection(
    source: &ApplicationProjectionSource,
    work_item_key: String,
    work_item_kind: &str,
    mut subjects: Vec<Value>,
    services: Vec<Value>,
    routes: Vec<Value>,
    fingerprints: Vec<Value>,
    evidence_ids: Vec<i64>,
    source_input_keys: Vec<String>,
) -> anyhow::Result<ProjectedApplicationWorkItem> {
    sort_values(&mut subjects);
    subjects.dedup();
    let selected_input_keys = source_input_keys.iter().collect::<BTreeSet<_>>();
    let authorized_evidence = evidence_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut manifest_inputs = source
        .inputs
        .iter()
        .filter(|input| selected_input_keys.contains(&input.input_key))
        .map(|input| {
            let evidence_ids = input
                .evidence_ids
                .iter()
                .copied()
                .filter(|id| authorized_evidence.contains(id))
                .collect::<Vec<_>>();
            json!({
                "content_hash": input.content_hash,
                "evidence_ids": evidence_ids,
                "input_key": input.input_key,
                "input_kind": input.input_kind,
                "source_id": input.source_id,
                "source_version": input.source_version,
            })
        })
        .collect::<Vec<_>>();
    sort_values(&mut manifest_inputs);
    let projection = json!({
        "fingerprints": fingerprints,
        "manifest_inputs": manifest_inputs,
        "projection_incomplete": false,
        "routes": routes,
        "services": services,
        "subjects": subjects,
    });
    let typed_projection = serde_json::from_value::<
        golish_agent_kit::task_orchestrator::ApplicationModelWorkItemProjectionContract,
    >(projection)
    .context("validate closed application work-item projection")?;
    let projection = canonicalize(
        serde_json::to_value(typed_projection).context("serialize typed application projection")?,
    );
    let bytes = serde_json::to_vec(&projection).context("serialize application projection")?;
    let projection_hash = format!("sha256:{}", sha256_hex(&bytes));
    Ok(ProjectedApplicationWorkItem {
        work_item_key,
        work_item_kind: work_item_kind.to_string(),
        projection_hash,
        evidence_ids,
        source_input_keys,
        projection,
    })
}

fn evidence_for_assets<'a>(
    outcomes: &[ProjectionOutcome],
    assets: impl IntoIterator<Item = &'a String>,
    authorized: &BTreeSet<i64>,
) -> Vec<i64> {
    let assets = assets
        .into_iter()
        .map(|asset| sanitize_asset(asset).to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut evidence = outcomes
        .iter()
        .filter(|outcome| assets.contains(&sanitize_asset(&outcome.asset).to_ascii_lowercase()))
        .flat_map(|outcome| outcome.evidence_ids.iter().copied())
        .filter(|id| authorized.contains(id))
        .collect::<Vec<_>>();
    evidence.sort_unstable();
    evidence.dedup();
    evidence
}

fn evidence_with_manifest_fallback(evidence_ids: Vec<i64>, authorized: &BTreeSet<i64>) -> Vec<i64> {
    if evidence_ids.is_empty() {
        authorized.iter().copied().collect()
    } else {
        evidence_ids
    }
}

fn source_input_keys(source: &ApplicationProjectionSource, evidence_ids: &[i64]) -> Vec<String> {
    let evidence_ids = evidence_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut keys = source
        .inputs
        .iter()
        .filter(|input| {
            evidence_ids.is_empty()
                || input
                    .evidence_ids
                    .iter()
                    .any(|id| evidence_ids.contains(id))
        })
        .map(|input| input.input_key.clone())
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    keys
}

fn known_asset_keys(source: &ApplicationProjectionSource) -> BTreeSet<String> {
    let mut known = BTreeSet::new();
    for target in &source.targets {
        known.insert(sanitize_asset(&target.value).to_ascii_lowercase());
    }
    for origin in &source.origins {
        known.insert(sanitize_origin(&origin.origin).to_ascii_lowercase());
    }
    for service in &source.services {
        known.insert(sanitize_asset(&service.host).to_ascii_lowercase());
        known.insert(format!(
            "{}:{}",
            sanitize_asset(&service.host).to_ascii_lowercase(),
            service.port
        ));
        known.insert(
            format!(
                "{}://{}:{}",
                service
                    .service
                    .as_deref()
                    .unwrap_or(&service.transport)
                    .to_ascii_lowercase(),
                sanitize_asset(&service.host).to_ascii_lowercase(),
                service.port
            )
            .to_ascii_lowercase(),
        );
    }
    known
}

fn sanitize_origin(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?;
            let port = url
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default();
            Some(format!(
                "{}://{}{}",
                url.scheme(),
                host.to_ascii_lowercase(),
                port
            ))
        })
        .unwrap_or_else(|| format!("redacted-invalid-origin:{}", stable_fragment(value)))
}

fn sanitize_asset(value: &str) -> String {
    if let Ok(mut url) = Url::parse(value) {
        if url.host_str().is_some() {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            return url.to_string().trim_end_matches('/').to_string();
        }
    }
    let trimmed = value.trim();
    let is_bounded_identity = !trimmed.is_empty()
        && trimmed.len() <= 255
        && trimmed.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'-' | b'_' | b':' | b'/' | b'*' | b'[' | b']')
        });
    if is_bounded_identity {
        trimmed.to_string()
    } else {
        format!("redacted-asset:{}", stable_fragment(value))
    }
}

fn stable_fragment(value: &str) -> String {
    sha256_hex(value.as_bytes())[..16].to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        Value::Object(values) => {
            let mut sorted = BTreeMap::new();
            for (key, value) in values {
                sorted.insert(key, canonicalize(value));
            }
            Value::Object(sorted.into_iter().collect::<Map<_, _>>())
        }
        scalar => scalar,
    }
}

fn sort_values(values: &mut [Value]) {
    values.sort_by_key(|value| {
        serde_json::to_string(&canonicalize(value.clone())).unwrap_or_default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source() -> ApplicationProjectionSource {
        let operation_id = Uuid::new_v4();
        let manifest_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        let origin_id = Uuid::new_v4();
        ApplicationProjectionSource {
            operation_id,
            manifest_id,
            manifest_hash: format!("sha256:{}", "a".repeat(64)),
            organization_id,
            inputs: vec![ProjectionManifestInput {
                input_key: "enumeration:handoff".to_string(),
                input_kind: "enumeration".to_string(),
                source_id: "handoff-fixture".to_string(),
                source_version: 1,
                content_hash: "b".repeat(64),
                evidence_ids: vec![11, 12],
            }],
            targets: vec![ProjectionTarget {
                id: target_id,
                target_type: "url".to_string(),
                value: "https://user:secret@example.test/orders?token=secret#fragment".to_string(),
            }],
            origins: vec![ProjectionOrigin {
                id: origin_id,
                origin: "https://example.test".to_string(),
                target_ids: vec![target_id],
            }],
            routes: vec![
                ProjectionRoute {
                    origin_id,
                    method: "get".to_string(),
                    url: "https://example.test/orders/123456?id=secret&token=secret".to_string(),
                    parameters: vec![
                        ProjectionParameter {
                            name: "token".to_string(),
                            location: "query".to_string(),
                            value_type: "string".to_string(),
                            required: false,
                        },
                        ProjectionParameter {
                            name: "id".to_string(),
                            location: "query".to_string(),
                            value_type: "string".to_string(),
                            required: false,
                        },
                    ],
                    status_code: Some(200),
                    content_type: Some("application/json".to_string()),
                },
                ProjectionRoute {
                    origin_id,
                    method: "GET".to_string(),
                    url: "https://example.test/orders/987654?id=other".to_string(),
                    parameters: vec![
                        ProjectionParameter {
                            name: "id".to_string(),
                            location: "query".to_string(),
                            value_type: "string".to_string(),
                            required: false,
                        },
                        ProjectionParameter {
                            name: "token".to_string(),
                            location: "query".to_string(),
                            value_type: "string".to_string(),
                            required: false,
                        },
                    ],
                    status_code: Some(200),
                    content_type: Some("application/json".to_string()),
                },
            ],
            fingerprints: vec![ProjectionFingerprint {
                origin_id,
                category: "framework".to_string(),
                name: "ExampleFW".to_string(),
                version: Some("1.2".to_string()),
                confidence: 0.9,
            }],
            services: vec![ProjectionService {
                target_id,
                web_origin_id: Some(origin_id),
                host: "example.test".to_string(),
                port: 443,
                transport: "tcp".to_string(),
                service: Some("https".to_string()),
                product: Some("Example Server".to_string()),
                version: Some("1.0".to_string()),
                tls: true,
            }],
            outcomes: vec![ProjectionOutcome {
                asset: "https://example.test".to_string(),
                evidence_ids: vec![12, 999],
            }],
        }
    }

    #[test]
    fn application_understanding_projection_redacts_values_and_deduplicates_routes() {
        let projections = build_application_work_item_projections(&source()).unwrap();
        assert_eq!(projections.len(), 1);
        let projection = &projections[0];
        assert_eq!(projection.work_item_kind, "web_origin");
        assert_eq!(projection.evidence_ids, vec![12]);
        assert_eq!(projection.source_input_keys, vec!["enumeration:handoff"]);
        assert_eq!(projection.projection["routes"].as_array().unwrap().len(), 1);
        assert_eq!(projection.projection["routes"][0]["method"], json!("GET"));
        assert_eq!(
            projection.projection["routes"][0]["route_shape"],
            json!("/orders/{value}")
        );
        assert_eq!(
            projection.projection["routes"][0]["parameters"],
            json!([
                {"location":"query","name":"id","required":false,"value_type":"string"},
                {"location":"query","name":"token","required":false,"value_type":"string"}
            ])
        );
        let encoded = serde_json::to_string(&projection.projection).unwrap();
        for forbidden in [
            "secret",
            "userinfo",
            "fragment",
            "cookie",
            "authorization",
            "capture_path",
            "raw_output",
            "body_base64",
            "banner",
        ] {
            assert!(
                !encoded.to_ascii_lowercase().contains(forbidden),
                "{forbidden}"
            );
        }
    }

    #[test]
    fn application_understanding_projection_maps_body_carrier_locations_to_safe_body() {
        let mut source = source();
        source.routes[0].parameters = vec![
            ProjectionParameter {
                name: "form_field".to_string(),
                location: "form".to_string(),
                value_type: "string".to_string(),
                required: false,
            },
            ProjectionParameter {
                name: "graphql_variable".to_string(),
                location: "graphql_variable".to_string(),
                value_type: "string".to_string(),
                required: true,
            },
            ProjectionParameter {
                name: "legacy_body".to_string(),
                location: "body_or_form".to_string(),
                value_type: "string".to_string(),
                required: false,
            },
        ];
        source.routes.truncate(1);

        let projections = build_application_work_item_projections(&source).unwrap();
        assert_eq!(
            projections[0].projection["routes"][0]["parameters"],
            json!([
                {"location":"body","name":"form_field","required":false,"value_type":"string"},
                {"location":"body","name":"graphql_variable","required":true,"value_type":"string"},
                {"location":"body","name":"legacy_body","required":false,"value_type":"string"}
            ])
        );
    }

    #[test]
    fn application_understanding_projection_never_falls_back_to_raw_malformed_urls() {
        let invalid_origin = "not a url?token=do-not-forward";
        let invalid_asset = "opaque?Cookie=do-not-forward";
        assert!(sanitize_origin(invalid_origin).starts_with("redacted-invalid-origin:"));
        assert!(sanitize_asset(invalid_asset).starts_with("redacted-asset:"));
        assert!(!sanitize_origin(invalid_origin).contains("do-not-forward"));
        assert!(!sanitize_asset(invalid_asset).contains("do-not-forward"));
    }

    #[test]
    fn application_understanding_projection_groups_uncovered_services_and_unknown_assets() {
        let mut source = source();
        let service_target_id = Uuid::new_v4();
        source.targets.push(ProjectionTarget {
            id: service_target_id,
            target_type: "ip".to_string(),
            value: "192.0.2.10".to_string(),
        });
        source.services.push(ProjectionService {
            target_id: service_target_id,
            web_origin_id: None,
            host: "192.0.2.10".to_string(),
            port: 22,
            transport: "tcp".to_string(),
            service: Some("ssh".to_string()),
            product: Some("OpenSSH".to_string()),
            version: Some("9.0".to_string()),
            tls: false,
        });
        source.outcomes.push(ProjectionOutcome {
            asset: "opaque-asset-with-no-row".to_string(),
            evidence_ids: vec![11],
        });
        let projections = build_application_work_item_projections(&source).unwrap();
        assert_eq!(
            projections
                .iter()
                .map(|item| item.work_item_kind.as_str())
                .collect::<Vec<_>>(),
            vec!["service_host", "unknown_asset", "web_origin"]
        );
        assert!(projections
            .iter()
            .any(|item| item.projection["services"][0]["port"] == json!(22)));
        assert!(projections.iter().any(|item| {
            item.work_item_kind == "unknown_asset"
                && item.projection["subjects"]
                    == json!([{"kind":"unknown_asset","value":"opaque-asset-with-no-row"}])
        }));
    }

    #[test]
    fn application_understanding_projection_hash_is_stable_under_row_reordering() {
        let source = source();
        let first = build_application_work_item_projections(&source).unwrap();
        let mut reordered = source.clone();
        reordered.routes.reverse();
        reordered.inputs[0].evidence_ids.reverse();
        reordered.outcomes[0].evidence_ids.reverse();
        let second = build_application_work_item_projections(&reordered).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn application_understanding_projection_chunks_routes_without_truncation() {
        let mut source = source();
        let origin_id = source.origins[0].id;
        source.routes = (0..(MAX_ROUTES_PER_WORK_ITEM + 1))
            .map(|index| ProjectionRoute {
                origin_id,
                method: "GET".to_string(),
                url: format!(
                    "https://example.test/page/action-{}{}?token=secret",
                    char::from(b'a' + u8::try_from(index / 26).unwrap()),
                    char::from(b'a' + u8::try_from(index % 26).unwrap()),
                ),
                parameters: vec![ProjectionParameter {
                    name: "token".to_string(),
                    location: "query".to_string(),
                    value_type: "string".to_string(),
                    required: false,
                }],
                status_code: Some(200),
                content_type: Some("text/html".to_string()),
            })
            .collect();
        let projections = build_application_work_item_projections(&source).unwrap();
        assert_eq!(projections.len(), 2);
        assert_eq!(
            projections
                .iter()
                .map(|item| item.projection["routes"].as_array().unwrap().len())
                .sum::<usize>(),
            MAX_ROUTES_PER_WORK_ITEM + 1
        );
        assert_ne!(projections[0].work_item_key, projections[1].work_item_key);
    }
}
