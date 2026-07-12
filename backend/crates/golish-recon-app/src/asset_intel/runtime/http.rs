//! HTTP-JSON provider runner. Moved out of `mod.rs` verbatim.

use super::super::*;
use crate::organization_recon::artifacts::{
    decode_utf8_clean, write_json_manifest, write_raw_bytes,
};
use crate::organization_recon::{
    ReconArtifactRef, ReconTaskError, ReconTaskManifest, ReconTaskStatus,
};

/// Default per-request transient-error retries when the descriptor's
/// `max_retries` is unset. Permanent failures (auth/quota/non-2xx/parse) are
/// never retried.
const DEFAULT_HTTP_MAX_RETRIES: u32 = 2;

/// Fixed backoff between transient-error retries of a single request.
const HTTP_RETRY_BACKOFF_MS: u64 = 750;

/// Max times a single request is retried after an HTTP 429 (rate limited),
/// separate from the transient-transport retries above. A 429 is recoverable by
/// waiting, so it must NOT drop the request's data — a dropped ASN/CT/OSINT
/// request left its coverage cell permanently "never attempted" and dead-looped
/// the target_intel gate.
const HTTP_RATE_LIMIT_MAX_RETRIES: u32 = 3;

/// Base for exponential backoff after a 429 when the server sends no usable
/// `Retry-After`: 1s, 2s, 4s for retries 1..=3.
const HTTP_RATE_LIMIT_BASE_BACKOFF_MS: u64 = 1_000;

/// Hard cap on any single rate-limit wait (also clamps a hostile/huge
/// `Retry-After`) so one throttled provider can't hang the whole stage.
const HTTP_RATE_LIMIT_MAX_BACKOFF_MS: u64 = 30_000;

/// Outcome of a single http_json request after retries. `Success` carries the
/// normalized contribution; `Failed` records why and whether the failure is
/// provider-wide (auth/quota) so the caller can stop early without burning the
/// rest of a paid quota.
enum RequestOutcome {
    Success {
        http_status_code: u16,
        candidates: OrganizationCandidates,
        profile: Vec<ProfileFieldEntry>,
    },
    Failed {
        reason: String,
        message: String,
        fatal: bool,
    },
}

/// Mirrors the native-provider query gate: company-name surveys run only
/// requests that do not reference `{{domain}}`; domain-keyed surveys run only
/// requests that do. This prevents domain expansion from re-firing broad org
/// searches and keeps `root_domain=={{domain}}` templates from running empty.
fn request_applies_to_domain_mode(
    request: &golish_pentest::models::AssetIntelHttpRequest,
    domain_mode: bool,
) -> bool {
    let is_domain_request = serde_json::to_string(request)
        .map(|text| text.contains("{{domain}}"))
        .unwrap_or(false);
    if domain_mode {
        is_domain_request
    } else {
        !is_domain_request
    }
}

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
    let golish_pentest::models::AssetIntelRuntimeConfig::HttpJson {
        requests,
        request_delay_ms,
        max_retries,
    } = &asset.runtime
    else {
        return Err(GolishError::Validation(format!(
            "tool '{}' is not an http_json provider",
            tool.id
        )));
    };
    let request_delay = Duration::from_millis(request_delay_ms.unwrap_or(0));
    let max_retries = max_retries.unwrap_or(DEFAULT_HTTP_MAX_RETRIES);

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

    let domain_mode = config
        .domain
        .as_deref()
        .map(str::trim)
        .is_some_and(|domain| !domain.is_empty());
    let applicable_requests = requests
        .iter()
        .filter(|request| request_applies_to_domain_mode(request, domain_mode))
        .collect::<Vec<_>>();
    if applicable_requests.is_empty() {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::CheckedEmpty,
            message: "http_json provider has no requests for this survey mode".into(),
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
                "state": "checked_empty",
                "reason": "no_applicable_requests",
                "domainMode": domain_mode,
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
    let mut request_errors: Vec<ReconTaskError> = Vec::new();
    // Run every non-fatal request and keep whatever succeeds. Partial records
    // remain landable observations, but any failed sibling keeps the provider
    // status Failed/retryable until a later full pass replaces its exact error.
    let mut succeeded_requests = 0usize;
    'requests: for (index, request) in applicable_requests.iter().enumerate() {
        if index > 0 && !request_delay.is_zero() {
            // Rate-limit pacing: keep back-to-back requests under the upstream
            // ceiling (0.zone ≤ 2 req/s) so the server doesn't drop big bodies.
            tokio::time::sleep(request_delay).await;
        }
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderProgress {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                message: format!("requesting '{}' ({})", request.id, request.method),
                stream: AssetIntelStreamSource::System,
            },
        );

        let (artifact, outcome) = run_one_http_request(
            &client,
            request,
            company_name,
            config,
            &secrets,
            &asset.normalize,
            &provider_id,
            run_id,
            &out_dir,
            max_retries,
        )
        .await?;
        let response_path = artifact.as_ref().map(|item| item.path.clone());
        if let Some(item) = artifact {
            artifacts.push(item);
        }

        match outcome {
            RequestOutcome::Success {
                http_status_code,
                candidates: delta,
                profile,
            } => {
                succeeded_requests += 1;
                let profile_count = profile.len();
                profile_entries.extend(profile);
                let added_total = delta.organizations.len() + delta.targets.len();
                if added_total > 0 {
                    let mut emitted = OrganizationCandidates::default();
                    for item in delta.organizations.iter() {
                        emitted.organizations.push(item.clone());
                    }
                    for item in delta.targets.iter() {
                        emitted.targets.push(item.clone());
                    }
                    merge_candidates(&mut candidates, delta);
                    emit_event(
                        sink,
                        AssetIntelStreamEvent::ProviderBatch {
                            run_id: run_id.to_string(),
                            provider_id: provider_id.clone(),
                            candidates: emitted,
                            source: AssetIntelBatchSource::Http,
                            artifact: response_path.clone(),
                            request_id: Some(request.id.clone()),
                        },
                    );
                }
                let normalized_record_count = added_total + profile_count;
                request_evidence.push(serde_json::json!({
                    "requestId": request.id,
                    "status": request_evidence_status(normalized_record_count),
                    "queryType": request.id,
                    "records": normalized_record_count,
                    "httpStatus": http_status_code,
                    "artifact": response_path,
                }));
            }
            RequestOutcome::Failed {
                reason,
                message,
                fatal,
            } => {
                tracing::warn!(
                    provider = %provider_id,
                    run_id,
                    request_id = %request.id,
                    reason = %reason,
                    error = %message,
                    fatal,
                    "asset_intel http_json request failed; keeping data from other requests"
                );
                request_evidence.push(serde_json::json!({
                    "requestId": request.id,
                    "queryType": request.id,
                    "status": "failed",
                    "records": 0,
                    "reason": reason,
                    "error": message,
                }));
                request_errors.push(ReconTaskError::new(reason, message));
                if fatal {
                    // Auth / quota errors are provider-wide: the rest of the
                    // batch would fail identically and burn paid quota.
                    break 'requests;
                }
            }
        }
    }

    if let Some(evidence) =
        enrich_0zone_asns_from_ip_ranges(&provider_id, run_id, &mut profile_entries, sink).await
    {
        request_evidence.push(evidence);
    }

    let candidate_count = candidates.organizations.len() + candidates.targets.len();
    let record_count = candidate_count + profile_entries.len();
    let failed_requests = request_errors.len();
    // Surfaced in the top-level status message when *nothing* succeeded, so an
    // all-failed run (e.g. a bad API key) still explains itself instead of a
    // bare "all N requests errored".
    let first_error_detail = request_errors
        .first()
        .map(|error| format!("{}: {}", error.code, error.message));
    let (state, manifest_status) =
        summarize_http_run(succeeded_requests, failed_requests, record_count);
    let state_label = match state {
        AssetIntelProviderRunState::Completed => "completed",
        AssetIntelProviderRunState::CheckedEmpty => "checked_empty",
        _ => "failed",
    };
    let manifest_path = persist_http_artifacts(
        &out_dir,
        run_id,
        &provider_id,
        manifest_status,
        record_count,
        artifacts,
        request_errors,
    )?;
    tracing::info!(
        provider = %provider_id,
        run_id,
        candidate_count,
        profile_field_count = profile_entries.len(),
        failed_requests,
        state = ?state,
        "asset_intel http_json provider finished"
    );
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: if succeeded_requests == 0 {
            match &first_error_detail {
                Some(detail) => format!("{provider_id} failed: {detail}"),
                None => format!("{provider_id} failed: all {failed_requests} request(s) errored"),
            }
        } else if record_count == 0 {
            format!("{provider_id} completed with no candidates")
        } else if failed_requests > 0 {
            format!(
                "{provider_id} normalized {record_count} record(s); {failed_requests} request(s) failed"
            )
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
            "state": state_label,
            "candidateCount": candidate_count,
            "profileFieldCount": profile_entries.len(),
            "succeededQueries": succeeded_requests,
            "failedQueries": failed_requests,
            "failedRequests": failed_requests,
            "requests": request_evidence,
            "manifestPath": manifest_path,
        }),
        profile_entries,
    )
}

/// Issue one http_json request (with transient-error retries) and turn the
/// response into a [`RequestOutcome`]. The raw response body is always
/// persisted as an artifact when it was read, regardless of how parsing goes,
/// so failures keep their evidence. This never early-returns on a per-request
/// problem — the caller decides how to aggregate.
#[allow(clippy::too_many_arguments)]
async fn run_one_http_request(
    client: &reqwest::Client,
    request: &golish_pentest::models::AssetIntelHttpRequest,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
    secrets: &HashMap<String, String>,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    provider_id: &str,
    run_id: &str,
    out_dir: &Path,
    max_retries: u32,
) -> Result<(Option<ReconArtifactRef>, RequestOutcome), GolishError> {
    let method = match reqwest::Method::from_bytes(request.method.as_bytes()) {
        Ok(method) => method,
        Err(err) => {
            return Ok((
                None,
                RequestOutcome::Failed {
                    reason: "bad_request".into(),
                    message: format!("bad HTTP method '{}': {err}", request.method),
                    fatal: false,
                },
            ));
        }
    };
    let url = render_http_template(&request.url, company_name, config, secrets);
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
            render_http_template(value, company_name, config, secrets),
        );
    }
    if !request.form.is_empty() {
        let form: HashMap<String, String> = request
            .form
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    render_http_template(value, company_name, config, secrets),
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
            secrets,
        ));
    }

    // Send + read the body, retrying transient transport failures (timeout /
    // connection reset / "error decoding response body") and backing off on an
    // HTTP 429 (rate limited) instead of dropping the request.
    let mut attempt: u32 = 0;
    let mut rate_limit_attempt: u32 = 0;
    let (http_status, body_bytes) = loop {
        let Some(attempt_builder) = builder.try_clone() else {
            // In-memory bodies always clone; this only trips for streaming
            // bodies we never build. Degrade to a single best-effort attempt.
            match builder.send().await {
                Ok(response) => {
                    let status = response.status();
                    match response.bytes().await {
                        Ok(bytes) => break (status, bytes),
                        Err(err) => {
                            return Ok((
                                None,
                                RequestOutcome::Failed {
                                    reason: classify_transport_error(&err).into(),
                                    message: err.to_string(),
                                    fatal: false,
                                },
                            ));
                        }
                    }
                }
                Err(err) => {
                    return Ok((
                        None,
                        RequestOutcome::Failed {
                            reason: classify_transport_error(&err).into(),
                            message: err.to_string(),
                            fatal: false,
                        },
                    ));
                }
            }
        };
        match attempt_builder.send().await {
            Ok(response) => {
                let status = response.status();
                // Capture Retry-After before the body read consumes the response.
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                match response.bytes().await {
                    Ok(bytes) => {
                        // Rate limited (429): wait (server Retry-After or
                        // exponential backoff) and retry rather than dropping
                        // this request — a dropped ASN/CT/OSINT request leaves
                        // its coverage cell "never attempted" and dead-loops the
                        // target_intel gate. Bounded by HTTP_RATE_LIMIT_MAX_RETRIES
                        // so a stuck provider can't hang the stage.
                        if status.as_u16() == 429
                            && rate_limit_attempt < HTTP_RATE_LIMIT_MAX_RETRIES
                        {
                            rate_limit_attempt += 1;
                            let backoff_ms =
                                rate_limit_backoff_ms(retry_after.as_deref(), rate_limit_attempt);
                            tracing::warn!(
                                provider = %provider_id,
                                run_id,
                                request_id = %request.id,
                                rate_limit_attempt,
                                backoff_ms,
                                "asset_intel http_json request rate limited (429); backing off before retry"
                            );
                            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                            continue;
                        }
                        break (status, bytes);
                    }
                    Err(err) => {
                        if is_retryable_transport(&err) && attempt < max_retries {
                            attempt += 1;
                            tracing::warn!(
                                provider = %provider_id,
                                run_id,
                                request_id = %request.id,
                                attempt,
                                error = %err,
                                "retrying asset_intel http_json request after body read error"
                            );
                            tokio::time::sleep(Duration::from_millis(HTTP_RETRY_BACKOFF_MS)).await;
                            continue;
                        }
                        return Ok((
                            None,
                            RequestOutcome::Failed {
                                reason: classify_transport_error(&err).into(),
                                message: err.to_string(),
                                fatal: false,
                            },
                        ));
                    }
                }
            }
            Err(err) => {
                if is_retryable_transport(&err) && attempt < max_retries {
                    attempt += 1;
                    tracing::warn!(
                        provider = %provider_id,
                        run_id,
                        request_id = %request.id,
                        attempt,
                        error = %err,
                        "retrying asset_intel http_json request after send error"
                    );
                    tokio::time::sleep(Duration::from_millis(HTTP_RETRY_BACKOFF_MS)).await;
                    continue;
                }
                return Ok((
                    None,
                    RequestOutcome::Failed {
                        reason: classify_transport_error(&err).into(),
                        message: err.to_string(),
                        fatal: false,
                    },
                ));
            }
        }
    };

    // Persist the raw body as evidence before parsing — even failures keep it.
    let response_artifact = write_raw_bytes(
        out_dir,
        format!("raw/response-{}.json", artifact_name(&request.id)),
        &body_bytes,
        "http_response",
    )?;

    let body = match decode_utf8_clean(&body_bytes) {
        Ok(body) => body,
        Err(error) => {
            return Ok((
                Some(response_artifact),
                RequestOutcome::Failed {
                    reason: error.code,
                    message: error.message,
                    fatal: false,
                },
            ));
        }
    };
    if !http_status.is_success() {
        let reason = classify_http_status(http_status);
        let preview: String = body.chars().take(512).collect();
        return Ok((
            Some(response_artifact),
            RequestOutcome::Failed {
                reason: reason.into(),
                message: format!("HTTP {} · {preview}", http_status.as_u16()),
                fatal: is_provider_fatal(reason),
            },
        ));
    }
    if let Some(error) = detect_provider_api_error(&body) {
        let reason = classify_provider_api_error(&error);
        return Ok((
            Some(response_artifact),
            RequestOutcome::Failed {
                reason: reason.into(),
                message: format!("provider returned code {}: {}", error.code, error.message),
                fatal: is_provider_fatal(reason),
            },
        ));
    }
    let Some((next, profile)) = normalize_json_document(provider_id, run_id, normalize, &body)
    else {
        return Ok((
            Some(response_artifact),
            RequestOutcome::Failed {
                reason: "parse_error".into(),
                message: format!("response '{}' is not valid JSON", request.id),
                fatal: false,
            },
        ));
    };
    Ok((
        Some(response_artifact),
        RequestOutcome::Success {
            http_status_code: http_status.as_u16(),
            candidates: next,
            profile,
        },
    ))
}

/// Whether a reqwest error is a *transient* transport failure worth retrying.
/// Notably covers "error decoding response body" (`is_body`/`is_decode`) — the
/// failure mode that silently zeroed 0.zone enrichment.
fn is_retryable_transport(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request() || err.is_body() || err.is_decode()
}

/// Parse a `Retry-After` header (RFC 7231 delta-seconds form only; the HTTP-date
/// form is unsupported → `None` so the caller uses exponential backoff) into a
/// millisecond delay clamped to [`HTTP_RATE_LIMIT_MAX_BACKOFF_MS`].
fn parse_retry_after_ms(value: Option<&str>) -> Option<u64> {
    let secs: u64 = value?.trim().parse().ok()?;
    Some(
        secs.saturating_mul(1_000)
            .min(HTTP_RATE_LIMIT_MAX_BACKOFF_MS),
    )
}

/// Backoff before retrying an HTTP 429: prefer the server's `Retry-After`, else
/// exponential `base * 2^(attempt-1)`. `attempt` is 1-based (1 = first retry).
/// Always clamped to [`HTTP_RATE_LIMIT_MAX_BACKOFF_MS`] and overflow-safe.
fn rate_limit_backoff_ms(retry_after: Option<&str>, attempt: u32) -> u64 {
    if let Some(ms) = parse_retry_after_ms(retry_after) {
        return ms;
    }
    let shift = attempt.saturating_sub(1).min(16);
    HTTP_RATE_LIMIT_BASE_BACKOFF_MS
        .saturating_mul(1u64 << shift)
        .min(HTTP_RATE_LIMIT_MAX_BACKOFF_MS)
}

/// Reasons that are provider-wide (every remaining request would fail the same
/// way), so the runner should stop instead of burning the rest of a paid quota.
fn is_provider_fatal(reason: &str) -> bool {
    matches!(reason, "unauthorized" | "quota_exceeded")
}

/// Map a finished request loop into the provider's terminal state.
///
/// A provider is terminal only when every applicable request completed. Partial
/// successes keep their normalized records but remain `Failed`/retryable.
fn summarize_http_run(
    succeeded_requests: usize,
    failed_requests: usize,
    record_count: usize,
) -> (AssetIntelProviderRunState, ReconTaskStatus) {
    if succeeded_requests == 0 || failed_requests > 0 {
        (AssetIntelProviderRunState::Failed, ReconTaskStatus::Failed)
    } else if record_count == 0 {
        (
            AssetIntelProviderRunState::CheckedEmpty,
            ReconTaskStatus::CheckedEmpty,
        )
    } else {
        (
            AssetIntelProviderRunState::Completed,
            ReconTaskStatus::Completed,
        )
    }
}

/// Typed per-request evidence state consumed by the exact technique mapper.
/// The legacy `ok` label was outside that mapper's vocabulary and therefore
/// made a successful HTTP response look like an execution error.
fn request_evidence_status(normalized_record_count: usize) -> &'static str {
    if normalized_record_count == 0 {
        "empty"
    } else {
        "found"
    }
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

#[cfg(test)]
mod http_runner_tests {
    use super::*;

    fn request_for_test(query: &str) -> golish_pentest::models::AssetIntelHttpRequest {
        golish_pentest::models::AssetIntelHttpRequest {
            id: "test".into(),
            method: "POST".into(),
            url: "https://example.test/api".into(),
            headers: std::collections::HashMap::new(),
            form: std::collections::HashMap::new(),
            json: serde_json::json!({ "query": query }),
            timeout_secs: 5,
        }
    }

    #[test]
    fn request_applies_gates_http_requests_by_domain_mode() {
        let company_request = request_for_test("{{company_name}}");
        let domain_request = request_for_test("root_domain=={{domain}}");

        assert!(request_applies_to_domain_mode(&company_request, false));
        assert!(!request_applies_to_domain_mode(&company_request, true));
        assert!(request_applies_to_domain_mode(&domain_request, true));
        assert!(!request_applies_to_domain_mode(&domain_request, false));
    }

    #[test]
    fn summarize_failed_when_no_request_succeeded() {
        let (state, manifest) = summarize_http_run(0, 2, 0);
        assert_eq!(state, AssetIntelProviderRunState::Failed);
        assert_eq!(manifest, ReconTaskStatus::Failed);
        // Even if some earlier provider data lingered in the counter, a run
        // where every request errored is still Failed (untrusted downstream).
        let (state, _) = summarize_http_run(0, 2, 9);
        assert_eq!(state, AssetIntelProviderRunState::Failed);
    }

    #[test]
    fn summarize_checked_empty_when_reachable_but_no_records() {
        let (state, manifest) = summarize_http_run(2, 0, 0);
        assert_eq!(state, AssetIntelProviderRunState::CheckedEmpty);
        assert_eq!(manifest, ReconTaskStatus::CheckedEmpty);
    }

    #[test]
    fn summarize_failed_but_landable_when_partial_success_has_records() {
        let (state, manifest) = summarize_http_run(2, 1, 5);
        assert_eq!(state, AssetIntelProviderRunState::Failed);
        assert_eq!(manifest, ReconTaskStatus::Failed);

        let (state, manifest) = summarize_http_run(2, 0, 5);
        assert_eq!(state, AssetIntelProviderRunState::Completed);
        assert_eq!(manifest, ReconTaskStatus::Completed);
    }

    #[test]
    fn successful_request_evidence_uses_found_or_empty_not_ok() {
        assert_eq!(request_evidence_status(0), "empty");
        assert_eq!(request_evidence_status(1), "found");
        assert_eq!(request_evidence_status(17), "found");
    }

    #[test]
    fn provider_fatal_only_for_auth_and_quota() {
        assert!(is_provider_fatal("unauthorized"));
        assert!(is_provider_fatal("quota_exceeded"));
        assert!(!is_provider_fatal("transport_error"));
        assert!(!is_provider_fatal("timeout"));
        assert!(!is_provider_fatal("rate_limited"));
        assert!(!is_provider_fatal("server_error"));
        assert!(!is_provider_fatal("parse_error"));
    }

    #[test]
    fn parse_retry_after_reads_delta_seconds_only() {
        assert_eq!(parse_retry_after_ms(Some("5")), Some(5_000));
        assert_eq!(parse_retry_after_ms(Some("  10 ")), Some(10_000));
        assert_eq!(parse_retry_after_ms(Some("0")), Some(0));
        // HTTP-date form is unsupported → None (caller falls back to backoff).
        assert_eq!(
            parse_retry_after_ms(Some("Wed, 21 Oct 2025 07:28:00 GMT")),
            None
        );
        assert_eq!(parse_retry_after_ms(None), None);
        // A huge Retry-After is clamped to the max wait.
        assert_eq!(
            parse_retry_after_ms(Some("99999")),
            Some(HTTP_RATE_LIMIT_MAX_BACKOFF_MS)
        );
    }

    #[test]
    fn rate_limit_backoff_prefers_retry_after_then_exponential() {
        // Server Retry-After wins when present and parseable.
        assert_eq!(rate_limit_backoff_ms(Some("3"), 1), 3_000);
        // No header → exponential 1s, 2s, 4s for retries 1..=3.
        assert_eq!(rate_limit_backoff_ms(None, 1), 1_000);
        assert_eq!(rate_limit_backoff_ms(None, 2), 2_000);
        assert_eq!(rate_limit_backoff_ms(None, 3), 4_000);
        // Big attempt is clamped and never overflows (shift saturates).
        assert_eq!(
            rate_limit_backoff_ms(None, 99),
            HTTP_RATE_LIMIT_MAX_BACKOFF_MS
        );
    }
}
