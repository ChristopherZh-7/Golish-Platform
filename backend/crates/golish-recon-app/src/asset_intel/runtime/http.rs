//! HTTP-JSON provider runner. Moved out of `mod.rs` verbatim.

use super::super::*;
use crate::organization_recon::artifacts::{
    decode_utf8_clean, write_json_manifest, write_raw_bytes,
};
use crate::organization_recon::{
    ReconArtifactRef, ReconTaskError, ReconTaskManifest, ReconTaskStatus,
};

pub(crate) async fn run_http_json_provider(
    pool: &sqlx::PgPool,
    tool: &ToolConfig,
    project_root: &Path,
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
    let out_dir = asset_intel_provider_output_dir(project_root, run_id, &provider_id);
    fs::create_dir_all(&out_dir)?;
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
    let mut artifacts = Vec::new();
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
                let reason = classify_transport_error(&err);
                let manifest_path = persist_http_artifacts(
                    &out_dir,
                    run_id,
                    &provider_id,
                    ReconTaskStatus::Failed,
                    count + profile_entries.len(),
                    artifacts,
                    vec![ReconTaskError::new(reason, err.to_string())],
                )?;
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
                        "reason": reason,
                        "requestId": request.id,
                        "error": err.to_string(),
                        "candidateCount": count,
                        "manifestPath": manifest_path,
                    }),
                    profile_entries,
                );
            }
        };
        let http_status = response.status();
        let body_bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                let count = candidates.organizations.len() + candidates.targets.len();
                let manifest_path = persist_http_artifacts(
                    &out_dir,
                    run_id,
                    &provider_id,
                    ReconTaskStatus::Failed,
                    count + profile_entries.len(),
                    artifacts,
                    vec![ReconTaskError::new("transport_error", err.to_string())],
                )?;
                let status = AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Failed,
                    message: format!("cannot read response '{}': {err}", request.id),
                };
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
                        "reason": "transport_error",
                        "requestId": request.id,
                        "error": err.to_string(),
                        "candidateCount": count,
                        "manifestPath": manifest_path,
                    }),
                    profile_entries,
                );
            }
        };
        let response_artifact = write_raw_bytes(
            &out_dir,
            format!("raw/response-{}.json", artifact_name(&request.id)),
            &body_bytes,
            "http_response",
        )?;
        let response_path = response_artifact.path.clone();
        artifacts.push(response_artifact);
        let body = match decode_utf8_clean(&body_bytes) {
            Ok(body) => body,
            Err(error) => {
                let count = candidates.organizations.len() + candidates.targets.len();
                let manifest_path = persist_http_artifacts(
                    &out_dir,
                    run_id,
                    &provider_id,
                    ReconTaskStatus::Failed,
                    count + profile_entries.len(),
                    artifacts,
                    vec![error.clone()],
                )?;
                let status = AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Failed,
                    message: format!("response '{}' is not valid UTF-8", request.id),
                };
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
                        "reason": error.code,
                        "requestId": request.id,
                        "candidateCount": count,
                        "manifestPath": manifest_path,
                    }),
                    profile_entries,
                );
            }
        };
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
            let reason = classify_http_status(http_status);
            let manifest_path = persist_http_artifacts(
                &out_dir,
                run_id,
                &provider_id,
                ReconTaskStatus::Failed,
                count + profile_entries.len(),
                artifacts,
                vec![ReconTaskError::new(
                    reason,
                    format!("HTTP {}", http_status.as_u16()),
                )],
            )?;
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
                    "reason": reason,
                    "requestId": request.id,
                    "status": http_status.as_u16(),
                    "preview": preview,
                    "candidateCount": count,
                    "manifestPath": manifest_path,
                }),
                profile_entries,
            );
        }
        if let Some(error) = detect_provider_api_error(&body) {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                request_id = %request.id,
                provider_code = %error.code,
                message = %error.message,
                "asset_intel http_json provider returned application-level error"
            );
            let count = candidates.organizations.len() + candidates.targets.len();
            let reason = classify_provider_api_error(&error);
            let manifest_path = persist_http_artifacts(
                &out_dir,
                run_id,
                &provider_id,
                ReconTaskStatus::Failed,
                count + profile_entries.len(),
                artifacts,
                vec![ReconTaskError::new(
                    reason,
                    format!("provider returned code {}: {}", error.code, error.message),
                )],
            )?;
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("request '{}' provider error: {}", request.id, error.message),
            };
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
                    "reason": reason,
                    "requestId": request.id,
                    "providerCode": error.code,
                    "error": error.message,
                    "candidateCount": count,
                    "manifestPath": manifest_path,
                    "preview": preview,
                }),
                profile_entries,
            );
        }

        let Some((next, profile)) =
            normalize_json_document(&provider_id, run_id, &asset.normalize, &body)
        else {
            let count = candidates.organizations.len() + candidates.targets.len();
            let manifest_path = persist_http_artifacts(
                &out_dir,
                run_id,
                &provider_id,
                ReconTaskStatus::Failed,
                count + profile_entries.len(),
                artifacts,
                vec![ReconTaskError::new(
                    "parse_error",
                    format!("response '{}' is not valid JSON", request.id),
                )],
            )?;
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("response '{}' is not valid JSON", request.id),
            };
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
                    "reason": "parse_error",
                    "requestId": request.id,
                    "candidateCount": count,
                    "manifestPath": manifest_path,
                }),
                profile_entries,
            );
        };
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
                    artifact: Some(response_path.clone()),
                    request_id: Some(request.id.clone()),
                },
            );
        }
        request_evidence.push(serde_json::json!({
            "requestId": request.id,
            "status": http_status.as_u16(),
            "artifact": response_path,
        }));
    }

    if let Some(evidence) =
        enrich_0zone_asns_from_ip_ranges(&provider_id, run_id, &mut profile_entries, sink).await
    {
        request_evidence.push(evidence);
    }

    let candidate_count = candidates.organizations.len() + candidates.targets.len();
    let record_count = candidate_count + profile_entries.len();
    let state = if record_count == 0 {
        AssetIntelProviderRunState::CheckedEmpty
    } else {
        AssetIntelProviderRunState::Completed
    };
    let manifest_path = persist_http_artifacts(
        &out_dir,
        run_id,
        &provider_id,
        if record_count == 0 {
            ReconTaskStatus::CheckedEmpty
        } else {
            ReconTaskStatus::Completed
        },
        record_count,
        artifacts,
        Vec::new(),
    )?;
    tracing::info!(
        provider = %provider_id,
        run_id,
        candidate_count,
        profile_field_count = profile_entries.len(),
        state = ?state,
        "asset_intel http_json provider completed"
    );
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: if record_count == 0 {
            format!("{provider_id} completed with no candidates")
        } else {
            format!("{provider_id} normalized {record_count} record(s)")
        },
    };
    finish_provider_run(
        sink,
        run_id,
        status,
        candidate_count,
        candidates,
        serde_json::json!({
            "provider": provider_id,
            "runId": run_id,
            "state": if record_count == 0 { "checked_empty" } else { "completed" },
            "candidateCount": candidate_count,
            "profileFieldCount": profile_entries.len(),
            "requests": request_evidence,
            "manifestPath": manifest_path,
        }),
        profile_entries,
    )
}

fn artifact_name(request_id: &str) -> String {
    let name: String = request_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        "request".into()
    } else {
        name
    }
}

fn classify_transport_error(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else {
        "transport_error"
    }
}

fn classify_http_status(status: reqwest::StatusCode) -> &'static str {
    match status.as_u16() {
        402 => "quota_exceeded",
        401 | 403 => "unauthorized",
        429 => "rate_limited",
        500..=599 => "server_error",
        _ => "http_status",
    }
}

struct ProviderApiError {
    code: String,
    message: String,
}

fn detect_provider_api_error(body: &str) -> Option<ProviderApiError> {
    let value: Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    let code = object.get("code")?;
    if is_provider_success_code(code) {
        return None;
    }
    let message = object
        .get("message")
        .or_else(|| object.get("msg"))
        .or_else(|| object.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("provider returned application-level error")
        .to_string();
    Some(ProviderApiError {
        code: provider_code_to_string(code),
        message,
    })
}

fn is_provider_success_code(code: &Value) -> bool {
    match code {
        Value::Number(number) => number
            .as_i64()
            .is_some_and(|value| value == 0 || value == 200),
        Value::String(value) => {
            let normalized = value.trim();
            normalized == "0"
                || normalized == "200"
                || normalized.eq_ignore_ascii_case("ok")
                || normalized.eq_ignore_ascii_case("success")
        }
        _ => false,
    }
}

fn provider_code_to_string(code: &Value) -> String {
    match code {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn classify_provider_api_error(error: &ProviderApiError) -> &'static str {
    let haystack = format!("{} {}", error.code, error.message).to_ascii_lowercase();
    if haystack.contains("api key")
        || haystack.contains("apikey")
        || haystack.contains("token")
        || haystack.contains("auth")
        || haystack.contains("unauthor")
        || haystack.contains("不合法")
        || haystack.contains("不存在")
    {
        "unauthorized"
    } else if haystack.contains("quota")
        || haystack.contains("limit")
        || haystack.contains("余额")
        || haystack.contains("次数")
    {
        "quota_exceeded"
    } else if haystack.contains("rate") || haystack.contains("频率") {
        "rate_limited"
    } else {
        "provider_api_error"
    }
}

fn persist_http_artifacts(
    out_dir: &Path,
    run_id: &str,
    provider_id: &str,
    status: ReconTaskStatus,
    record_count: usize,
    artifacts: Vec<ReconArtifactRef>,
    errors: Vec<ReconTaskError>,
) -> Result<PathBuf, GolishError> {
    let mut manifest = ReconTaskManifest::new(run_id, provider_id, "passive_internet", provider_id);
    manifest.status = status;
    manifest.record_count = record_count;
    manifest.checked_empty = matches!(manifest.status, ReconTaskStatus::CheckedEmpty);
    manifest.artifacts = artifacts;
    manifest.errors = errors;
    write_json_manifest(out_dir, &manifest)
}
