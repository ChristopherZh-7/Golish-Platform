//! Agent-bridge wiring: assembles shared services (sidecar, db, graph,
//! memory, sub-agents, pentest/MCP tools) onto a per-session [`AgentBridge`].

use std::collections::BTreeSet;
use std::sync::Arc;

use super::super::agent_bridge::AgentBridge;
use crate::state::AgentState;

/// Configure the agent bridge with shared services from AgentState.
///
/// This also looks up and sets the memory file path for project instructions
/// based on the workspace path and indexed codebases in settings.
///
/// Sub-agent model overrides from settings are applied to the registry.
///
/// IMPORTANT: Each session gets its own SidecarState instance to enable
/// per-session isolation and avoid blocking between tabs when agents run concurrently.
pub async fn configure_bridge(
    bridge: &mut AgentBridge,
    state: &AgentState,
    session_id: &str,
    app_handle: Option<tauri::AppHandle>,
) {
    let is_title_gen = golish_core::is_title_gen_session_id(session_id);

    if is_title_gen {
        configure_title_gen(bridge).await;
    }

    configure_core_services(bridge, state).await;
    configure_domain_hooks(bridge, state);

    let settings = state.settings_manager.get().await;
    configure_memory_and_embeddings(bridge, state, &settings).await;
    configure_sub_agents(bridge, &settings).await;

    if !is_title_gen {
        setup_bridge_mcp_tools(bridge, state).await;
        register_pentest_tools(bridge, state, app_handle).await;
        register_visible_pty_tool(bridge, state).await;
        // P3-c: the listener also books successful background jobs into the
        // evidence ledger, so it needs a repo handle + the project-path scope.
        let bg_repo: std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider> =
            std::sync::Arc::new(crate::ai::db_bridge::GolishDbRepoProvider::new(
                state.db_pool.clone(),
            ));
        let pp = bridge
            .workspace()
            .read()
            .await
            .to_string_lossy()
            .to_string();
        let bg_project_path = if pp == "." || pp.is_empty() {
            None
        } else {
            Some(pp)
        };
        spawn_background_output_listener(bridge);
        spawn_background_completion_listener(
            bridge,
            bg_repo,
            state.db_pool.clone(),
            bg_project_path,
        );
    }
}

/// Wire this session's background-job stdout/stderr chunks into the normal tool
/// output event stream. The frontend already knows how to append
/// `ToolOutputChunk` to shell-like tool panels, so attributed `pentest_run`
/// background jobs become visible without a separate UI path.
fn spawn_background_output_listener(bridge: &AgentBridge) {
    use golish_core::events::AiEvent;

    let Some(session_id) = bridge.event_session_id().map(str::to_string) else {
        tracing::debug!("[configure_bridge] No session id; skipping background output listener");
        return;
    };
    let event_tx = bridge.get_or_create_event_tx();
    let mut rx = golish_app_core::background_jobs::manager().subscribe_output_chunks();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(chunk) => {
                    if chunk.session_id.as_deref() != Some(session_id.as_str()) {
                        continue;
                    }
                    let _ = event_tx.send(AiEvent::ToolOutputChunk {
                        request_id: chunk.request_id,
                        tool_name: chunk.tool_name,
                        chunk: chunk.chunk,
                        stream: chunk.stream.to_string(),
                        source: chunk.source,
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                    tracing::warn!(
                        "[background-output-listener] lagged; dropped {} chunk(s)",
                        dropped
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    tracing::info!("[configure_bridge] Started background-job output listener");
}

/// Wire this session's background-job completions back into the agent.
///
/// Long shell/pentest commands that exceed their soft timeout keep running in
/// the background (see `golish-app-core/background_jobs.rs`). When one finishes,
/// the job manager broadcasts a [`JobCompletion`]; this per-session listener
/// (started once per `configure_bridge`) picks up the ones attributed to this
/// session and:
/// 1. emits a `ToolBackgroundCompleted` event so the frontend can surface it,
/// 2. books a successful job's output into the evidence ledger (P3-c), and
/// 3. queues a note (with the evidence id) so the agent learns the outcome — and
///    can cite a REAL evidence id — on its next turn.
///
/// [`JobCompletion`]: golish_app_core::background_jobs::JobCompletion
fn spawn_background_completion_listener(
    bridge: &AgentBridge,
    db_repo: std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider>,
    db_pool: std::sync::Arc<sqlx::PgPool>,
    project_path: Option<String>,
) {
    use golish_core::events::AiEvent;

    let Some(session_id) = bridge.event_session_id().map(str::to_string) else {
        tracing::debug!(
            "[configure_bridge] No session id; skipping background completion listener"
        );
        return;
    };
    let notes = bridge.background_notes_handle();
    let event_tx = bridge.get_or_create_event_tx();
    let mut rx = golish_app_core::background_jobs::manager().subscribe_completions();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(jc) => {
                    // Only handle jobs this session started.
                    if jc.session_id.as_deref() != Some(session_id.as_str()) {
                        continue;
                    }

                    tracing::info!(
                        message = "[background-listener] job finished",
                        session_id = %session_id,
                        job_id = %jc.job_id,
                        status = %jc.status.as_str(),
                    );

                    let _ = event_tx.send(AiEvent::ToolBackgroundCompleted {
                        job_id: jc.job_id.clone(),
                        command: jc.command.clone(),
                        status: jc.status.as_str().to_string(),
                        exit_code: jc.exit_code,
                        stdout_tail: jc.stdout_tail.clone(),
                        stderr_tail: jc.stderr_tail.clone(),
                        duration_ms: jc.duration_ms,
                    });

                    // P3-c: book the (successful) job into the evidence ledger so
                    // the agent can cite a real id next turn.
                    let evidence_id = maybe_append_background_evidence(
                        &db_repo,
                        &session_id,
                        project_path.as_deref(),
                        &jc,
                    )
                    .await;
                    maybe_store_background_structured_output(
                        &db_pool,
                        project_path.as_deref(),
                        &jc,
                    )
                    .await;
                    maybe_store_background_batch_liveness_outcomes(
                        &db_pool,
                        &session_id,
                        project_path.as_deref(),
                        &jc,
                        evidence_id,
                    )
                    .await;
                    maybe_store_background_batch_port_outcomes(
                        &db_pool,
                        &session_id,
                        project_path.as_deref(),
                        &jc,
                        evidence_id,
                    )
                    .await;
                    maybe_store_background_batch_service_outcomes(
                        &db_pool,
                        &session_id,
                        project_path.as_deref(),
                        &jc,
                        evidence_id,
                    )
                    .await;

                    // Observability (design 2026-06-05): surface background-job
                    // evidence as a first-class HarnessTrace so it appears in the
                    // timeline. This path was the worst blind spot — backgrounded
                    // scans' real ids previously only reached the agent via a
                    // next-turn prompt note, invisible to any run reconstruction.
                    if let Some(eid) = evidence_id {
                        let evidence_kind = background_evidence_kind(&jc.command);
                        let _ = event_tx.send(AiEvent::HarnessTrace {
                            operation_id: session_id.clone(),
                            stage: String::new(),
                            agent_path: "main".to_string(),
                            trace: golish_core::events::HarnessTraceKind::EvidenceBooked {
                                tool: evidence_kind.to_string(),
                                evidence_id: eid,
                                source: "background".to_string(),
                            },
                        });
                    }

                    let note = format_background_note(&jc, evidence_id);
                    match notes.lock() {
                        Ok(mut q) => q.push(note),
                        // Recover the Vec without dropping the note on poison.
                        Err(poisoned) => poisoned.into_inner().push(note),
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                    tracing::warn!(
                        "[background-listener] lagged; dropped {} completion(s)",
                        dropped
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    tracing::info!("[configure_bridge] Started background-job completion listener");
}

async fn maybe_store_background_structured_output(
    db_pool: &sqlx::PgPool,
    project_path: Option<&str>,
    jc: &golish_app_core::background_jobs::JobCompletion,
) {
    use golish_app_core::background_jobs::JobStatus;

    if jc.status != JobStatus::Done {
        return;
    }
    let retained_stdout = golish_app_core::background_jobs::manager()
        .snapshot(&jc.job_id)
        .map(|snapshot| snapshot.stdout)
        .filter(|stdout| !stdout.trim().is_empty());
    let stdout = retained_stdout.unwrap_or_else(|| jc.stdout_tail.clone());
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return;
    }

    let store = golish_pentest::output_store::PgPentestStore::new(db_pool);
    match golish_pentest::output_store::maybe_detect_and_store_via_context(
        &store,
        &jc.command,
        stdout,
        project_path,
        golish_pentest::output_store::StoreContext {
            organization_id: jc.organization_id,
        },
    )
    .await
    {
        Some(stats) => {
            tracing::info!(
                target: "harness::evidence",
                job_id = %jc.job_id,
                org_id = ?jc.organization_id,
                tool = %stats.tool_name,
                parsed = stats.parsed_count,
                stored = stats.stored_count,
                errors = ?stats.errors,
                "background job structured output stored"
            );
        }
        None => {
            tracing::debug!(
                target: "harness::evidence",
                job_id = %jc.job_id,
                command = %jc.command,
                "background job structured output not detected"
            );
        }
    }
}

async fn maybe_store_background_batch_liveness_outcomes(
    db_pool: &sqlx::PgPool,
    session_id: &str,
    project_path: Option<&str>,
    jc: &golish_app_core::background_jobs::JobCompletion,
    evidence_id: Option<i64>,
) {
    use golish_agent_kit::harness::evidence_facts::TECH_EAS_LIVENESS;
    use golish_app_core::background_jobs::JobStatus;

    if jc.status != JobStatus::Done {
        return;
    }
    let Some(organization_id) = jc.organization_id else {
        return;
    };
    let Some(evidence_id) = evidence_id else {
        return;
    };
    let Some(tool) = background_command_tool_name(&jc.command) else {
        return;
    };
    if !is_batch_liveness_command(tool.as_str(), &jc.command) {
        return;
    }
    let Some(input) = batch_input_text_from_command(
        &jc.command,
        project_path,
        tool.as_str(),
        &jc.job_id,
        "liveness",
    )
    .await
    else {
        return;
    };
    let targets = batch_input_targets(&input);
    if targets.is_empty() {
        return;
    }
    let Some(allowed_assets) =
        scoped_eas_outcome_asset_keys(db_pool, organization_id, EasOutcomeKeyMode::Liveness).await
    else {
        return;
    };
    if allowed_assets.is_empty() {
        tracing::info!(
            target: "harness::evidence",
            job_id = %jc.job_id,
            org_id = %organization_id,
            "background batch liveness outcomes skipped: org has no in-scope EAS assets"
        );
        return;
    }
    let retained_stdout = golish_app_core::background_jobs::manager()
        .snapshot(&jc.job_id)
        .map(|snapshot| snapshot.stdout)
        .unwrap_or_else(|| jc.stdout_tail.clone());
    let source = background_evidence_tool_name(&jc.command);

    let mut stored = 0usize;
    let mut skipped = 0usize;
    for target in targets {
        if target.contains('/') && !target.starts_with("http://") && !target.starts_with("https://")
        {
            skipped += 1;
            continue;
        }
        let Some(asset) = eas_outcome_asset_key_for_mode(&target, EasOutcomeKeyMode::Liveness)
        else {
            skipped += 1;
            continue;
        };
        if !allowed_assets.contains(&asset) {
            skipped += 1;
            continue;
        }
        let found = batch_output_mentions_target(&retained_stdout, &asset);
        let write = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
            organization_id,
            run_id: session_id.to_string(),
            asset,
            technique: TECH_EAS_LIVENESS.to_string(),
            outcome: if found { "found" } else { "empty" }.to_string(),
            source: Some(source.clone()),
            query: Some(jc.command.clone()),
            result_count: Some(if found { 1 } else { 0 }),
            confidence: None,
            evidence_ids: vec![evidence_id],
            collected_at: Some(chrono::Utc::now()),
        };
        match golish_db::repo::technique_outcomes::upsert(db_pool, &write).await {
            Ok(()) => stored += 1,
            Err(err) => {
                tracing::warn!(
                    target: "harness::evidence",
                    job_id = %jc.job_id,
                    error = %err,
                    "background batch liveness outcome upsert failed"
                );
            }
        }
    }

    if stored > 0 || skipped > 0 {
        tracing::info!(
            target: "harness::evidence",
            job_id = %jc.job_id,
            org_id = %organization_id,
            source = %source,
            stored,
            skipped,
            "background batch liveness outcomes stored"
        );
    }
}

async fn maybe_store_background_batch_port_outcomes(
    db_pool: &sqlx::PgPool,
    session_id: &str,
    project_path: Option<&str>,
    jc: &golish_app_core::background_jobs::JobCompletion,
    evidence_id: Option<i64>,
) {
    use golish_agent_kit::harness::evidence_facts::TECH_EAS_PORT;
    use golish_app_core::background_jobs::JobStatus;

    if jc.status != JobStatus::Done {
        return;
    }
    let Some(organization_id) = jc.organization_id else {
        return;
    };
    let Some(evidence_id) = evidence_id else {
        return;
    };
    let Some(tool) = background_command_tool_name(&jc.command) else {
        return;
    };
    if !matches!(tool.as_str(), "naabu" | "masscan") {
        return;
    }
    let Some(input) =
        batch_input_text_from_command(&jc.command, project_path, tool.as_str(), &jc.job_id, "port")
            .await
    else {
        return;
    };
    let targets = batch_input_targets(&input);
    if targets.is_empty() {
        return;
    }
    let Some(allowed_assets) =
        scoped_eas_outcome_asset_keys(db_pool, organization_id, EasOutcomeKeyMode::Host).await
    else {
        return;
    };
    if allowed_assets.is_empty() {
        tracing::info!(
            target: "harness::evidence",
            job_id = %jc.job_id,
            org_id = %organization_id,
            "background batch port outcomes skipped: org has no in-scope EAS assets"
        );
        return;
    }
    let retained_stdout = golish_app_core::background_jobs::manager()
        .snapshot(&jc.job_id)
        .map(|snapshot| snapshot.stdout)
        .unwrap_or_else(|| jc.stdout_tail.clone());
    let open_counts = open_port_counts(tool.as_str(), &retained_stdout);
    let source = background_evidence_tool_name(&jc.command);

    let mut stored = 0usize;
    let mut skipped = 0usize;
    for target in targets {
        if target.contains('/') && !target.starts_with("http://") && !target.starts_with("https://")
        {
            skipped += 1;
            continue;
        }
        let Some(asset) = golish_pentest_domain::canonical_asset_key(&target).map(|key| key.key)
        else {
            skipped += 1;
            continue;
        };
        if !allowed_assets.contains(&asset) {
            skipped += 1;
            continue;
        }
        let count = open_counts.get(&asset).copied().unwrap_or(0);
        let outcome = if count > 0 { "found" } else { "empty" };
        let write = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
            organization_id,
            run_id: session_id.to_string(),
            asset,
            technique: TECH_EAS_PORT.to_string(),
            outcome: outcome.to_string(),
            source: Some(source.clone()),
            query: Some(jc.command.clone()),
            result_count: Some(count.min(i32::MAX as usize) as i32),
            confidence: None,
            evidence_ids: vec![evidence_id],
            collected_at: Some(chrono::Utc::now()),
        };
        match golish_db::repo::technique_outcomes::upsert(db_pool, &write).await {
            Ok(()) => stored += 1,
            Err(err) => {
                tracing::warn!(
                    target: "harness::evidence",
                    job_id = %jc.job_id,
                    error = %err,
                    "background batch port outcome upsert failed"
                );
            }
        }
    }

    if stored > 0 || skipped > 0 {
        tracing::info!(
            target: "harness::evidence",
            job_id = %jc.job_id,
            org_id = %organization_id,
            source = %source,
            stored,
            skipped,
            "background batch port outcomes stored"
        );
    }
}

async fn maybe_store_background_batch_service_outcomes(
    db_pool: &sqlx::PgPool,
    session_id: &str,
    project_path: Option<&str>,
    jc: &golish_app_core::background_jobs::JobCompletion,
    evidence_id: Option<i64>,
) {
    use golish_agent_kit::harness::evidence_facts::TECH_EAS_SERVICE_FINGERPRINT;
    use golish_app_core::background_jobs::JobStatus;

    if jc.status != JobStatus::Done {
        return;
    }
    let Some(organization_id) = jc.organization_id else {
        return;
    };
    let Some(evidence_id) = evidence_id else {
        return;
    };
    let Some(tool) = background_command_tool_name(&jc.command) else {
        return;
    };
    if !is_batch_service_command(tool.as_str(), &jc.command) {
        return;
    }
    let Some(input) = batch_input_text_from_command(
        &jc.command,
        project_path,
        tool.as_str(),
        &jc.job_id,
        "service",
    )
    .await
    else {
        return;
    };
    let targets = batch_input_targets(&input);
    if targets.is_empty() {
        return;
    }
    let Some(allowed_assets) =
        scoped_eas_outcome_asset_keys(db_pool, organization_id, EasOutcomeKeyMode::Host).await
    else {
        return;
    };
    if allowed_assets.is_empty() {
        tracing::info!(
            target: "harness::evidence",
            job_id = %jc.job_id,
            org_id = %organization_id,
            "background batch service outcomes skipped: org has no in-scope EAS assets"
        );
        return;
    }
    let retained_stdout = golish_app_core::background_jobs::manager()
        .snapshot(&jc.job_id)
        .map(|snapshot| snapshot.stdout)
        .unwrap_or_else(|| jc.stdout_tail.clone());
    let source = background_evidence_tool_name(&jc.command);

    let mut stored = 0usize;
    let mut skipped = 0usize;
    for target in targets {
        if target.contains('/') && !target.starts_with("http://") && !target.starts_with("https://")
        {
            skipped += 1;
            continue;
        }
        let Some(asset) = golish_pentest_domain::canonical_asset_key(&target).map(|key| key.key)
        else {
            skipped += 1;
            continue;
        };
        if !allowed_assets.contains(&asset) {
            skipped += 1;
            continue;
        }
        let found = service_output_mentions_target(&retained_stdout, &asset);
        let write = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
            organization_id,
            run_id: session_id.to_string(),
            asset,
            technique: TECH_EAS_SERVICE_FINGERPRINT.to_string(),
            outcome: if found { "found" } else { "empty" }.to_string(),
            source: Some(source.clone()),
            query: Some(jc.command.clone()),
            result_count: Some(if found { 1 } else { 0 }),
            confidence: None,
            evidence_ids: vec![evidence_id],
            collected_at: Some(chrono::Utc::now()),
        };
        match golish_db::repo::technique_outcomes::upsert(db_pool, &write).await {
            Ok(()) => stored += 1,
            Err(err) => {
                tracing::warn!(
                    target: "harness::evidence",
                    job_id = %jc.job_id,
                    error = %err,
                    "background batch service outcome upsert failed"
                );
            }
        }
    }

    if stored > 0 || skipped > 0 {
        tracing::info!(
            target: "harness::evidence",
            job_id = %jc.job_id,
            org_id = %organization_id,
            source = %source,
            stored,
            skipped,
            "background batch service outcomes stored"
        );
    }
}

fn batch_input_targets(input: &str) -> Vec<String> {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn is_batch_liveness_command(tool: &str, command: &str) -> bool {
    match tool {
        "httpx" => {
            command_has_any_flag(command, &["-l", "-list", "--list", "-input"])
                || heredoc_body_from_command(command).is_some()
        }
        "nmap" => {
            command_has_any_flag(command, &["-iL"])
                && command_has_any_flag(command, &["-sn", "-sP"])
        }
        _ => false,
    }
}

fn is_batch_service_command(tool: &str, command: &str) -> bool {
    match tool {
        "whatweb" => true,
        "nmap" => {
            command_has_any_flag(command, &["-iL"])
                && !command_has_any_flag(command, &["-sn", "-sP"])
                && command_has_any_flag(command, &["-sV", "-A", "--version-all", "--version-light"])
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EasOutcomeKeyMode {
    /// Endpoint liveness keeps URL port/path in the coverage key.
    Liveness,
    /// Port/service coverage is host-level.
    Host,
}

async fn scoped_eas_outcome_asset_keys(
    db_pool: &sqlx::PgPool,
    organization_id: uuid::Uuid,
    mode: EasOutcomeKeyMode,
) -> Option<BTreeSet<String>> {
    let rows = match sqlx::query_as::<_, (String, String)>(
        "SELECT value, real_ip FROM targets WHERE scope::text = 'in' AND organization_id = $1",
    )
    .bind(organization_id)
    .fetch_all(db_pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(
                target: "harness::evidence",
                org_id = %organization_id,
                error = %err,
                "failed to load org in-scope assets for EAS outcome scoping"
            );
            return None;
        }
    };
    Some(eas_outcome_asset_keys_from_rows(rows, mode))
}

fn eas_outcome_asset_keys_from_rows(
    rows: Vec<(String, String)>,
    mode: EasOutcomeKeyMode,
) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for (value, real_ip) in rows {
        if let Some(key) = eas_outcome_asset_key_for_mode(&value, mode) {
            keys.insert(key);
        }
        for ip in real_ip
            .split([',', ';', '\n', '\r', '\t', ' '])
            .map(str::trim)
            .filter(|ip| !ip.is_empty())
        {
            if let Some(key) = eas_outcome_asset_key_for_mode(ip, mode) {
                keys.insert(key);
            }
        }
    }
    keys
}

fn eas_outcome_asset_key_for_mode(value: &str, mode: EasOutcomeKeyMode) -> Option<String> {
    match mode {
        EasOutcomeKeyMode::Liveness => {
            golish_agent_kit::harness::evidence_facts::eas_liveness_asset_key(value).or_else(|| {
                golish_pentest_domain::canonical_asset_key(value).and_then(|key| {
                    matches!(key.class, golish_pentest_domain::AssetClass::Ip).then_some(key.key)
                })
            })
        }
        EasOutcomeKeyMode::Host => {
            golish_pentest_domain::canonical_asset_key(value).map(|key| key.key)
        }
    }
}

async fn batch_input_text_from_command(
    command: &str,
    project_path: Option<&str>,
    tool: &str,
    job_id: &str,
    kind: &str,
) -> Option<String> {
    if let Some(input) = heredoc_body_from_command(command) {
        return Some(input);
    }

    let input_file = batch_input_file_from_command(command, project_path, tool)?;
    match tokio::fs::read_to_string(&input_file).await {
        Ok(input) => Some(input),
        Err(err) => {
            tracing::debug!(
                target: "harness::evidence",
                job_id = %job_id,
                input_file = %input_file.display(),
                kind,
                error = %err,
                "background batch outcome input file unavailable"
            );
            None
        }
    }
}

fn command_has_any_flag(command: &str, flags: &[&str]) -> bool {
    command_tokens(command).iter().any(|token| {
        flags.iter().any(|flag| {
            token.eq_ignore_ascii_case(flag)
                || token
                    .strip_prefix(flag)
                    .map(|rest| rest.starts_with('='))
                    .unwrap_or(false)
        })
    })
}

fn open_port_counts(tool: &str, stdout: &str) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let host = match tool {
            "naabu" => host_from_naabu_line(line),
            "masscan" => host_from_masscan_line(line),
            _ => None,
        };
        let Some(host) = host else {
            continue;
        };
        if let Some(asset) = golish_pentest_domain::canonical_asset_key(host).map(|key| key.key) {
            *counts.entry(asset).or_insert(0) += 1;
        }
    }
    counts
}

fn host_from_naabu_line(line: &str) -> Option<&str> {
    let (host, port) = line.rsplit_once(':')?;
    port.chars().all(|c| c.is_ascii_digit()).then_some(host)
}

fn host_from_masscan_line(line: &str) -> Option<&str> {
    line.strip_prefix("Discovered open port ")
        .and_then(|rest| rest.split_once(" on "))
        .map(|(_, host)| host.trim())
        .filter(|host| !host.is_empty())
}

fn batch_input_file_from_command(
    command: &str,
    project_path: Option<&str>,
    tool: &str,
) -> Option<std::path::PathBuf> {
    let flags: &[&str] = match tool {
        "naabu" => &["-list"],
        "httpx" => &["-l", "-list", "--list", "-input"],
        "masscan" | "nmap" => &["-iL"],
        "whatweb" => &["--input-file"],
        _ => return None,
    };
    let tokens = command_tokens(command);
    let raw = tokens
        .windows(2)
        .find_map(|pair| {
            flags
                .contains(&pair[0].as_str())
                .then_some(pair[1].as_str())
        })
        .or_else(|| {
            tokens.iter().find_map(|token| {
                flags
                    .iter()
                    .find_map(|flag| token.strip_prefix(&format!("{flag}=")))
            })
        })?;
    let path = std::path::PathBuf::from(clean_command_path_arg(raw));
    if path.is_absolute() {
        Some(path)
    } else {
        project_path.map(|base| std::path::Path::new(base).join(path))
    }
}

fn clean_command_path_arg(raw: &str) -> String {
    raw.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn heredoc_body_from_command(command: &str) -> Option<String> {
    let heredoc_at = command.find("<<")?;
    let mut rest = &command[heredoc_at + 2..];
    if let Some(after_dash) = rest.strip_prefix('-') {
        rest = after_dash;
    }
    rest = rest.trim_start();

    let (delimiter, after_delimiter) =
        if let Some(quote) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') {
            let quoted = &rest[quote.len_utf8()..];
            let end = quoted.find(quote)?;
            (&quoted[..end], &quoted[end + quote.len_utf8()..])
        } else {
            let end = rest.find(char::is_whitespace)?;
            (&rest[..end], &rest[end..])
        };
    if delimiter.is_empty() {
        return None;
    }

    let body_start = after_delimiter.find('\n')?;
    let body = &after_delimiter[body_start + 1..];
    let mut lines = Vec::new();
    for line in body.lines() {
        let normalized = line.trim_end_matches('\r');
        if normalized == delimiter {
            return Some(lines.join("\n"));
        }
        lines.push(line);
    }
    None
}

fn service_output_mentions_target(stdout: &str, asset: &str) -> bool {
    batch_output_mentions_target(stdout, asset)
}

fn batch_output_mentions_target(stdout: &str, asset: &str) -> bool {
    let needle = asset.to_ascii_lowercase();
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .any(|line| contains_asset_token(&line.to_ascii_lowercase(), &needle))
}

fn contains_asset_token(line: &str, needle: &str) -> bool {
    let mut search_from = 0usize;
    while let Some(relative) = line[search_from..].find(needle) {
        let start = search_from + relative;
        let end = start + needle.len();
        let before_ok = start == 0 || !is_asset_char(line.as_bytes()[start - 1]);
        let after_ok = end == line.len() || !is_asset_char(line.as_bytes()[end]);
        if before_ok && after_ok {
            return true;
        }
        search_from = start + 1;
    }
    false
}

fn is_asset_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
}

/// Append a **successful** background job's output to the evidence ledger (P3-c),
/// gated on harness stage mode. Returns the new evidence id so it can be surfaced
/// to the agent (letting it cite a REAL id in a StageDeliverable's `evidence_refs`
/// instead of fabricating one, which the gate would then BLOCK).
///
/// Background jobs terminate **outside** the agentic loop, so the synchronous
/// evidence path in `golish-agent-runtime` never books them — a backgrounded scan
/// would otherwise be lost (AGENTS.md I7/I8: a scan that actually ran is
/// "checked", not "unchecked"). The operation grouping key is a deterministic
/// per-session uuid: the live tracker's random `session_uuid` isn't reachable
/// from this detached listener, so background evidence forms its own stable
/// per-session hash chain. That is fine for the gate's fabricated-ref check,
/// which verifies an id **exists**, not its chain membership.
async fn maybe_append_background_evidence(
    db_repo: &std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider>,
    session_id: &str,
    project_path: Option<&str>,
    jc: &golish_app_core::background_jobs::JobCompletion,
) -> Option<i64> {
    use golish_app_core::background_jobs::JobStatus;

    let op_id = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("golish-bg-op:{session_id}").as_bytes(),
    );
    let raw_output = if jc.stderr_tail.trim().is_empty() {
        jc.stdout_tail.clone()
    } else {
        format!("{}\n[stderr]\n{}", jc.stdout_tail, jc.stderr_tail)
    };

    // PR2 (coverage 投影) · backgrounded scans are the main target_intel/EAS
    // evidence path. Derive (technique, asset, outcome) from the job command;
    // EAS needs stderr too because tools such as nmap report DNS failures there.
    let distinguish_failure =
        golish_agent_kit::harness::feature_flags::failure_outcome_error_enabled();
    let facts = golish_agent_kit::harness::evidence_facts::coverage_facts_from_command(&jc.command)
        .map(|(technique, asset)| {
            let outcome = golish_agent_kit::harness::evidence_facts::coverage_outcome_for_run(
                technique,
                &raw_output,
                jc.status == JobStatus::Done,
                distinguish_failure,
            );
            (technique, asset, outcome)
        });

    // Cleanly-finished jobs are always citable evidence. Failed mapped probes
    // are also evidence because they close a coverage cell as error/empty. Killed
    // jobs remain just a user-visible note: an aborted scan is not a completed
    // check.
    if jc.status != JobStatus::Done && !(jc.status == JobStatus::Failed && facts.is_some()) {
        return None;
    }
    let evidence_tool = background_evidence_tool_name(&jc.command);
    let evidence_kind = background_evidence_kind(&jc.command);

    match db_repo
        .evidence_append(
            op_id,
            None,
            Some(session_id),
            project_path,
            &evidence_tool,
            evidence_kind,
            &jc.command,
            &raw_output,
            facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
        )
        .await
    {
        Ok(id) => {
            tracing::info!(
                target: "harness::evidence",
                job_id = %jc.job_id,
                evidence_id = id,
                "background job evidence appended"
            );
            Some(id)
        }
        Err(e) => {
            tracing::warn!(
                target: "harness::evidence",
                error = %e,
                "background job evidence append failed (continuing)"
            );
            None
        }
    }
}

fn background_command_tool_name(command: &str) -> Option<String> {
    let (first, rest) = command_token(command)?;
    let base = command_token_base(first);
    if is_ruby_interpreter(&base) {
        if let Some(wrapped) = background_wrapped_tool_name(rest) {
            return Some(wrapped);
        }
    }
    if base.is_empty() {
        None
    } else {
        Some(base)
    }
}

fn command_token(command: &str) -> Option<(&str, &str)> {
    let s = command.trim_start();
    if s.is_empty() {
        return None;
    }
    Some(
        if let Some(quote) = s.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let rest = &s[quote.len_utf8()..];
            match rest.find(quote) {
                Some(end) => (&rest[..end], &rest[end + quote.len_utf8()..]),
                None => (rest, ""),
            }
        } else {
            match s.find(char::is_whitespace) {
                Some(end) => (&s[..end], &s[end..]),
                None => (s, ""),
            }
        },
    )
}

fn command_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = command;
    while let Some((token, next)) = command_token(rest) {
        if token.is_empty() {
            break;
        }
        tokens.push(token.to_string());
        if next.len() == rest.len() {
            break;
        }
        rest = next;
    }
    tokens
}

fn command_token_base(token: &str) -> String {
    token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(token)
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}

fn is_ruby_interpreter(base: &str) -> bool {
    matches!(base, "ruby" | "ruby.exe") || base.starts_with("ruby3")
}

fn background_wrapped_tool_name(rest: &str) -> Option<String> {
    let mut remaining = rest;
    loop {
        let (token, next) = command_token(remaining)?;
        let base = command_token_base(token);
        if !base.is_empty() && !base.starts_with('-') {
            return Some(base);
        }
        remaining = next;
    }
}

fn background_evidence_tool_name(command: &str) -> String {
    background_command_tool_name(command).unwrap_or_else(|| "background_job".to_string())
}

fn background_evidence_kind(command: &str) -> &'static str {
    let Some(tool) = background_command_tool_name(command) else {
        return "background_job";
    };
    match tool.as_str() {
        "httpx" | "whatweb" | "curl" | "wget" | "http" => "http_probe",
        "nmap" => "nmap",
        "naabu" => "port_probe",
        "dig" | "dnsx" | "host" | "nslookup" => "dns_a",
        "whois" | "asn" | "whois-asn" => "whois",
        "ctfr" => "ct_log",
        _ => "background_job",
    }
}

/// Render a concise, model-facing summary of a finished background job. When an
/// evidence id was booked (P3-c), it is surfaced so the agent can cite it.
fn format_background_note(
    jc: &golish_app_core::background_jobs::JobCompletion,
    evidence_id: Option<i64>,
) -> String {
    let exit = jc
        .exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let mut note = format!(
        "Background job `{}` (`{}`) finished: status={}, exit={}, took {} ms.",
        jc.job_id,
        jc.command,
        jc.status.as_str(),
        exit,
        jc.duration_ms
    );
    if let Some(id) = evidence_id {
        note.push_str(&format!(
            "\n  evidence_id={id} — cite this in a StageDeliverable's evidence_refs."
        ));
    }
    let stdout = jc.stdout_tail.trim();
    if !stdout.is_empty() {
        note.push_str("\n  stdout (tail):\n");
        note.push_str(stdout);
    }
    let stderr = jc.stderr_tail.trim();
    if !stderr.is_empty() {
        note.push_str("\n  stderr (tail):\n");
        note.push_str(stderr);
    }
    note
}

async fn configure_title_gen(bridge: &mut AgentBridge) {
    bridge.set_tool_config(
        golish_agent_kit::tool_definitions::ToolSelectionConfig::with_preset(
            golish_agent_kit::tool_definitions::ToolPreset::None,
        ),
    );
    let mut registry = bridge.tool_registry().write().await;
    registry.clear();
    drop(registry);
    tracing::info!("[configure_bridge] Title-gen session: disabled all tools");
}

async fn configure_core_services(bridge: &mut AgentBridge, state: &AgentState) {
    let workspace_path = bridge.workspace().read().await.clone();
    let sidecar_state = std::sync::Arc::new(golish_sidecar::SidecarState::with_config(
        state.sidecar_config.clone(),
    ));
    if let Err(e) = sidecar_state.initialize(workspace_path).await {
        tracing::warn!("Failed to initialize per-session sidecar: {}", e);
    }
    let sidecar_backend: std::sync::Arc<
        dyn golish_agent_kit::sidecar_trait::SessionCaptureBackend,
    > = std::sync::Arc::new(crate::ai::sidecar_bridge::SidecarCaptureBackend::new(
        sidecar_state,
    ));

    // db tracking + readiness + chain persistence travel together and use
    // a generic readiness-gate bound that can't go through `BridgeBackends`,
    // so call `set_db_backend` directly first — `db_repo` / `embedder`
    // applied via `apply_backends` below need the live tracker to exist.
    let tracking_backend: std::sync::Arc<dyn golish_agent_kit::db_traits::DbTrackingBackend> =
        std::sync::Arc::new(crate::ai::tracking_bridge::PgTrackingBackend::new(
            state.db_pool.clone(),
        ));
    let chain_persistence: std::sync::Arc<dyn golish_sub_agents::SubAgentChainPersistence> =
        std::sync::Arc::new(crate::ai::tracking_bridge::PgChainPersistence::new(
            state.db_pool.clone(),
        ));
    let ready_gate = crate::ai::tracking_bridge::CoreDbReadyGate(state.db_ready.clone());
    bridge.set_db_backend(tracking_backend, ready_gate, chain_persistence);

    let graph_backend = std::sync::Arc::new(crate::ai::graph_bridge::GraphClientBackend::new(
        state.db_pool.clone(),
    ));
    let db_repo: std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider> =
        std::sync::Arc::new(crate::ai::db_bridge::GolishDbRepoProvider::new(
            state.db_pool.clone(),
        ));

    bridge.apply_backends(golish_agent_bridge::BridgeBackends {
        indexer: Some(state.indexer_state.clone()),
        sidecar: Some(sidecar_backend),
        settings: Some(state.settings_manager.clone()),
        graph: Some(graph_backend),
        db_repo: Some(db_repo),
        ..Default::default()
    });
}

fn configure_domain_hooks(bridge: &mut AgentBridge, state: &AgentState) {
    let pool = state.db_pool.clone();
    bridge.set_post_shell_hook(std::sync::Arc::new(
        move |cmd, stdout, project_path, organization_id| {
            let pool = pool.clone();
            Box::pin(async move {
                let store = golish_pentest::output_store::PgPentestStore::new(&pool);
                let _ = golish_pentest::output_store::maybe_detect_and_store_via_context(
                    &store,
                    &cmd,
                    &stdout,
                    project_path.as_deref(),
                    golish_pentest::output_store::StoreContext { organization_id },
                )
                .await;
            })
        },
    ));
    bridge.set_output_classifier(std::sync::Arc::new(|cmd, stdout| {
        golish_pentest::output_store::has_structured_storage(cmd, stdout)
    }));
}

async fn configure_memory_and_embeddings(
    bridge: &mut AgentBridge,
    state: &AgentState,
    settings: &golish_settings::GolishSettings,
) {
    if let Some(ref key) = settings.ai.openai.api_key {
        if !key.is_empty() {
            let base = settings
                .ai
                .openai
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let embedder =
                golish_db::embeddings::HttpEmbedder::new(base, key, "text-embedding-3-small", 1536);
            let bridged = crate::ai::embedder_bridge::EmbedderBridge::new(embedder);
            bridge.set_embedder(std::sync::Arc::new(bridged));
            tracing::info!("[agent] Semantic memory enabled (text-embedding-3-small)");
        }
    }

    let workspace_path = bridge.workspace().read().await.clone();
    let memory_file_path = find_memory_file_for_workspace(&workspace_path, &settings.codebases);
    if let Some(ref path) = memory_file_path {
        tracing::info!(
            "[agent] Using memory file from codebase settings: {}",
            path.display()
        );
    }
    bridge.set_memory_file_path(memory_file_path).await;

    let model_factory =
        golish_agent_kit::llm_client::LlmClientFactory::new(state.settings_manager.clone());
    bridge.set_model_factory(std::sync::Arc::new(model_factory));
}

async fn configure_sub_agents(bridge: &AgentBridge, settings: &golish_settings::GolishSettings) {
    apply_sub_agent_model_settings(bridge, &settings.ai).await;
}

/// Backs the submit tool's closeout reconciliation barrier (Piece 3) with the
/// process-wide background-job manager: reports the running scans this session
/// started so `submit_stage_deliverable` can wait for them before grading.
struct ManagerBackgroundJobs;

#[async_trait::async_trait]
impl crate::ai::harness_submit_tool::BackgroundJobsQuery for ManagerBackgroundJobs {
    async fn running_for_session(
        &self,
        session_id: &str,
    ) -> Vec<crate::ai::harness_submit_tool::RunningJobInfo> {
        golish_app_core::background_jobs::manager()
            .running_for_session(session_id)
            .into_iter()
            .map(|j| crate::ai::harness_submit_tool::RunningJobInfo {
                job_id: j.job_id,
                command: j.command,
                elapsed_ms: j.elapsed_ms,
            })
            .collect()
    }
}

/// Total time the submit-time reconciliation barrier (Piece 3) waits for a
/// session's background scans to settle before telling the agent to wait +
/// resubmit. Tunable via `GOLISH_SUBMIT_RECONCILE_WAIT_MS`; default is 0 so the
/// wait is visible as a separate `wait_for_background_jobs` tool step instead
/// of making the submit card spin for minutes.
const DEFAULT_SUBMIT_RECONCILE_WAIT_MS: u64 = 0;

fn submit_reconcile_wait_ms() -> u64 {
    std::env::var("GOLISH_SUBMIT_RECONCILE_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SUBMIT_RECONCILE_WAIT_MS)
}

async fn register_pentest_tools(
    bridge: &AgentBridge,
    state: &AgentState,
    app_handle: Option<tauri::AppHandle>,
) {
    {
        let pentest_tools = state.pentest_tool_factory.create_ai_tools(
            state.pentest_config_manager.clone(),
            state.pty_manager.clone(),
            state.pty_output_tap.clone(),
            state.active_terminal_session.clone(),
            state.pentest_busy_sessions.clone(),
            state.ai_state.runtime.clone(),
            state.db_pool.clone(),
        );
        let mut registry = bridge.tool_registry().write().await;
        for tool in pentest_tools {
            tracing::info!("[pentest-ai] Registered tool: {}", tool.name());
            registry.register_tool(tool);
        }
    }

    {
        let bridge_tools = state.pentest_tool_factory.create_bridge_tools(
            state.db_pool.clone(),
            state.pentest_config_manager.clone(),
            app_handle,
            bridge.harness_active_org_id_handle(),
        );
        let mut registry = bridge.tool_registry().write().await;
        for tool in bridge_tools {
            tracing::info!("[pentest-bridge] Registered tool: {}", tool.name());
            registry.register_tool(tool);
        }
    }

    // C2c · deterministic StageDeliverable submission tool. The reporter/
    // orchestrator fills typed args; the handler captures the structured
    // deliverable into the bridge side-channel for the stage gate, replacing the
    // fragile "parse a ```json block out of prose" path.
    {
        // P2 · give the tool a read-only evidence-ledger handle so it can run
        // validate-on-submit (reject fabricated evidence_refs immediately rather
        // than returning a misleading `accepted`).
        let evidence_repo: std::sync::Arc<dyn crate::ai::harness_submit_tool::EvidenceLedgerQuery> =
            std::sync::Arc::new(crate::ai::db_bridge::GolishDbRepoProvider::new(
                state.db_pool.clone(),
            ));
        let mut submit_tool = crate::ai::harness_submit_tool::SubmitStageDeliverableTool::new(
            bridge.harness_active_stage_handle(),
            bridge.harness_last_deliverable_handle(),
        )
        .with_evidence_repo(evidence_repo)
        // Share the active engagement-org id so the submit-time gate preview also
        // projects the org-keyed DB business-table truth (ASN/CT/OSINT) — not just
        // the session-keyed command-path facts (DNS/WHOIS/SUBDOMAIN). Without this
        // the per-org recon sub-agent's submit gate marks ASN/CT/OSINT "never
        // attempted" forever and dead-loops even after enrich landed the data.
        .with_org_id_source(bridge.harness_active_org_id_handle())
        // Wave-aware stages freeze their submit-preview denominator to targets
        // present at operation_state.stage_started_at; discoveries during this
        // stage are queued for a follow-up wave instead of moving the current
        // gate.
        .with_operation_id_source(bridge.harness_active_operation_id_handle())
        // Piece 3 · closeout reconciliation barrier: a submit that arrives while
        // this session still has backgrounded scans running defers fast by default
        // and tells the model to call wait_for_background_jobs. Operators can opt
        // back into the old bounded-in-submit wait via GOLISH_SUBMIT_RECONCILE_WAIT_MS.
        .with_background_jobs(std::sync::Arc::new(ManagerBackgroundJobs))
        .with_reconcile_timeouts(submit_reconcile_wait_ms(), 1000);
        // 乙 · scope the real-id suggestion to this chat session (the string both
        // evidence write paths stamp on the ledger) so a fabricated-ref needs_fix
        // can name the operation's real ids.
        if let Some(sid) = bridge.event_session_id() {
            submit_tool = submit_tool.with_session_id(sid);
        }
        let tool = std::sync::Arc::new(submit_tool);
        let mut registry = bridge.tool_registry().write().await;
        tracing::info!("[harness] Registered tool: submit_stage_deliverable");
        registry.register_tool(tool);
    }

    // Observability (design 2026-06-05): self-service run introspection. Lets the
    // agent read its own merged decision timeline (main + sub-agents) when stuck,
    // instead of the user pointing at log files. Scoped to the current chat
    // session so a no-arg call returns this run.
    {
        // Read the run's transcripts from the SAME base the writer uses
        // (workspace-relative for a real workspace); `default_transcript_base`
        // is home-only and would miss workspace runs — the "no logs" symptom.
        // The bridge's workspace is set at construction (before this), so it is
        // available here even though `transcript_base_dir()` is set just after.
        let workspace = bridge.workspace().read().await.clone();
        let mut trace_tool = crate::ai::harness_trace_tool::HarnessTraceTool::new().with_base_dir(
            golish_events::op_trace::resolve_transcript_base(Some(&workspace)),
        );
        if let Some(sid) = bridge.event_session_id() {
            trace_tool = trace_tool.with_session_id(sid);
        }
        let tool = std::sync::Arc::new(trace_tool);
        let mut registry = bridge.tool_registry().write().await;
        tracing::info!("[harness] Registered tool: harness_trace");
        registry.register_tool(tool);
    }

    // Lead-agent decision handoff tool. Always registered; it only reaches the LLM
    // via the task-primary policy (`BridgeToolSelection.start_operation`). The lead
    // turn calls it to begin the structured planner; the Task-mode router reads the
    // captured objective from the bridge side-channel after the lead turn.
    {
        let tool = std::sync::Arc::new(crate::ai::start_operation_tool::StartOperationTool::new(
            bridge.pending_plan_request_handle(),
        ));
        let mut registry = bridge.tool_registry().write().await;
        tracing::info!("[lead-agent] Registered tool: start_operation");
        registry.register_tool(tool);
    }
}

async fn register_visible_pty_tool(bridge: &AgentBridge, state: &AgentState) {
    let visible_cmd_tool = golish_app_core::pty_interactive::VisibleRunPtyCmdTool::new(
        state.pty_manager.clone(),
        state.pty_output_tap.clone(),
        state.active_terminal_session.clone(),
    );
    let mut registry = bridge.tool_registry().write().await;
    registry.register_tool(Arc::new(visible_cmd_tool));
    // Companion poll tool for commands moved to the background on soft-timeout
    // (shares the process-global background-job manager — no per-call state).
    registry.register_tool(Arc::new(golish_app_core::pty_interactive::CheckJobTool));
    // Companion cancel tool: lets the agent kill a stuck background job (e.g. a
    // hung DNS AXFR) after check_job shows no progress, instead of waiting out
    // the hard-timeout watchdog. Same process-global manager — no per-call state.
    registry.register_tool(Arc::new(golish_app_core::pty_interactive::KillJobTool));
    registry.register_tool(Arc::new(
        golish_app_core::pty_interactive::WaitForBackgroundJobsTool,
    ));
    tracing::info!(
        "[configure_bridge] Registered VisibleRunPtyCmdTool + CheckJobTool + KillJobTool + WaitForBackgroundJobsTool (background job control)"
    );
}

/// MCP tool executor that routes tool calls through the MCP manager.
///
/// Handles tools with the `mcp__` prefix; returns `None` for all others.
pub struct McpManagerToolExecutor {
    manager: Arc<golish_mcp::McpManager>,
}

impl McpManagerToolExecutor {
    pub fn new(manager: Arc<golish_mcp::McpManager>) -> Self {
        Self { manager }
    }
}

#[async_trait::async_trait]
impl golish_agent_runtime::agentic_loop::McpToolExecutor for McpManagerToolExecutor {
    async fn execute_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)> {
        if !tool_name.starts_with("mcp__") {
            return None;
        }
        match self.manager.call_tool(tool_name, args.clone()).await {
            Ok(result) => {
                let (value, success) = golish_mcp::convert_mcp_result_to_tool_result(result);
                Some((value, success))
            }
            Err(e) => {
                tracing::error!("[mcp] Tool call failed for '{}': {}", tool_name, e);
                Some((serde_json::json!({"error": e.to_string()}), false))
            }
        }
    }
}

/// Set up MCP tool definitions and executor on a bridge from the global MCP manager.
/// This is called during bridge configuration and also when MCP servers change.
pub async fn setup_bridge_mcp_tools(bridge: &AgentBridge, state: &AgentState) {
    let manager_guard = state.mcp_manager.read().await;
    let Some(manager) = manager_guard.as_ref() else {
        tracing::debug!("[mcp] Global MCP manager not yet initialized, skipping tool setup");
        return;
    };

    let manager = Arc::clone(manager);
    drop(manager_guard);

    match manager.list_tools().await {
        Ok(tools) => {
            let tool_definitions: Vec<rig::completion::ToolDefinition> =
                tools.iter().map(|tool| tool.to_tool_definition()).collect();

            tracing::info!(
                "[mcp] Setting {} MCP tools on bridge",
                tool_definitions.len()
            );

            let executor = Arc::new(McpManagerToolExecutor {
                manager: Arc::clone(&manager),
            });

            bridge.set_mcp_tools(tool_definitions).await;
            bridge.set_mcp_executor(executor).await;
        }
        Err(e) => {
            tracing::warn!("[mcp] Failed to list MCP tools: {}", e);
        }
    }
}

/// Apply sub-agent model overrides from settings to the registry.
async fn apply_sub_agent_model_settings(
    bridge: &AgentBridge,
    ai_settings: &golish_settings::schema::AiSettings,
) {
    let mut registry = bridge.sub_agent_registry().write().await;

    for (agent_id, config) in &ai_settings.sub_agent_models {
        if let Some(agent) = registry.get_mut(agent_id) {
            if let (Some(provider), Some(model)) = (&config.provider, &config.model) {
                let provider_str = provider.to_string();
                agent.set_model_override(&provider_str, model);
                tracing::info!(
                    "Sub-agent '{}' configured to use {}/{}",
                    agent_id,
                    provider_str,
                    model
                );
            }
            agent.temperature = config.temperature;
            agent.max_tokens = config.max_tokens;
            agent.top_p = config.top_p;
        } else {
            tracing::warn!(
                "Sub-agent model config for '{}' ignored: agent not found in registry",
                agent_id
            );
        }
    }
}

/// Find the memory file path for a workspace by matching against indexed codebases.
pub(crate) fn find_memory_file_for_workspace(
    workspace_path: &std::path::Path,
    codebases: &[golish_settings::schema::CodebaseConfig],
) -> Option<std::path::PathBuf> {
    // Canonicalize workspace path for comparison
    let workspace_canonical = workspace_path.canonicalize().ok()?;

    // Find matching codebase
    for config in codebases {
        let codebase_path = golish_core::paths::expand_tilde(&config.path);
        if let Ok(codebase_canonical) = codebase_path.canonicalize() {
            // Check if workspace is the codebase or a subdirectory
            if workspace_canonical == codebase_canonical
                || workspace_canonical.starts_with(&codebase_canonical)
            {
                // Found matching codebase
                if let Some(ref memory_file) = config.memory_file {
                    // Return just the filename - it will be resolved relative to workspace
                    return Some(std::path::PathBuf::from(memory_file));
                }
                // Codebase found but no memory file configured
                return None;
            }
        }
    }

    // No matching codebase found
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_app_core::background_jobs::{JobCompletion, JobStatus};

    fn sample_completion() -> JobCompletion {
        JobCompletion {
            job_id: "job_abc123".to_string(),
            session_id: Some("sess-1".to_string()),
            organization_id: None,
            command: "nmap -p- example.com".to_string(),
            status: JobStatus::Done,
            exit_code: Some(0),
            stdout_tail: "open: 80,443".to_string(),
            stderr_tail: String::new(),
            duration_ms: 4200,
        }
    }

    #[test]
    fn note_includes_evidence_id_when_booked() {
        let note = format_background_note(&sample_completion(), Some(42));
        assert!(note.contains("evidence_id=42"), "note: {note}");
        assert!(note.contains("evidence_refs"), "note: {note}");
        assert!(note.contains("job_abc123"));
        assert!(note.contains("open: 80,443"));
    }

    #[test]
    fn note_omits_evidence_id_when_not_booked() {
        let note = format_background_note(&sample_completion(), None);
        assert!(!note.contains("evidence_id="), "note: {note}");
        assert!(note.contains("status=done"));
    }

    #[test]
    fn submit_reconcile_default_defers_to_visible_wait_tool() {
        assert_eq!(DEFAULT_SUBMIT_RECONCILE_WAIT_MS, 0);
    }

    #[test]
    fn background_httpx_command_books_http_probe_kind() {
        let cmd = r#""/Users/me/Application Support/golish-platform/tools/httpx/httpx" -u https://example.com -silent"#;
        assert_eq!(background_command_tool_name(cmd).as_deref(), Some("httpx"));
        assert_eq!(background_evidence_tool_name(cmd), "httpx");
        assert_eq!(background_evidence_kind(cmd), "http_probe");
    }

    #[test]
    fn background_ruby_wrapped_whatweb_books_http_probe_kind() {
        let cmd = r#""/Users/me/.rbenv/versions/3.2.11/bin/ruby" "/Users/me/Application Support/golish-platform/tools/whatweb/whatweb" -a 1 https://example.com"#;
        assert_eq!(
            background_command_tool_name(cmd).as_deref(),
            Some("whatweb")
        );
        assert_eq!(background_evidence_tool_name(cmd), "whatweb");
        assert_eq!(background_evidence_kind(cmd), "http_probe");
    }

    #[test]
    fn background_port_scan_commands_keep_real_tool_names() {
        assert_eq!(background_evidence_tool_name("nmap -sV 1.2.3.4"), "nmap");
        assert_eq!(background_evidence_kind("nmap -sV 1.2.3.4"), "nmap");

        let cmd = r#""/Users/me/Application Support/golish-platform/tools/naabu/naabu" -host 1.2.3.4 -silent"#;
        assert_eq!(background_evidence_tool_name(cmd), "naabu");
        assert_eq!(background_evidence_kind(cmd), "port_probe");
    }

    #[test]
    fn batch_port_input_file_is_recovered_from_quoted_command() {
        let cmd = r#""/Users/me/Application Support/golish-platform/tools/naabu/naabu" -list .golish/tool-inputs/targets.txt -silent"#;
        assert_eq!(
            batch_input_file_from_command(cmd, Some("/tmp/ws"), "naabu")
                .unwrap()
                .to_string_lossy(),
            "/tmp/ws/.golish/tool-inputs/targets.txt"
        );
    }

    #[test]
    fn batch_service_input_file_is_recovered_from_equals_command() {
        let cmd = r#""/Users/me/Application Support/golish-platform/tools/whatweb/whatweb" -a 1 --input-file=.golish/tool-inputs/urls.txt --max-threads 25"#;
        assert_eq!(
            batch_input_file_from_command(cmd, Some("/tmp/ws"), "whatweb")
                .unwrap()
                .to_string_lossy(),
            "/tmp/ws/.golish/tool-inputs/urls.txt"
        );
    }

    #[test]
    fn batch_service_input_file_strips_quotes_around_absolute_equals_value() {
        let cmd = r#"whatweb -a 1 --input-file='/Users/me/ws/.golish/tool-inputs/urls.txt' --max-threads 25"#;
        assert_eq!(
            batch_input_file_from_command(cmd, Some("/tmp/ws"), "whatweb")
                .unwrap()
                .to_string_lossy(),
            "/Users/me/ws/.golish/tool-inputs/urls.txt"
        );
    }

    #[test]
    fn batch_liveness_input_file_is_recovered_from_httpx_l_flag() {
        let cmd = r#""/Users/me/Application Support/golish-platform/tools/httpx/httpx" -l .golish/tool-inputs/hosts.txt -json -silent"#;
        assert_eq!(
            batch_input_file_from_command(cmd, Some("/tmp/ws"), "httpx")
                .unwrap()
                .to_string_lossy(),
            "/tmp/ws/.golish/tool-inputs/hosts.txt"
        );
    }

    #[test]
    fn batch_liveness_input_is_recovered_from_httpx_quoted_heredoc() {
        let cmd = "\"/Users/me/Application Support/golish-platform/tools/httpx/httpx\" -json -sc -silent <<'GOLISH_STDIN'\nhttp://39.99.254.48\nqs.stock.pingan.com\nGOLISH_STDIN";
        assert_eq!(
            heredoc_body_from_command(cmd).as_deref(),
            Some("http://39.99.254.48\nqs.stock.pingan.com")
        );
        assert!(is_batch_liveness_command("httpx", cmd));
        assert_eq!(
            batch_input_targets(&heredoc_body_from_command(cmd).unwrap()),
            vec!["http://39.99.254.48", "qs.stock.pingan.com"]
        );
    }

    #[test]
    fn batch_liveness_input_is_recovered_from_httpx_dev_stdin_heredoc() {
        let cmd = "\"/Users/me/Application Support/golish-platform/tools/httpx/httpx\" -l /dev/stdin -sc -silent 2>&1 << 'EOF'\nhttps://www.example.com\nhttp://www.example.net\nEOF";
        assert_eq!(
            heredoc_body_from_command(cmd).as_deref(),
            Some("https://www.example.com\nhttp://www.example.net")
        );
        assert!(is_batch_liveness_command("httpx", cmd));
    }

    #[test]
    fn batch_liveness_and_service_commands_are_classified_by_intent() {
        assert!(is_batch_liveness_command(
            "httpx",
            r#"httpx -l .golish/tool-inputs/hosts.txt -json -silent"#
        ));
        assert!(is_batch_liveness_command(
            "httpx",
            r#"httpx --list=.golish/tool-inputs/hosts.txt -json -silent"#
        ));
        assert!(is_batch_liveness_command(
            "nmap",
            r#"nmap -sn -iL .golish/tool-inputs/hosts.txt -T4"#
        ));
        assert!(!is_batch_service_command(
            "nmap",
            r#"nmap -sn -iL .golish/tool-inputs/hosts.txt -T4"#
        ));
        assert!(is_batch_service_command(
            "nmap",
            r#"nmap -sV -iL .golish/tool-inputs/hosts.txt -T4"#
        ));
        assert!(is_batch_service_command(
            "whatweb",
            r#"whatweb --input-file=.golish/tool-inputs/urls.txt"#
        ));
    }

    #[test]
    fn service_output_mentions_canonical_target_from_url_input() {
        let stdout = "https://Example.COM [200 OK] nginx";
        assert!(service_output_mentions_target(stdout, "example.com"));
        assert!(!service_output_mentions_target(
            stdout,
            "missing.example.com"
        ));
    }

    #[test]
    fn service_output_target_match_does_not_hit_ip_prefix() {
        let stdout = "http://120.233.149.110 [200 OK] nginx";
        assert!(service_output_mentions_target(stdout, "120.233.149.110"));
        assert!(!service_output_mentions_target(stdout, "120.233.149.1"));
    }

    #[test]
    fn open_port_counts_canonicalize_batch_hits() {
        let counts = open_port_counts("naabu", "Example.COM:443\n1.2.3.4:80\nnoise\n");
        assert_eq!(counts.get("example.com"), Some(&1));
        assert_eq!(counts.get("1.2.3.4"), Some(&1));
    }

    #[test]
    fn eas_outcome_scope_keys_include_target_value_and_real_ip() {
        let rows = vec![
            (
                "https://Example.COM/login".to_string(),
                "1.2.3.4".to_string(),
            ),
            (
                "www.example.net".to_string(),
                "5.6.7.8; 2001:db8::1".to_string(),
            ),
        ];

        let liveness = eas_outcome_asset_keys_from_rows(rows.clone(), EasOutcomeKeyMode::Liveness);
        assert!(liveness.contains("example.com/login"));
        assert!(liveness.contains("1.2.3.4"));
        assert!(liveness.contains("www.example.net"));
        assert!(liveness.contains("5.6.7.8"));
        assert!(liveness.contains("2001:db8::1"));

        let host = eas_outcome_asset_keys_from_rows(rows, EasOutcomeKeyMode::Host);
        assert!(host.contains("example.com"));
        assert!(!host.contains("example.com/login"));
        assert!(host.contains("1.2.3.4"));
        assert!(host.contains("www.example.net"));
    }

    #[test]
    fn eas_outcome_scope_keys_do_not_include_unrelated_guess() {
        let rows = vec![("pinganfdc.com".to_string(), String::new())];
        let liveness = eas_outcome_asset_keys_from_rows(rows, EasOutcomeKeyMode::Liveness);

        assert!(liveness.contains("pinganfdc.com"));
        assert!(!liveness.contains("149.120.175.217"));
        assert!(!liveness.contains("fzpingan.cn"));
    }

    #[test]
    fn background_unknown_command_keeps_generic_kind() {
        assert_eq!(
            background_evidence_kind("custom-tool --flag"),
            "background_job"
        );
        assert_eq!(
            background_evidence_tool_name("custom-tool --flag"),
            "custom-tool"
        );
    }
}
