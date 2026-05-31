//! HTTP-JSON provider runner. Moved out of `mod.rs` verbatim.

use super::super::*;

pub(crate) async fn run_http_json_provider(
    pool: &sqlx::PgPool,
    tool: &ToolConfig,
    run_id: &str,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
    sink: Option<&EventEmitterHandle>,
) -> Result<
    (
        AssetIntelProviderRunStatus,
        OrganizationCandidates,
        Value,
        Vec<ProfileFieldEntry>,
    ),
    GolishError,
> {
    let Some(asset) = tool.asset_intel.as_ref() else {
        return Err(GolishError::Validation(format!(
            "tool '{}' has no asset_intel descriptor",
            tool.id
        )));
    };
    let (provider_id, display_name) = provider_identity(tool, asset);
    let golish_pentest::models::AssetIntelRuntimeConfig::HttpJson { requests } = &asset.runtime
    else {
        return Err(GolishError::Validation(format!(
            "tool '{}' is not an http_json provider",
            tool.id
        )));
    };

    // Sequential per-request loop; accumulator stays here so every early
    // return path can hand whatever has already been hydrated up to the
    // hydrate orchestrator (we don't want to drop master-record fields
    // because a later request 500'd).
    let mut profile_entries: Vec<ProfileFieldEntry> = Vec::new();

    emit_provider_started(
        sink,
        run_id,
        &provider_id,
        display_name,
        AssetIntelProviderRuntimeKind::HttpJson,
    );

    if requests.is_empty() {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Unavailable,
            message: "http_json provider has no requests".into(),
        };
        return finish_provider_run(
            sink,
            run_id,
            status,
            0,
            OrganizationCandidates::default(),
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "unavailable",
                "reason": "no_requests",
            }),
            profile_entries,
        );
    }

    let secrets = match resolve_http_secrets(pool, asset, requests).await? {
        Ok(values) => values,
        Err(missing) => {
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("missing integration secret(s): {}", missing.join(", ")),
            };
            return finish_provider_run(
                sink,
                run_id,
                status,
                0,
                OrganizationCandidates::default(),
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "unavailable",
                    "reason": "missing_secrets",
                    "missing": missing,
                }),
                profile_entries,
            );
        }
    };

    let client = reqwest::Client::builder()
        .user_agent(concat!("golish/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| GolishError::Internal(format!("http client build failed: {err}")))?;
    let mut candidates = OrganizationCandidates::default();
    let mut request_evidence = Vec::new();
    for request in requests {
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderProgress {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                message: format!("requesting '{}' ({})", request.id, request.method),
                stream: AssetIntelStreamSource::System,
            },
        );
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|err| GolishError::Validation(format!("bad HTTP method: {err}")))?;
        let url = render_http_template(&request.url, company_name, config, &secrets);
        let timeout_secs = request.timeout_secs.clamp(1, 120);
        tracing::info!(
            provider = %provider_id,
            run_id,
            request_id = %request.id,
            timeout_secs,
            "running asset_intel http_json request"
        );
        let mut builder = client
            .request(method, &url)
            .timeout(Duration::from_secs(timeout_secs));
        for (name, value) in &request.headers {
            builder = builder.header(
                name,
                render_http_template(value, company_name, config, &secrets),
            );
        }
        if !request.form.is_empty() {
            let form: HashMap<String, String> = request
                .form
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        render_http_template(value, company_name, config, &secrets),
                    )
                })
                .collect();
            let mut encoded = url::form_urlencoded::Serializer::new(String::new());
            for (key, value) in &form {
                encoded.append_pair(key, value);
            }
            builder = builder
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(encoded.finish());
        } else if !request.json.is_null() {
            builder = builder.json(&render_http_json_value(
                &request.json,
                company_name,
                config,
                &secrets,
            ));
        }

        let response = match builder.send().await {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(
                    provider = %provider_id,
                    run_id,
                    request_id = %request.id,
                    error = %err,
                    "asset_intel http_json request failed"
                );
                let status = AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Failed,
                    message: format!("request '{}' failed: {err}", request.id),
                };
                let count = candidates.organizations.len() + candidates.targets.len();
                return finish_provider_run(
                    sink,
                    run_id,
                    status,
                    count,
                    candidates,
                    serde_json::json!({
                        "provider": provider_id,
                        "runId": run_id,
                        "state": "failed",
                        "reason": "request_failed",
                        "requestId": request.id,
                        "error": err.to_string(),
                        "candidateCount": count,
                    }),
                    profile_entries,
                );
            }
        };
        let http_status = response.status();
        let body = response.text().await.unwrap_or_default();
        let preview: String = body.chars().take(512).collect();
        if !http_status.is_success() {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                request_id = %request.id,
                status = http_status.as_u16(),
                "asset_intel http_json request returned non-success status"
            );
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("request '{}' returned HTTP {http_status}", request.id),
            };
            let count = candidates.organizations.len() + candidates.targets.len();
            return finish_provider_run(
                sink,
                run_id,
                status,
                count,
                candidates,
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "http_status",
                    "requestId": request.id,
                    "status": http_status.as_u16(),
                    "preview": preview,
                    "candidateCount": count,
                }),
                profile_entries,
            );
        }

        if let Some((next, profile)) =
            normalize_json_document(&provider_id, run_id, &asset.normalize, &body)
        {
            profile_entries.extend(profile);
            let added_total = next.organizations.len() + next.targets.len();
            if added_total > 0 {
                let mut delta = OrganizationCandidates::default();
                for item in next.organizations.iter() {
                    delta.organizations.push(item.clone());
                }
                for item in next.targets.iter() {
                    delta.targets.push(item.clone());
                }
                merge_candidates(&mut candidates, next);
                emit_event(
                    sink,
                    AssetIntelStreamEvent::ProviderBatch {
                        run_id: run_id.to_string(),
                        provider_id: provider_id.clone(),
                        candidates: delta,
                        source: AssetIntelBatchSource::Http,
                        artifact: None,
                        request_id: Some(request.id.clone()),
                    },
                );
            }
        }
        request_evidence.push(serde_json::json!({
            "requestId": request.id,
            "status": http_status.as_u16(),
        }));
    }

    if let Some(evidence) =
        enrich_0zone_asns_from_ip_ranges(&provider_id, run_id, &mut profile_entries, sink).await
    {
        request_evidence.push(evidence);
    }

    let total = candidates.organizations.len() + candidates.targets.len();
    let state = if total == 0 {
        AssetIntelProviderRunState::CheckedEmpty
    } else {
        AssetIntelProviderRunState::Completed
    };
    tracing::info!(
        provider = %provider_id,
        run_id,
        candidate_count = total,
        state = ?state,
        "asset_intel http_json provider completed"
    );
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: if total == 0 {
            format!("{provider_id} completed with no candidates")
        } else {
            format!("{provider_id} normalized {total} candidate(s)")
        },
    };
    finish_provider_run(
        sink,
        run_id,
        status,
        total,
        candidates,
        serde_json::json!({
            "provider": provider_id,
            "runId": run_id,
            "state": if total == 0 { "checked_empty" } else { "completed" },
            "candidateCount": total,
            "requests": request_evidence,
        }),
        profile_entries,
    )
}
