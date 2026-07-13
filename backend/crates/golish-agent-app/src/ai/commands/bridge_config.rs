//! Agent-bridge wiring: assembles shared services (sidecar, db, graph,
//! memory, sub-agents, pentest/MCP tools) onto a per-session [`AgentBridge`].

use std::collections::{BTreeSet, HashMap};
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
    }
}

/// Fully subscribed but not yet running listener pair. GUI init constructs this
/// only after every fallible/async bridge setup step, then activates it inside
/// the stable-session publish transition. The overlap with the old subscriber
/// prevents a completion gap; `JobCompletion::try_claim_processing` makes that
/// overlap exactly-once for evidence/DB side effects.
pub(crate) struct PreparedBridgeBackgroundListeners {
    session_id: String,
    output_rx: tokio::sync::broadcast::Receiver<golish_app_core::background_jobs::JobOutputChunk>,
    completion_rx:
        tokio::sync::broadcast::Receiver<golish_app_core::background_jobs::JobCompletion>,
    db_repo: Arc<dyn golish_agent_kit::db_traits::DbRepoProvider>,
    db_pool: Arc<sqlx::PgPool>,
    project_path: Option<String>,
    retired: tokio::sync::watch::Receiver<bool>,
}

pub(crate) async fn prepare_bridge_background_listeners(
    bridge: &AgentBridge,
    state: &AgentState,
) -> Option<PreparedBridgeBackgroundListeners> {
    if bridge
        .event_session_id()
        .is_some_and(golish_core::is_title_gen_session_id)
    {
        return None;
    }
    let session_id = bridge.event_session_id()?.to_string();
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
    // Claim only after the last await. Once claimed, both tasks are spawned
    // synchronously so cancellation cannot strand a published bridge in a
    // permanently "started" state with no listeners.
    let retired = bridge.claim_background_listener_lifecycle()?;
    let manager = golish_app_core::background_jobs::manager();
    Some(PreparedBridgeBackgroundListeners {
        session_id,
        output_rx: manager.subscribe_output_chunks(),
        completion_rx: manager.subscribe_completions(),
        db_repo: bg_repo,
        db_pool: state.db_pool.clone(),
        project_path: bg_project_path,
        retired,
    })
}

pub(crate) fn activate_bridge_background_listeners(
    bridge: &AgentBridge,
    prepared: PreparedBridgeBackgroundListeners,
) {
    let event_tx = bridge.get_or_create_event_tx();
    let notes = bridge.background_notes_handle();
    spawn_background_output_listener(
        prepared.session_id.clone(),
        event_tx.clone(),
        prepared.output_rx,
        prepared.retired.clone(),
    );
    spawn_background_completion_listener(
        prepared.session_id,
        notes,
        event_tx,
        prepared.completion_rx,
        prepared.db_repo,
        prepared.db_pool,
        prepared.project_path,
        prepared.retired,
    );
}

/// Convenience for standalone bridges that have no replacement handoff. GUI
/// init uses prepare + publish-transition activation instead.
pub async fn configure_bridge_background_listeners(bridge: &AgentBridge, state: &AgentState) {
    if let Some(prepared) = prepare_bridge_background_listeners(bridge, state).await {
        activate_bridge_background_listeners(bridge, prepared);
    }
}

/// Wire this session's background-job stdout/stderr chunks into the normal tool
/// output event stream. The frontend already knows how to append
/// `ToolOutputChunk` to shell-like tool panels, so attributed `pentest_run`
/// background jobs become visible without a separate UI path.
fn spawn_background_output_listener(
    session_id: String,
    event_tx: tokio::sync::mpsc::UnboundedSender<golish_core::events::AiEvent>,
    mut rx: tokio::sync::broadcast::Receiver<golish_app_core::background_jobs::JobOutputChunk>,
    mut retired: tokio::sync::watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        loop {
            let received = tokio::select! {
                biased;
                changed = retired.changed() => {
                    if changed.is_err() || *retired.borrow() {
                        let mut remaining_at_retirement = rx.len();
                        while remaining_at_retirement > 0 {
                            match rx.try_recv() {
                                Ok(chunk) => {
                                    remaining_at_retirement -= 1;
                                    process_background_output_chunk(&session_id, &event_tx, chunk);
                                }
                                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(dropped)) => {
                                    remaining_at_retirement = remaining_at_retirement.saturating_sub(
                                        usize::try_from(dropped).unwrap_or(usize::MAX),
                                    );
                                    tracing::warn!(
                                        "[background-output-listener] retirement drain lagged; dropped {} chunk(s)",
                                        dropped
                                    );
                                }
                                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
                                | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                            }
                        }
                        break;
                    }
                    continue;
                }
                received = rx.recv() => received,
            };
            match received {
                Ok(chunk) => process_background_output_chunk(&session_id, &event_tx, chunk),
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

fn process_background_output_chunk(
    session_id: &str,
    event_tx: &tokio::sync::mpsc::UnboundedSender<golish_core::events::AiEvent>,
    chunk: golish_app_core::background_jobs::JobOutputChunk,
) {
    use golish_core::events::AiEvent;

    if chunk.session_id.as_deref() != Some(session_id) || !chunk.try_claim_processing() {
        return;
    }
    let _ = event_tx.send(AiEvent::ToolOutputChunk {
        request_id: chunk.request_id,
        tool_name: chunk.tool_name,
        chunk: chunk.chunk,
        stream: chunk.stream.to_string(),
        source: chunk.source,
    });
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
    session_id: String,
    notes: Arc<std::sync::Mutex<Vec<String>>>,
    event_tx: tokio::sync::mpsc::UnboundedSender<golish_core::events::AiEvent>,
    mut rx: tokio::sync::broadcast::Receiver<golish_app_core::background_jobs::JobCompletion>,
    db_repo: std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider>,
    db_pool: std::sync::Arc<sqlx::PgPool>,
    project_path: Option<String>,
    mut retired: tokio::sync::watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        loop {
            let received = tokio::select! {
                biased;
                changed = retired.changed() => {
                    if changed.is_err() || *retired.borrow() {
                        // A completion broadcast just before the candidate's
                        // pre-subscription exists only in this old queue. Drain
                        // it before exit; broadcasts after pre-subscription are
                        // also seen by the new generation and deduplicated by
                        // the shared processing claim.
                        let mut remaining_at_retirement = rx.len();
                        while remaining_at_retirement > 0 {
                            match rx.try_recv() {
                                Ok(completion) => {
                                    remaining_at_retirement -= 1;
                                    process_background_completion(
                                        &session_id,
                                        &notes,
                                        &event_tx,
                                        &db_repo,
                                        &db_pool,
                                        project_path.as_deref(),
                                        completion,
                                    )
                                    .await;
                                }
                                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(dropped)) => {
                                    remaining_at_retirement = remaining_at_retirement.saturating_sub(
                                        usize::try_from(dropped).unwrap_or(usize::MAX),
                                    );
                                    tracing::warn!(
                                        "[background-listener] retirement drain lagged; dropped {} completion(s)",
                                        dropped
                                    );
                                }
                                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
                                | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                            }
                        }
                        break;
                    }
                    continue;
                }
                received = rx.recv() => received,
            };
            match received {
                Ok(completion) => {
                    process_background_completion(
                        &session_id,
                        &notes,
                        &event_tx,
                        &db_repo,
                        &db_pool,
                        project_path.as_deref(),
                        completion,
                    )
                    .await;
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

async fn process_background_completion(
    session_id: &str,
    notes: &Arc<std::sync::Mutex<Vec<String>>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<golish_core::events::AiEvent>,
    db_repo: &Arc<dyn golish_agent_kit::db_traits::DbRepoProvider>,
    db_pool: &Arc<sqlx::PgPool>,
    project_path: Option<&str>,
    jc: golish_app_core::background_jobs::JobCompletion,
) {
    use golish_core::events::AiEvent;

    if jc.session_id.as_deref() != Some(session_id) {
        return;
    }
    if !jc.try_claim_processing() {
        tracing::debug!(
            job_id = %jc.job_id,
            session_id,
            "background completion already claimed by another bridge generation"
        );
        return;
    }

    tracing::info!(
        message = "[background-listener] job finished",
        session_id,
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

    let evidence_id =
        maybe_append_background_evidence(db_repo, session_id, project_path, &jc).await;
    maybe_store_background_structured_output(db_pool, project_path, &jc).await;
    maybe_store_background_batch_liveness_outcomes(
        db_pool,
        session_id,
        project_path,
        &jc,
        evidence_id,
    )
    .await;
    maybe_store_background_batch_port_outcomes(db_pool, session_id, project_path, &jc, evidence_id)
        .await;
    maybe_store_background_batch_service_outcomes(
        db_pool,
        session_id,
        project_path,
        &jc,
        evidence_id,
    )
    .await;
    maybe_store_background_vuln_outcomes(db_pool, session_id, project_path, &jc, evidence_id).await;

    if let Some(evidence_id) = evidence_id {
        let evidence_kind = background_evidence_kind(&jc.command);
        let _ = event_tx.send(AiEvent::HarnessTrace {
            operation_id: session_id.to_string(),
            stage: String::new(),
            agent_path: "main".to_string(),
            trace: golish_core::events::HarnessTraceKind::EvidenceBooked {
                tool: evidence_kind.to_string(),
                evidence_id,
                source: "background".to_string(),
            },
        });
    }

    let note = format_background_note(&jc, evidence_id);
    match notes.lock() {
        Ok(mut queue) => queue.push(note),
        Err(poisoned) => poisoned.into_inner().push(note),
    }
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
            ..Default::default()
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
    if tool == "httpx" {
        if let Some(allowed_host_assets) =
            scoped_eas_outcome_asset_keys(db_pool, organization_id, EasOutcomeKeyMode::Host).await
        {
            let hits = httpx_probe_hits(&retained_stdout);
            persist_eas_web_probe_hits(
                db_pool,
                organization_id,
                project_path,
                &source,
                evidence_id,
                &allowed_host_assets,
                &hits,
            )
            .await;
        }
    }

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
        // Dead-asset ongoing marking (design 2026-07-02-dead-asset-liveness-state
        // §4): an EAS liveness probe covered this asset and httpx did not report it
        // alive → stamp the matching target(s) 'dead'. The DB write is guarded
        // (only fires while the row still has no alive signal + is not already
        // 'alive'), so a host that naabu proves has open ports stays/gets 'alive'
        // regardless of landing order. Found assets are stamped 'alive' by the
        // web-probe landing above; nothing to do here for them.
        if !found {
            mark_eas_liveness_dead_asset(db_pool, organization_id, project_path, &asset).await;
        }
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
    use golish_agent_kit::harness::evidence_facts::{TECH_EAS_LIVENESS, TECH_EAS_PORT};
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
    let open_hits = open_port_hits(tool.as_str(), &retained_stdout);
    let source = background_evidence_tool_name(&jc.command);
    persist_eas_open_port_hits(
        db_pool,
        organization_id,
        project_path,
        &source,
        evidence_id,
        &allowed_assets,
        &open_hits,
    )
    .await;

    let target_assets = targets
        .iter()
        .filter(|target| {
            !(target.contains('/')
                && !target.starts_with("http://")
                && !target.starts_with("https://"))
        })
        .filter_map(|target| golish_pentest_domain::canonical_asset_key(target).map(|key| key.key))
        .filter(|asset| allowed_assets.contains(asset))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let existing_open_ports =
        match golish_db::repo::coverage_truth::confirmed_open_service_ports_for_assets(
            db_pool,
            Some(organization_id),
            project_path,
            &target_assets,
        )
        .await
        {
            Ok(rows) => rows
                .into_iter()
                .map(|row| (row.asset, row.ports))
                .collect::<HashMap<_, _>>(),
            Err(err) => {
                tracing::warn!(
                    target: "harness::evidence",
                    job_id = %jc.job_id,
                    org_id = %organization_id,
                    error = %err,
                    "background batch port outcome could not read existing open ports"
                );
                HashMap::new()
            }
        };

    let mut stored = 0usize;
    let mut skipped = 0usize;
    let mut stale_open_skipped = 0usize;
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
        if should_skip_empty_port_outcome(count, existing_open_ports.get(&asset)) {
            skipped += 1;
            stale_open_skipped += 1;
            continue;
        }
        let outcome = if count > 0 { "found" } else { "empty" };
        let write = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
            organization_id,
            run_id: session_id.to_string(),
            asset: asset.clone(),
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
        let liveness_write = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
            organization_id,
            run_id: session_id.to_string(),
            asset,
            technique: TECH_EAS_LIVENESS.to_string(),
            outcome: outcome.to_string(),
            source: Some(source.clone()),
            query: Some(jc.command.clone()),
            result_count: Some(count.min(i32::MAX as usize) as i32),
            confidence: None,
            evidence_ids: vec![evidence_id],
            collected_at: Some(chrono::Utc::now()),
        };
        if let Err(err) =
            golish_db::repo::technique_outcomes::upsert(db_pool, &liveness_write).await
        {
            tracing::warn!(
                target: "harness::evidence",
                job_id = %jc.job_id,
                error = %err,
                "background batch port-derived liveness outcome upsert failed"
            );
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
            stale_open_skipped,
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
    use golish_agent_kit::harness::evidence_facts::{
        TECH_EAS_SERVICE_FINGERPRINT, TECH_EAS_WEB_FINGERPRINT,
    };
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
    let web_hits = match tool.as_str() {
        "whatweb" => whatweb_probe_hits(&retained_stdout),
        _ => Vec::new(),
    };
    persist_eas_web_probe_hits(
        db_pool,
        organization_id,
        project_path,
        &source,
        evidence_id,
        &allowed_assets,
        &web_hits,
    )
    .await;
    if tool == "whatweb" {
        let mut stored = 0usize;
        let mut skipped = 0usize;
        for target in targets {
            let Some(asset) =
                golish_pentest_domain::canonical_asset_key(&target).map(|key| key.key)
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
                technique: TECH_EAS_WEB_FINGERPRINT.to_string(),
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
                        "background batch web-fingerprint outcome upsert failed"
                    );
                }
            }
        }
        tracing::info!(
            target: "harness::evidence",
            job_id = %jc.job_id,
            org_id = %organization_id,
            source = %source,
            stored,
            skipped,
            "background whatweb WEB-FINGERPRINT outcomes stored without SERVICE-FINGERPRINT outcome"
        );
        return;
    }
    let service_hits = match tool.as_str() {
        "nmap" => nmap_service_hits(&retained_stdout),
        _ => Vec::new(),
    };
    persist_eas_service_hits(
        db_pool,
        organization_id,
        project_path,
        &source,
        evidence_id,
        &allowed_assets,
        &service_hits,
    )
    .await;

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

/// vuln_triage 公式化扫描（nuclei）的 `technique_outcomes` 落库
/// （gate-capability-ledger Phase 2 / Task 2.2）。
///
/// crediting 从「动作」转「状态」：nuclei 命中一行 JSON = 某 `(host, WSTG 类)` `found`；
/// 命令 `-tags`/`-tag` 指定的类是「本次真跑过」的覆盖集，跑了没命中 = `empty`（I8）；
/// 既没被 `-tags` 覆盖、也没命中的类不 upsert（保持 `not_attempted`，fail-closed）。
/// 覆盖集 / 命中类由 [`wstg_mapping::wstg_technique_for_tag`] 确定性推导，绝不模型
/// 自报（护栏 1）。`asset` 过 `canonical_asset_key` 归一（护栏 3）。
async fn maybe_store_background_vuln_outcomes(
    db_pool: &sqlx::PgPool,
    session_id: &str,
    project_path: Option<&str>,
    jc: &golish_app_core::background_jobs::JobCompletion,
    evidence_id: Option<i64>,
) {
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
    // vuln_triage 的公式化扫描器：nuclei（模板+tag）、sqlmap（专测 SQLi）、wpscan
    // （WordPress n-day JSON）。每个都由确定性 handler 从真实输出推导覆盖集/命中类
    // （护栏 1，绝不模型自报）。nikto 暂不接：其输出是自由文本，tag→WSTG 归一有歧义，
    // 强接会伪造覆盖（违反 fail-closed），待稳定契约后再扩展。
    if !matches!(tool.as_str(), "nuclei" | "sqlmap" | "wpscan") {
        return;
    }

    let mut targets = vuln_scan_command_targets(&tool, &jc.command);
    if let Some(input) =
        batch_input_text_from_command(&jc.command, project_path, &tool, &jc.job_id, "vuln").await
    {
        targets.extend(batch_input_targets(&input));
    }
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
            "background vuln outcomes skipped: org has no in-scope assets"
        );
        return;
    }

    let retained_stdout = golish_app_core::background_jobs::manager()
        .snapshot(&jc.job_id)
        .map(|snapshot| snapshot.stdout)
        .unwrap_or_else(|| jc.stdout_tail.clone());
    let source = background_evidence_tool_name(&jc.command);

    // 覆盖集（本次真跑的 WSTG 类）+ 逐资产命中计数（按扫描器确定性推导，护栏 1）。
    let covered = vuln_scan_covered_techniques(&tool, &jc.command);
    let hits = vuln_scan_wstg_hits(&tool, &targets, &retained_stdout);

    let mut stored = 0usize;
    let mut skipped = 0usize;
    for target in &targets {
        let Some(asset) = golish_pentest_domain::canonical_asset_key(target).map(|key| key.key)
        else {
            skipped += 1;
            continue;
        };
        if !allowed_assets.contains(&asset) {
            skipped += 1;
            continue;
        }
        let asset_hits = hits.get(&asset);
        // 本资产要落的技术 = 覆盖集 ∪ 命中集（命中但未在 -tags 里的真命中也算 found）。
        let mut techniques: std::collections::BTreeSet<&'static str> =
            covered.iter().copied().collect();
        if let Some(asset_hits) = asset_hits {
            techniques.extend(asset_hits.keys().copied());
        }
        for technique in techniques {
            let count = asset_hits
                .and_then(|h| h.get(technique))
                .copied()
                .unwrap_or(0);
            let outcome = if count > 0 { "found" } else { "empty" };
            let write = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
                organization_id,
                run_id: session_id.to_string(),
                asset: asset.clone(),
                technique: technique.to_string(),
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
                        "background vuln technique_outcome upsert failed"
                    );
                }
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
            "background vuln outcomes stored"
        );
    }
}

/// vuln_triage 公式化扫描器的命令行目标（逗号分隔可多个）。批量 `-l`/`--url-file`
/// 目标由 [`batch_input_text_from_command`] 另行读入。
fn vuln_scan_command_targets(tool: &str, command: &str) -> Vec<String> {
    match tool {
        "nuclei" => nuclei_command_targets(command),
        // sqlmap / wpscan 用 `-u`/`--url` 指定目标。
        "sqlmap" | "wpscan" => {
            let mut targets = Vec::new();
            if let Some(value) = command_flag_value(command, &["-u", "--url"]) {
                for part in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    targets.push(part.to_string());
                }
            }
            targets
        }
        _ => Vec::new(),
    }
}

/// 「本次真跑过」的 WSTG 覆盖集（确定性，护栏 1：由工具身份 + 命令推导，绝不模型自报）。
///
/// - nuclei：`-tags` 归一（见 [`nuclei_covered_techniques`]）。
/// - sqlmap：专测 SQLi 的工具，跑了就等于「尝试过 WSTG-INPV-05」。
/// - wpscan：专找 WordPress 已知漏洞，跑了就等于「尝试过 GOLISH-NDAY」。
fn vuln_scan_covered_techniques(
    tool: &str,
    command: &str,
) -> std::collections::BTreeSet<&'static str> {
    use golish_agent_kit::harness::wstg_mapping::{GOLISH_NDAY, WSTG_SQLI};
    match tool {
        "nuclei" => nuclei_covered_techniques(command),
        "sqlmap" => std::collections::BTreeSet::from([WSTG_SQLI]),
        "wpscan" => std::collections::BTreeSet::from([GOLISH_NDAY]),
        _ => std::collections::BTreeSet::new(),
    }
}

/// 解析扫描器输出 → 每个 host 资产命中的 WSTG 类及次数（确定性）。
///
/// nuclei 的命中天然带 host（`matched-at`）故逐行聚合；sqlmap/wpscan 是单技术工具，
/// 命中是「全局布尔」（此 run 是否证到 SQLi / n-day），故把全局命中归到命令行的
/// 每个目标资产上（这些工具通常单目标 `-u`）。
fn vuln_scan_wstg_hits(
    tool: &str,
    targets: &[String],
    stdout: &str,
) -> HashMap<String, HashMap<&'static str, usize>> {
    use golish_agent_kit::harness::wstg_mapping::{GOLISH_NDAY, WSTG_SQLI};
    match tool {
        "nuclei" => nuclei_wstg_hits(stdout),
        "sqlmap" => {
            let hit = if sqlmap_injection_confirmed(stdout) {
                vec![WSTG_SQLI]
            } else {
                vec![]
            };
            single_technique_hits(targets, &hit)
        }
        "wpscan" => {
            let hit = if wpscan_vulnerability_found(stdout) {
                vec![GOLISH_NDAY]
            } else {
                vec![]
            };
            single_technique_hits(targets, &hit)
        }
        _ => HashMap::new(),
    }
}

/// 把单技术工具的「全局命中类」归到命令行每个目标资产上（过 `canonical_asset_key`
/// 归一，护栏 3）。命中集为空 → 返回空 map（覆盖集仍会让 handler 记 `empty`，I8）。
fn single_technique_hits(
    targets: &[String],
    hit_techniques: &[&'static str],
) -> HashMap<String, HashMap<&'static str, usize>> {
    let mut hits: HashMap<String, HashMap<&'static str, usize>> = HashMap::new();
    if hit_techniques.is_empty() {
        return hits;
    }
    for target in targets {
        let Some(asset) = golish_pentest_domain::canonical_asset_key(target).map(|k| k.key) else {
            continue;
        };
        let entry = hits.entry(asset).or_default();
        for tech in hit_techniques {
            *entry.entry(*tech).or_insert(0) += 1;
        }
    }
    hits
}

/// sqlmap 是否确定性证到注入点。只认无歧义的成功标记（fail-closed）。
fn sqlmap_injection_confirmed(stdout: &str) -> bool {
    let lower = stdout.to_ascii_lowercase();
    lower.contains("sqlmap identified the following injection point")
        || lower.contains("the following injection point(s)")
        || (lower.contains("parameter") && lower.contains("is vulnerable"))
}

/// wpscan（`--format json` 或文本）是否报出至少一个漏洞。
///
/// 优先解析 JSON：任一 `vulnerabilities` 数组非空 → 命中；能解析但全空 → 未命中。
/// 非 JSON（未加 `--format json`）→ 不猜（fail-closed，返回 false，记为 checked_empty）。
fn wpscan_vulnerability_found(stdout: &str) -> bool {
    let trimmed = stdout.trim_start();
    let start = trimmed.find('{');
    let Some(start) = start else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[start..]) else {
        return false;
    };
    json_has_non_empty_vulnerabilities(&value)
}

/// 递归查 wpscan JSON 里是否存在非空 `vulnerabilities` 数组。
fn json_has_non_empty_vulnerabilities(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Array(items)) = map.get("vulnerabilities") {
                if !items.is_empty() {
                    return true;
                }
            }
            map.values().any(json_has_non_empty_vulnerabilities)
        }
        serde_json::Value::Array(items) => items.iter().any(json_has_non_empty_vulnerabilities),
        _ => false,
    }
}

/// nuclei 命令行里 `-u`/`-target`/`-host` 直接指定的目标（逗号分隔可多个）。
/// `-l`/`-list` 文件目标由 [`batch_input_text_from_command`] 另行读入。
fn nuclei_command_targets(command: &str) -> Vec<String> {
    let mut targets = Vec::new();
    if let Some(value) = command_flag_value(command, &["-u", "-target", "-host"]) {
        for part in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            targets.push(part.to_string());
        }
    }
    targets
}

/// 从 `-tags`/`-tag` 推导「本次真跑过」的 WSTG 覆盖集（确定性，护栏 1）。
/// 无 `-tags` ⇒ 空集 ⇒ 只对真命中记 `found`，不臆造 `empty`（I8 fail-closed）。
fn nuclei_covered_techniques(command: &str) -> std::collections::BTreeSet<&'static str> {
    let mut covered = std::collections::BTreeSet::new();
    if let Some(value) = command_flag_value(command, &["-tags", "-tag", "--tags"]) {
        for tag in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(tech) = golish_agent_kit::harness::wstg_mapping::wstg_technique_for_tag(tag)
            {
                covered.insert(tech);
            }
        }
    }
    covered
}

/// 解析 nuclei JSON-lines stdout，聚合每个 host 资产命中的 WSTG 类及次数。
/// host 取自 `matched-at` / `host` / `ip`，过 `canonical_asset_key` 归一；类取自
/// `info.tags`（数组或逗号串）经 [`wstg_mapping::wstg_technique_for_tag`] 归一。
fn nuclei_wstg_hits(stdout: &str) -> HashMap<String, HashMap<&'static str, usize>> {
    let mut hits: HashMap<String, HashMap<&'static str, usize>> = HashMap::new();
    for line in stdout.lines().map(str::trim).filter(|l| l.starts_with('{')) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let host_raw = value
            .get("matched-at")
            .or_else(|| value.get("matched_at"))
            .or_else(|| value.get("host"))
            .or_else(|| value.get("ip"))
            .and_then(|v| v.as_str());
        let Some(host_raw) = host_raw else {
            continue;
        };
        let Some(asset) = golish_pentest_domain::canonical_asset_key(host_raw).map(|k| k.key)
        else {
            continue;
        };
        let techniques = nuclei_line_techniques(&value);
        if techniques.is_empty() {
            continue;
        }
        let entry = hits.entry(asset).or_default();
        for tech in techniques {
            *entry.entry(tech).or_insert(0) += 1;
        }
    }
    hits
}

/// 从一行 nuclei JSON 的 `info.tags`（数组或逗号串）归一出命中的 WSTG 类。
fn nuclei_line_techniques(value: &serde_json::Value) -> std::collections::BTreeSet<&'static str> {
    let mut techniques = std::collections::BTreeSet::new();
    let tags = value.get("info").and_then(|info| info.get("tags"));
    let mut push_tag = |tag: &str| {
        if let Some(tech) = golish_agent_kit::harness::wstg_mapping::wstg_technique_for_tag(tag) {
            techniques.insert(tech);
        }
    };
    match tags {
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                if let Some(tag) = item.as_str() {
                    push_tag(tag);
                }
            }
        }
        Some(serde_json::Value::String(s)) => {
            for tag in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                push_tag(tag);
            }
        }
        _ => {}
    }
    techniques
}

/// 取命令行某个 flag 的值，支持 `-flag value` 与 `-flag=value` 两种形式。
/// 返回第一个命中的 flag 的值。
fn command_flag_value(command: &str, flags: &[&str]) -> Option<String> {
    let tokens = command_tokens(command);
    let mut iter = tokens.iter().peekable();
    while let Some(token) = iter.next() {
        for flag in flags {
            if let Some(rest) = token.strip_prefix(flag) {
                if rest.is_empty() {
                    if let Some(next) = iter.peek() {
                        return Some(next.to_string());
                    }
                } else if let Some(value) = rest.strip_prefix('=') {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenPortHit {
    host: String,
    asset: String,
    port: i32,
    transport: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceHit {
    host: String,
    asset: String,
    port: i32,
    transport: String,
    service_name: Option<String>,
    service_product: Option<String>,
    service_version: Option<String>,
    banner: Option<String>,
    raw_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebProbeHit {
    url: String,
    host: String,
    asset: String,
    scheme: String,
    port: i32,
    status_code: Option<i32>,
    title: Option<String>,
    webserver: Option<String>,
    content_type: Option<String>,
    technologies: Vec<String>,
    ip: Option<String>,
    raw_line: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct EasLandingTargetRow {
    id: uuid::Uuid,
    value: String,
    target_type: String,
    real_ip: String,
    project_path: Option<String>,
}

async fn persist_eas_open_port_hits(
    db_pool: &sqlx::PgPool,
    organization_id: uuid::Uuid,
    project_path: Option<&str>,
    source: &str,
    evidence_id: i64,
    allowed_assets: &BTreeSet<String>,
    hits: &[OpenPortHit],
) {
    let mut stored_targets = 0usize;
    let mut stored_endpoints = 0usize;
    let mut skipped = 0usize;

    for hit in hits {
        if !allowed_assets.contains(&hit.asset) {
            skipped += 1;
            continue;
        }
        let rows = match load_eas_landing_targets_for_asset(
            db_pool,
            organization_id,
            project_path,
            &hit.asset,
        )
        .await
        {
            Ok(rows) => prefer_exact_landing_targets(rows, &hit.asset),
            Err(err) => {
                tracing::warn!(
                    target: "harness::evidence",
                    org_id = %organization_id,
                    asset = %hit.asset,
                    error = %err,
                    "failed to load target rows for EAS port landing"
                );
                skipped += 1;
                continue;
            }
        };
        if rows.is_empty() {
            skipped += 1;
            continue;
        }

        let port_entry = serde_json::json!({
            "port": hit.port,
            "proto": hit.transport,
            "service": "",
            "state": "open",
            "source": source,
            "evidence_id": evidence_id,
        });
        let ports = serde_json::json!([port_entry]);
        for row in &rows {
            match golish_db::repo::targets::update_recon_extended_by_id(
                db_pool, row.id, "", "", "", None, "", "", "", &ports,
            )
            .await
            {
                Ok(()) => stored_targets += 1,
                Err(err) => tracing::warn!(
                    target: "harness::evidence",
                    target_id = %row.id,
                    asset = %hit.asset,
                    error = %err,
                    "failed to persist EAS open port into targets.ports"
                ),
            }
        }

        if let Some(endpoint_ip) = endpoint_ip_for_hit(&hit.host, None, &rows) {
            if let Some(identity) = golish_db::repo::surface_identity::normalize_network_endpoint(
                &endpoint_ip,
                hit.port,
                &hit.transport,
            ) {
                match golish_db::repo::network_endpoints::upsert_by_identity(
                    db_pool,
                    Some(organization_id),
                    project_path,
                    &identity,
                    Some("open"),
                    None,
                    None,
                    None,
                    None,
                    Some(hit.port == 443),
                    Some(source),
                    Some(0.9),
                    true,
                )
                .await
                {
                    Ok(_) => stored_endpoints += 1,
                    Err(err) => tracing::warn!(
                        target: "harness::evidence",
                        asset = %hit.asset,
                        port = hit.port,
                        error = %err,
                        "failed to persist EAS open port into network_endpoints"
                    ),
                }
            }
        }
    }

    if stored_targets > 0 || stored_endpoints > 0 || skipped > 0 {
        tracing::info!(
            target: "harness::evidence",
            org_id = %organization_id,
            source,
            stored_targets,
            stored_endpoints,
            skipped,
            "background EAS open ports landed into target surface tables"
        );
    }
}

async fn persist_eas_service_hits(
    db_pool: &sqlx::PgPool,
    organization_id: uuid::Uuid,
    project_path: Option<&str>,
    source: &str,
    evidence_id: i64,
    allowed_assets: &BTreeSet<String>,
    hits: &[ServiceHit],
) {
    let mut stored_targets = 0usize;
    let mut stored_endpoints = 0usize;
    let mut stored_fingerprints = 0usize;
    let mut skipped = 0usize;

    for hit in hits {
        if !allowed_assets.contains(&hit.asset) {
            skipped += 1;
            continue;
        }
        let rows = match load_eas_landing_targets_for_asset(
            db_pool,
            organization_id,
            project_path,
            &hit.asset,
        )
        .await
        {
            Ok(rows) => prefer_exact_landing_targets(rows, &hit.asset),
            Err(err) => {
                tracing::warn!(
                    target: "harness::evidence",
                    org_id = %organization_id,
                    asset = %hit.asset,
                    error = %err,
                    "failed to load target rows for EAS service landing"
                );
                skipped += 1;
                continue;
            }
        };
        if rows.is_empty() {
            skipped += 1;
            continue;
        }

        let port_entry = serde_json::json!({
            "port": hit.port,
            "proto": hit.transport,
            "service": hit.service_name.clone().unwrap_or_default(),
            "product": hit.service_product.clone().unwrap_or_default(),
            "version": hit.service_version.clone().unwrap_or_default(),
            "banner": hit.banner.clone().unwrap_or_default(),
            "state": "open",
            "source": source,
            "evidence_id": evidence_id,
        });
        let ports = serde_json::json!([port_entry]);
        for row in &rows {
            match golish_db::repo::targets::update_recon_extended_by_id(
                db_pool, row.id, "", "", "", None, "", "", "", &ports,
            )
            .await
            {
                Ok(()) => stored_targets += 1,
                Err(err) => tracing::warn!(
                    target: "harness::evidence",
                    target_id = %row.id,
                    asset = %hit.asset,
                    error = %err,
                    "failed to persist EAS service into targets.ports"
                ),
            }

            if let Some(name) = hit
                .service_product
                .as_deref()
                .or(hit.service_name.as_deref())
                .filter(|value| !value.trim().is_empty())
            {
                let evidence = serde_json::json!({
                    "source": source,
                    "evidence_id": evidence_id,
                    "raw": hit.raw_line,
                    "port": hit.port,
                    "transport": hit.transport,
                });
                match golish_db::repo::fingerprints::upsert(
                    db_pool,
                    row.id,
                    row.project_path.as_deref().or(project_path),
                    "service",
                    name,
                    hit.service_version.as_deref(),
                    0.85,
                    &evidence,
                    None,
                    source,
                )
                .await
                {
                    Ok(_) => stored_fingerprints += 1,
                    Err(err) => tracing::warn!(
                        target: "harness::evidence",
                        target_id = %row.id,
                        asset = %hit.asset,
                        error = %err,
                        "failed to persist EAS service fingerprint"
                    ),
                }
            }
        }

        if let Some(endpoint_ip) = endpoint_ip_for_hit(&hit.host, None, &rows) {
            if let Some(identity) = golish_db::repo::surface_identity::normalize_network_endpoint(
                &endpoint_ip,
                hit.port,
                &hit.transport,
            ) {
                match golish_db::repo::network_endpoints::upsert_by_identity(
                    db_pool,
                    Some(organization_id),
                    project_path,
                    &identity,
                    Some("open"),
                    hit.service_name.as_deref(),
                    hit.service_product.as_deref(),
                    hit.service_version.as_deref(),
                    hit.banner.as_deref(),
                    Some(service_name_implies_tls(
                        hit.service_name.as_deref().unwrap_or_default(),
                    )),
                    Some(source),
                    Some(0.85),
                    true,
                )
                .await
                {
                    Ok(_) => stored_endpoints += 1,
                    Err(err) => tracing::warn!(
                        target: "harness::evidence",
                        asset = %hit.asset,
                        port = hit.port,
                        error = %err,
                        "failed to persist EAS service into network_endpoints"
                    ),
                }
            }
        }
    }

    if stored_targets > 0 || stored_endpoints > 0 || stored_fingerprints > 0 || skipped > 0 {
        tracing::info!(
            target: "harness::evidence",
            org_id = %organization_id,
            source,
            stored_targets,
            stored_endpoints,
            stored_fingerprints,
            skipped,
            "background EAS services landed into target surface tables"
        );
    }
}

async fn persist_eas_web_probe_hits(
    db_pool: &sqlx::PgPool,
    organization_id: uuid::Uuid,
    project_path: Option<&str>,
    source: &str,
    evidence_id: i64,
    allowed_assets: &BTreeSet<String>,
    hits: &[WebProbeHit],
) {
    let mut stored_targets = 0usize;
    let mut stored_endpoints = 0usize;
    let mut stored_origins = 0usize;
    let mut stored_fingerprints = 0usize;
    let mut skipped = 0usize;

    for hit in hits {
        if !allowed_assets.contains(&hit.asset) {
            skipped += 1;
            continue;
        }
        let rows = match load_eas_landing_targets_for_asset(
            db_pool,
            organization_id,
            project_path,
            &hit.asset,
        )
        .await
        {
            Ok(rows) => prefer_exact_landing_targets(rows, &hit.asset),
            Err(err) => {
                tracing::warn!(
                    target: "harness::evidence",
                    org_id = %organization_id,
                    asset = %hit.asset,
                    error = %err,
                    "failed to load target rows for EAS web probe landing"
                );
                skipped += 1;
                continue;
            }
        };
        if rows.is_empty() {
            skipped += 1;
            continue;
        }

        let (service_product, service_version) = hit
            .webserver
            .as_deref()
            .map(golish_pentest::output_store::parse_server_version)
            .unwrap_or_else(|| (String::new(), None));
        let service_product = (!service_product.is_empty()).then_some(service_product);
        let port_entry = serde_json::json!({
            "port": hit.port,
            "proto": "tcp",
            "service": hit.scheme,
            "product": service_product.clone().unwrap_or_default(),
            "version": service_version.clone().unwrap_or_default(),
            "state": "open",
            "tls_detected": hit.scheme == "https",
            "url": hit.url,
            "http_title": hit.title.clone().unwrap_or_default(),
            "http_status": hit.status_code,
            "webserver": hit.webserver.clone().unwrap_or_default(),
            "content_type": hit.content_type.clone().unwrap_or_default(),
            "technologies": hit.technologies.clone(),
            "source": source,
            "evidence_id": evidence_id,
        });
        let ports = serde_json::json!([port_entry]);

        for row in &rows {
            let real_ip = if is_ip_target_type_label(&row.target_type) {
                ""
            } else {
                hit.ip.as_deref().unwrap_or("")
            };
            match golish_db::repo::targets::update_recon_extended_by_id(
                db_pool,
                row.id,
                real_ip,
                "",
                hit.title.as_deref().unwrap_or(""),
                hit.status_code,
                hit.webserver.as_deref().unwrap_or(""),
                "",
                hit.content_type.as_deref().unwrap_or(""),
                &ports,
            )
            .await
            {
                Ok(()) => stored_targets += 1,
                Err(err) => tracing::warn!(
                    target: "harness::evidence",
                    target_id = %row.id,
                    asset = %hit.asset,
                    error = %err,
                    "failed to persist EAS web probe into targets"
                ),
            }

            stored_fingerprints += persist_web_fingerprints(
                db_pool,
                row,
                project_path,
                source,
                evidence_id,
                hit,
                service_product.as_deref(),
                service_version.as_deref(),
            )
            .await;
        }

        let endpoint =
            if let Some(endpoint_ip) = endpoint_ip_for_hit(&hit.host, hit.ip.as_deref(), &rows) {
                golish_db::repo::surface_identity::normalize_network_endpoint(
                    &endpoint_ip,
                    hit.port,
                    "tcp",
                )
            } else {
                None
            };
        let mut endpoint_id = None;
        if let Some(identity) = endpoint {
            match golish_db::repo::network_endpoints::upsert_by_identity(
                db_pool,
                Some(organization_id),
                project_path,
                &identity,
                Some("open"),
                Some(hit.scheme.as_str()),
                service_product.as_deref(),
                service_version.as_deref(),
                hit.webserver.as_deref(),
                Some(hit.scheme == "https"),
                Some(source),
                Some(0.9),
                true,
            )
            .await
            {
                Ok(endpoint) => {
                    endpoint_id = Some(endpoint.id);
                    stored_endpoints += 1;
                }
                Err(err) => tracing::warn!(
                    target: "harness::evidence",
                    asset = %hit.asset,
                    url = %hit.url,
                    error = %err,
                    "failed to persist EAS web probe into network_endpoints"
                ),
            }
        }

        if let Some(origin_identity) =
            golish_db::repo::surface_identity::normalize_web_origin(&hit.url)
        {
            match golish_db::repo::web_origins::upsert_by_identity(
                db_pool,
                Some(organization_id),
                project_path,
                &origin_identity,
                Some(source),
                Some(0.9),
                true,
            )
            .await
            {
                Ok(origin) => {
                    stored_origins += 1;
                    let raw = serde_json::json!({
                        "source": source,
                        "evidence_id": evidence_id,
                        "raw": hit.raw_line,
                        "technologies": hit.technologies.clone(),
                    });
                    let capture_path = format!("background:eas:{source}:{}", hit.url);
                    let observed_ip = endpoint_ip_for_hit(&hit.host, hit.ip.as_deref(), &rows);
                    let input = golish_db::repo::web_origin_observations::NewWebOriginObservation {
                        organization_id: Some(organization_id),
                        project_path,
                        web_origin_id: origin.id,
                        network_endpoint_id: endpoint_id,
                        target_id: rows.first().map(|row| row.id),
                        observed_ip: observed_ip.as_deref(),
                        sni: (origin_identity.host_type == "domain")
                            .then_some(origin_identity.host.as_str()),
                        host_header: Some(origin_identity.host.as_str()),
                        status_code: hit.status_code,
                        title: hit.title.as_deref(),
                        final_url: Some(hit.url.as_str()),
                        redirect_chain: None,
                        body_hash: None,
                        favicon_hash: None,
                        screenshot_path: None,
                        capture_path: Some(capture_path.as_str()),
                        confidence: Some(0.9),
                        source: Some(source),
                        raw: Some(&raw),
                    };
                    if let Err(err) =
                        golish_db::repo::web_origin_observations::upsert_observation_dedupe(
                            db_pool, &input,
                        )
                        .await
                    {
                        tracing::warn!(
                            target: "harness::evidence",
                            asset = %hit.asset,
                            url = %hit.url,
                            error = %err,
                            "failed to persist EAS web origin observation"
                        );
                    }
                }
                Err(err) => tracing::warn!(
                    target: "harness::evidence",
                    asset = %hit.asset,
                    url = %hit.url,
                    error = %err,
                    "failed to persist EAS web origin"
                ),
            }
        }
    }

    if stored_targets > 0
        || stored_endpoints > 0
        || stored_origins > 0
        || stored_fingerprints > 0
        || skipped > 0
    {
        tracing::info!(
            target: "harness::evidence",
            org_id = %organization_id,
            source,
            stored_targets,
            stored_endpoints,
            stored_origins,
            stored_fingerprints,
            skipped,
            "background EAS web probes landed into target surface tables"
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn persist_web_fingerprints(
    db_pool: &sqlx::PgPool,
    row: &EasLandingTargetRow,
    project_path: Option<&str>,
    source: &str,
    evidence_id: i64,
    hit: &WebProbeHit,
    service_product: Option<&str>,
    service_version: Option<&str>,
) -> usize {
    let mut stored = 0usize;
    let evidence = serde_json::json!({
        "source": source,
        "evidence_id": evidence_id,
        "url": hit.url,
        "status_code": hit.status_code,
        "raw": hit.raw_line,
    });
    if let Some(name) = service_product.filter(|name| !name.trim().is_empty()) {
        match golish_db::repo::fingerprints::upsert(
            db_pool,
            row.id,
            row.project_path.as_deref().or(project_path),
            "web_server",
            name,
            service_version,
            0.9,
            &evidence,
            None,
            source,
        )
        .await
        {
            Ok(_) => stored += 1,
            Err(err) => tracing::warn!(
                target: "harness::evidence",
                target_id = %row.id,
                url = %hit.url,
                error = %err,
                "failed to persist EAS web server fingerprint"
            ),
        }
    }
    for tech in &hit.technologies {
        let tech = tech.trim();
        if tech.is_empty() || service_product.is_some_and(|name| name.eq_ignore_ascii_case(tech)) {
            continue;
        }
        let (name, version) = golish_pentest::output_store::parse_server_version(tech);
        if name.is_empty() {
            continue;
        }
        match golish_db::repo::fingerprints::upsert(
            db_pool,
            row.id,
            row.project_path.as_deref().or(project_path),
            "technology",
            &name,
            version.as_deref(),
            0.75,
            &evidence,
            None,
            source,
        )
        .await
        {
            Ok(_) => stored += 1,
            Err(err) => tracing::warn!(
                target: "harness::evidence",
                target_id = %row.id,
                url = %hit.url,
                technology = %name,
                error = %err,
                "failed to persist EAS technology fingerprint"
            ),
        }
    }
    stored
}

/// Dead-asset ongoing marking (design 2026-07-02-dead-asset-liveness-state §4):
/// an EAS liveness probe covered `asset` and found it not alive → stamp the
/// matching in-scope target(s) `liveness_state='dead'`. Reuses the same landing
/// resolver as the alive stamps so the two agree on which target row an asset
/// maps to; the DB write is guarded (`mark_dead_if_no_signal_by_id`) so it only
/// fires while the row genuinely has no alive signal — a host naabu proved has
/// open ports (stamped 'alive') is left untouched regardless of landing order.
async fn mark_eas_liveness_dead_asset(
    db_pool: &sqlx::PgPool,
    organization_id: uuid::Uuid,
    project_path: Option<&str>,
    asset: &str,
) {
    let rows =
        match load_eas_landing_targets_for_asset(db_pool, organization_id, project_path, asset)
            .await
        {
            Ok(rows) => prefer_exact_landing_targets(rows, asset),
            Err(err) => {
                tracing::warn!(
                    target: "harness::evidence",
                    org_id = %organization_id,
                    asset = %asset,
                    error = %err,
                    "failed to load target rows for EAS dead-asset marking"
                );
                return;
            }
        };
    for row in &rows {
        match golish_db::repo::targets::mark_dead_if_no_signal_by_id(db_pool, row.id).await {
            Ok(marked) if marked > 0 => tracing::info!(
                target: "harness::evidence",
                org_id = %organization_id,
                asset = %asset,
                target_id = %row.id,
                "marked EAS-probed asset dead (no liveness signal)"
            ),
            Ok(_) => {}
            Err(err) => tracing::warn!(
                target: "harness::evidence",
                org_id = %organization_id,
                asset = %asset,
                target_id = %row.id,
                error = %err,
                "failed to mark EAS-probed asset dead"
            ),
        }
    }
}

async fn load_eas_landing_targets_for_asset(
    db_pool: &sqlx::PgPool,
    organization_id: uuid::Uuid,
    project_path: Option<&str>,
    asset: &str,
) -> Result<Vec<EasLandingTargetRow>, sqlx::Error> {
    sqlx::query_as::<_, EasLandingTargetRow>(
        r#"
        SELECT id, value, target_type::text AS target_type, real_ip, project_path
        FROM targets
        WHERE scope::text = 'in'
          AND organization_id = $1
          AND ($2::text IS NULL OR project_path = $2 OR project_path = '')
          AND (
            lower(value) = lower($3)
            OR EXISTS (
                SELECT 1
                FROM regexp_split_to_table(coalesce(real_ip, ''), '[,;[:space:]]+') AS ip
                WHERE lower(ip) = lower($3)
            )
          )
        ORDER BY
          CASE WHEN lower(value) = lower($3) THEN 0 ELSE 1 END,
          CASE WHEN target_type::text = 'ip' THEN 0 ELSE 1 END,
          updated_at DESC
        "#,
    )
    .bind(organization_id)
    .bind(project_path)
    .bind(asset)
    .fetch_all(db_pool)
    .await
}

fn prefer_exact_landing_targets(
    rows: Vec<EasLandingTargetRow>,
    asset: &str,
) -> Vec<EasLandingTargetRow> {
    let exact: Vec<EasLandingTargetRow> = rows
        .iter()
        .filter(|row| landing_row_value_matches_asset(row, asset))
        .cloned()
        .collect();
    if exact.is_empty() {
        rows
    } else {
        exact
    }
}

fn landing_row_value_matches_asset(row: &EasLandingTargetRow, asset: &str) -> bool {
    row.value.eq_ignore_ascii_case(asset)
        || golish_pentest_domain::canonical_asset_key(&row.value)
            .map(|key| key.key.eq_ignore_ascii_case(asset))
            .unwrap_or(false)
}

fn endpoint_ip_for_hit(
    host: &str,
    explicit_ip: Option<&str>,
    rows: &[EasLandingTargetRow],
) -> Option<String> {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Some(host.to_string());
    }
    if let Some(ip) = explicit_ip
        .map(str::trim)
        .filter(|ip| ip.parse::<std::net::IpAddr>().is_ok())
    {
        return Some(ip.to_string());
    }
    rows.iter()
        .flat_map(|row| split_real_ip_values(&row.real_ip))
        .find(|ip| ip.parse::<std::net::IpAddr>().is_ok())
}

fn split_real_ip_values(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split([',', ';', '\n', '\r', '\t', ' '])
        .map(str::trim)
        .filter(|ip| !ip.is_empty())
        .map(str::to_string)
}

fn is_ip_target_type_label(target_type: &str) -> bool {
    matches!(target_type, "ip" | "ipv4" | "ip_address" | "cidr")
}

fn service_name_implies_tls(service_name: &str) -> bool {
    let service = service_name.to_ascii_lowercase();
    service.contains("https") || service.contains("ssl") || service.contains("tls")
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

fn open_port_counts(tool: &str, stdout: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for hit in open_port_hits(tool, stdout) {
        *counts.entry(hit.asset).or_insert(0) += 1;
    }
    counts
}

fn should_skip_empty_port_outcome(count: usize, existing_open_ports: Option<&Vec<u16>>) -> bool {
    count == 0 && existing_open_ports.is_some_and(|ports| !ports.is_empty())
}

fn open_port_hits(tool: &str, stdout: &str) -> Vec<OpenPortHit> {
    let mut hits = Vec::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let hit = match tool {
            "naabu" => host_port_from_naabu_line(line),
            "masscan" => host_port_from_masscan_line(line),
            _ => None,
        };
        let Some((host, port, transport)) = hit else {
            continue;
        };
        if let Some(asset) = golish_pentest_domain::canonical_asset_key(host).map(|key| key.key) {
            hits.push(OpenPortHit {
                host: host.to_string(),
                asset,
                port,
                transport: transport.to_string(),
            });
        }
    }
    hits
}

fn host_port_from_naabu_line(line: &str) -> Option<(&str, i32, &'static str)> {
    let (host, port) = line.rsplit_once(':')?;
    let port = port.parse::<i32>().ok()?;
    (1..=65535).contains(&port).then_some((host, port, "tcp"))
}

fn host_port_from_masscan_line(line: &str) -> Option<(&str, i32, &str)> {
    line.strip_prefix("Discovered open port ")
        .and_then(|rest| rest.split_once(" on "))
        .and_then(|(port_part, host)| {
            let (port, transport) = port_part.split_once('/')?;
            let port = port.trim().parse::<i32>().ok()?;
            let transport = transport.trim();
            ((1..=65535).contains(&port)
                && matches!(transport, "tcp" | "udp")
                && !host.trim().is_empty())
            .then_some((host.trim(), port, transport))
        })
}

fn httpx_probe_hits(stdout: &str) -> Vec<WebProbeHit> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            if line.starts_with('{') {
                serde_json::from_str::<serde_json::Value>(line)
                    .ok()
                    .and_then(|value| httpx_probe_hit_from_json(&value, line))
            } else if line.starts_with("http://") || line.starts_with("https://") {
                web_probe_hit_from_url(line, None, None, None, None, Vec::new(), None, line)
            } else {
                None
            }
        })
        .collect()
}

fn httpx_probe_hit_from_json(value: &serde_json::Value, raw_line: &str) -> Option<WebProbeHit> {
    let url = value.get("url")?.as_str()?;
    let status_code = value
        .get("status_code")
        .and_then(|value| value.as_i64())
        .and_then(|value| i32::try_from(value).ok());
    let title = json_string_field(value, "title");
    let webserver = json_string_field(value, "webserver");
    let content_type = json_string_field(value, "content_type");
    let technologies = value
        .get("tech")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let ip = json_string_field(value, "host_ip")
        .or_else(|| json_string_field(value, "ip"))
        .or_else(|| {
            value
                .get("a")
                .and_then(|value| value.as_array())
                .and_then(|items| items.iter().find_map(|item| item.as_str()))
                .map(str::to_string)
        });

    web_probe_hit_from_url(
        url,
        status_code,
        title,
        webserver,
        content_type,
        technologies,
        ip,
        raw_line,
    )
}

#[allow(clippy::too_many_arguments)]
fn web_probe_hit_from_url(
    url: &str,
    status_code: Option<i32>,
    title: Option<String>,
    webserver: Option<String>,
    content_type: Option<String>,
    technologies: Vec<String>,
    ip: Option<String>,
    raw_line: &str,
) -> Option<WebProbeHit> {
    let origin = golish_db::repo::surface_identity::normalize_web_origin(url)?;
    let asset = golish_pentest_domain::canonical_asset_key(&origin.host).map(|key| key.key)?;
    Some(WebProbeHit {
        url: url.to_string(),
        host: origin.host,
        asset,
        scheme: origin.scheme,
        port: origin.port,
        status_code,
        title,
        webserver,
        content_type,
        technologies: dedupe_strings(technologies),
        ip,
        raw_line: raw_line.to_string(),
    })
}

fn whatweb_probe_hits(stdout: &str) -> Vec<WebProbeHit> {
    stdout
        .lines()
        .filter_map(whatweb_probe_hit_from_line)
        .collect()
}

fn whatweb_probe_hit_from_line(line: &str) -> Option<WebProbeHit> {
    let clean = strip_ansi_codes(line);
    let url_start = clean.find("http://").or_else(|| clean.find("https://"))?;
    let after_url_start = &clean[url_start..];
    let url_end = after_url_start
        .find(char::is_whitespace)
        .unwrap_or(after_url_start.len());
    let url = &after_url_start[..url_end];
    let rest = after_url_start[url_end..].trim_start();
    let status_code = bracketed_status_code(rest);
    let after_status = rest
        .find(']')
        .map(|idx| rest[idx + 1..].trim_start())
        .unwrap_or(rest);
    let parsed = parse_whatweb_plugins(after_status);
    web_probe_hit_from_url(
        url,
        status_code,
        parsed.title,
        parsed.webserver,
        None,
        parsed.technologies,
        None,
        clean.trim(),
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WhatWebPlugins {
    webserver: Option<String>,
    title: Option<String>,
    technologies: Vec<String>,
}

fn parse_whatweb_plugins(input: &str) -> WhatWebPlugins {
    let mut parsed = WhatWebPlugins::default();
    let mut technologies = Vec::new();
    for segment in input
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (name, value) = bracket_plugin(segment);
        match name.to_ascii_lowercase().as_str() {
            "httpserver" => {
                if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
                    parsed.webserver = Some(value.to_string());
                    technologies.push(value.to_string());
                }
            }
            "poweredby" => {
                if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
                    technologies.push(value.to_string());
                }
            }
            "title" => {
                parsed.title = value.filter(|value| !value.is_empty());
            }
            "ip" | "country" | "content-language" | "uncommonheaders" => {}
            _ => {
                technologies.push(value.unwrap_or(name));
            }
        }
    }
    parsed.technologies = dedupe_strings(technologies);
    parsed
}

fn bracket_plugin(segment: &str) -> (String, Option<String>) {
    let Some(start) = segment.find('[') else {
        return (segment.trim().to_string(), None);
    };
    let name = segment[..start].trim().to_string();
    let value = segment[start + 1..]
        .find(']')
        .map(|end| segment[start + 1..start + 1 + end].trim().to_string());
    (name, value)
}

fn bracketed_status_code(input: &str) -> Option<i32> {
    let open = input.find('[')?;
    let inner = &input[open + 1..];
    let code: String = inner.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if code.len() == 3 {
        code.parse().ok()
    } else {
        None
    }
}

fn nmap_service_hits(stdout: &str) -> Vec<ServiceHit> {
    let mut hits = Vec::new();
    let mut current_host: Option<String> = None;
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(host) = nmap_scan_report_host(line) {
            current_host = Some(host);
            continue;
        }
        let Some(host) = current_host.as_deref() else {
            continue;
        };
        let Some(hit) = nmap_service_hit_from_line(host, line) else {
            continue;
        };
        hits.push(hit);
    }
    hits
}

fn nmap_scan_report_host(line: &str) -> Option<String> {
    let rest = line.strip_prefix("Nmap scan report for ")?;
    if let Some(open) = rest.rfind('(') {
        if rest.ends_with(')') {
            return Some(rest[open + 1..rest.len() - 1].trim().to_string());
        }
    }
    rest.split_whitespace().next().map(str::to_string)
}

fn nmap_service_hit_from_line(host: &str, line: &str) -> Option<ServiceHit> {
    let mut parts = line.split_whitespace();
    let port_proto = parts.next()?;
    let state = parts.next()?;
    if state != "open" {
        return None;
    }
    let service = parts.next()?.to_string();
    let (port, transport) = port_proto.split_once('/')?;
    let port = port.parse::<i32>().ok()?;
    if !(1..=65535).contains(&port) || !matches!(transport, "tcp" | "udp") {
        return None;
    }
    let banner = parts.collect::<Vec<_>>().join(" ");
    let (product, version) = if banner.is_empty() {
        (None, None)
    } else {
        let (name, version) = golish_pentest::output_store::parse_server_version(&banner);
        (Some(name), version)
    };
    let asset = golish_pentest_domain::canonical_asset_key(host).map(|key| key.key)?;
    Some(ServiceHit {
        host: host.to_string(),
        asset,
        port,
        transport: transport.to_string(),
        service_name: Some(service),
        service_product: product,
        service_version: version,
        banner: (!banner.is_empty()).then_some(banner),
        raw_line: line.to_string(),
    })
}

fn strip_ansi_codes(input: &str) -> String {
    regex::Regex::new(r"\x1b\[[0-9;]*[A-Za-z]")
        .expect("valid ansi escape regex")
        .replace_all(input, "")
        .into_owned()
}

fn json_string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn dedupe_strings(items: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let key = item.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(item.to_string());
        }
    }
    out
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
        "nuclei" => &["-l", "-list"],
        "sqlmap" => &["-m", "--bulk-file"],
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
    let repo_provider = std::sync::Arc::new(crate::ai::db_bridge::GolishDbRepoProvider::new(
        state.db_pool.clone(),
    ));
    let runtime_memory: std::sync::Arc<dyn golish_agent_kit::db_traits::RuntimeMemoryRepository> =
        repo_provider.clone();
    let chain_persistence: std::sync::Arc<dyn golish_sub_agents::SubAgentChainPersistence> =
        std::sync::Arc::new(
            crate::ai::tracking_bridge::PgChainPersistence::new(state.db_pool.clone())
                .with_runtime_memory_repository(runtime_memory.clone()),
        );
    let ready_gate = crate::ai::tracking_bridge::CoreDbReadyGate(state.db_ready.clone());
    bridge.set_db_backend(tracking_backend, ready_gate, chain_persistence);
    bridge.set_runtime_memory_repository(runtime_memory);

    let graph_backend = std::sync::Arc::new(crate::ai::graph_bridge::GraphClientBackend::new(
        state.db_pool.clone(),
    ));
    let db_repo: std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider> = repo_provider;
    let knowledge_context: std::sync::Arc<dyn golish_memory_app::ContextPackProvider> =
        std::sync::Arc::new(
            crate::ai::db_bridge::knowledge_context::PgKnowledgeContextAdapter::new(
                state.db_pool.clone(),
            )
            .expect("fixed local-only ContextPack adapter configuration is valid"),
        );

    bridge.apply_backends(golish_agent_bridge::BridgeBackends {
        indexer: Some(state.indexer_state.clone()),
        sidecar: Some(sidecar_backend),
        settings: Some(state.settings_manager.clone()),
        graph: Some(graph_backend),
        db_repo: Some(db_repo),
        knowledge_memory: Some(state.knowledge_memory.clone()),
        knowledge_context: Some(knowledge_context),
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
                    golish_pentest::output_store::StoreContext {
                        organization_id,
                        ..Default::default()
                    },
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
        // One-shot LLM handle for the JS collect/extract tools' AI-assisted
        // recipe/extraction passes (设计 2026-06-30-jsapi-ai-tools). Fixed to
        // DeepSeek via app settings; tools degrade to deterministic when the key
        // is absent.
        let llm_one_shot: Option<std::sync::Arc<dyn golish_app_core::ports::llm::LlmOneShot>> =
            Some(std::sync::Arc::new(
                crate::ai::llm_one_shot::SettingsLlmOneShot::new(state.settings_manager.clone()),
            ));
        let bridge_tools = state.pentest_tool_factory.create_bridge_tools(
            state.db_pool.clone(),
            state.pentest_config_manager.clone(),
            app_handle,
            bridge.harness_active_org_id_handle(),
            llm_one_shot,
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
        let provider = std::sync::Arc::new(crate::ai::db_bridge::GolishDbRepoProvider::new(
            state.db_pool.clone(),
        ));
        let evidence_repo: std::sync::Arc<dyn crate::ai::harness_submit_tool::EvidenceLedgerQuery> =
            provider.clone();
        let runtime_memory_repo: std::sync::Arc<
            dyn golish_agent_kit::db_traits::RuntimeMemoryRepository,
        > = provider.clone();
        let candidate_submit_tool = std::sync::Arc::new(
            crate::ai::candidate_submit_tool::SubmitCandidateAttemptTool::new(
                runtime_memory_repo.clone(),
            ),
        );
        let mut submit_tool = crate::ai::harness_submit_tool::SubmitStageDeliverableTool::new(
            bridge.harness_active_stage_handle(),
            bridge.harness_last_deliverable_handle(),
        )
        .with_evidence_repo(evidence_repo)
        .with_runtime_memory_repository(runtime_memory_repo)
        .with_captured_submission_sink(bridge.harness_captured_submission_handle())
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
        tracing::info!("[harness] Registered tool: submit_candidate_attempt");
        registry.register_tool(candidate_submit_tool);
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
            processing_claim: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[test]
    fn completion_processing_claim_is_shared_across_broadcast_clones() {
        let completion = sample_completion();
        let overlapping_generation = completion.clone();

        assert!(completion.try_claim_processing());
        assert!(!overlapping_generation.try_claim_processing());
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
    fn batch_vuln_input_file_is_recovered_from_nuclei_l_flag() {
        let cmd = r#"nuclei -json -silent -tags sqli,xss -l .golish/tool-inputs/vuln-targets.txt"#;
        assert_eq!(
            batch_input_file_from_command(cmd, Some("/tmp/ws"), "nuclei")
                .unwrap()
                .to_string_lossy(),
            "/tmp/ws/.golish/tool-inputs/vuln-targets.txt"
        );
    }

    #[test]
    fn batch_vuln_input_file_is_recovered_from_sqlmap_m_flag() {
        let cmd = r#"sqlmap -m .golish/tool-inputs/sqlmap-targets.txt --batch --level 1"#;
        assert_eq!(
            batch_input_file_from_command(cmd, Some("/tmp/ws"), "sqlmap")
                .unwrap()
                .to_string_lossy(),
            "/tmp/ws/.golish/tool-inputs/sqlmap-targets.txt"
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
    fn empty_port_outcome_is_skipped_when_db_still_has_open_ports() {
        assert!(should_skip_empty_port_outcome(0, Some(&vec![82])));
        assert!(!should_skip_empty_port_outcome(1, Some(&vec![82])));
        assert!(!should_skip_empty_port_outcome(0, Some(&Vec::new())));
        assert!(!should_skip_empty_port_outcome(0, None));
    }

    #[test]
    fn open_port_hits_preserve_batch_ports() {
        let hits = open_port_hits("naabu", "59.82.14.249:80\n59.82.14.249:443\nnoise\n");

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].asset, "59.82.14.249");
        assert_eq!(hits[0].port, 80);
        assert_eq!(hits[1].port, 443);
        assert_eq!(hits[1].transport, "tcp");
    }

    #[test]
    fn masscan_open_port_hits_keep_transport() {
        let hits = open_port_hits(
            "masscan",
            "Discovered open port 53/udp on 1.2.3.4\nDiscovered open port 443/tcp on 1.2.3.4\n",
        );

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].port, 53);
        assert_eq!(hits[0].transport, "udp");
        assert_eq!(hits[1].port, 443);
        assert_eq!(hits[1].transport, "tcp");
    }

    #[test]
    fn whatweb_probe_hits_strip_ansi_and_extract_web_facts() {
        let stdout = "\u{1b}[1m\u{1b}[34mhttp://59.82.14.249\u{1b}[0m [404 Not Found] \u{1b}[1mHTTPServer\u{1b}[0m[\u{1b}[36mTengine\u{1b}[0m], \u{1b}[1mPoweredBy\u{1b}[0m[Tengine/2], \u{1b}[1mTitle\u{1b}[0m[\u{1b}[33m404 Not Found\u{1b}[0m]";
        let hits = whatweb_probe_hits(stdout);

        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.asset, "59.82.14.249");
        assert_eq!(hit.scheme, "http");
        assert_eq!(hit.port, 80);
        assert_eq!(hit.status_code, Some(404));
        assert_eq!(hit.webserver.as_deref(), Some("Tengine"));
        assert!(hit.technologies.iter().any(|tech| tech == "Tengine/2"));
    }

    #[test]
    fn httpx_probe_hits_parse_jsonl_metadata() {
        let stdout = r#"{"url":"https://example.com","host":"example.com","host_ip":"1.2.3.4","port":"443","title":"hello","scheme":"https","webserver":"nginx/1.24","content_type":"text/html","status_code":200,"tech":["Nginx","React"]}"#;
        let hits = httpx_probe_hits(stdout);

        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.asset, "example.com");
        assert_eq!(hit.ip.as_deref(), Some("1.2.3.4"));
        assert_eq!(hit.port, 443);
        assert_eq!(hit.status_code, Some(200));
        assert_eq!(hit.webserver.as_deref(), Some("nginx/1.24"));
        assert_eq!(hit.technologies, vec!["Nginx", "React"]);
    }

    #[test]
    fn nmap_service_hits_parse_open_service_rows() {
        let stdout = "Nmap scan report for scanme.example.com (45.33.32.156)\nPORT    STATE SERVICE VERSION\n80/tcp  open  http    Apache httpd 2.4.7\n443/tcp closed https\n";
        let hits = nmap_service_hits(stdout);

        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.asset, "45.33.32.156");
        assert_eq!(hit.port, 80);
        assert_eq!(hit.service_name.as_deref(), Some("http"));
        assert_eq!(hit.service_product.as_deref(), Some("Apache httpd"));
        assert_eq!(hit.service_version.as_deref(), Some("2.4.7"));
    }

    #[test]
    fn nuclei_covered_techniques_from_tags_flag() {
        use golish_agent_kit::harness::wstg_mapping::{WSTG_SQLI, WSTG_XSS};
        let covered = nuclei_covered_techniques("nuclei -tags sqli,xss,unknown -u https://a.com");
        assert!(covered.contains(WSTG_SQLI));
        assert!(covered.contains(WSTG_XSS));
        // unknown tag fail-closed: not counted as covered.
        assert_eq!(covered.len(), 2);
        // equals form.
        let eq = nuclei_covered_techniques("nuclei -tags=cve -l hosts.txt");
        assert!(eq.contains(golish_agent_kit::harness::wstg_mapping::GOLISH_NDAY));
        // no -tags => empty covered set (only real hits credit found).
        assert!(!nuclei_command_targets("nuclei -u https://a.com").is_empty());
        assert!(nuclei_covered_techniques("nuclei -u https://a.com").is_empty());
    }

    #[test]
    fn nuclei_command_targets_split_and_flags() {
        let t = nuclei_command_targets("nuclei -u https://a.com,https://b.com -tags sqli");
        assert_eq!(
            t,
            vec!["https://a.com".to_string(), "https://b.com".to_string()]
        );
        assert!(nuclei_command_targets("nuclei -l hosts.txt").is_empty());
    }

    #[test]
    fn nuclei_wstg_hits_aggregate_per_asset_from_tags() {
        use golish_agent_kit::harness::wstg_mapping::{GOLISH_NDAY, WSTG_EXPOSURE_CONFIG};
        let stdout = concat!(
            r#"{"template-id":"git-config","matched-at":"https://example.com/.git/config","info":{"name":"Git Config","tags":["exposure","config"]}}"#,
            "\n",
            r#"{"template-id":"CVE-2021-1","matched-at":"https://example.com/x","info":{"tags":"cve"}}"#,
            "\n",
            "noise line that is not json\n",
        );
        let hits = nuclei_wstg_hits(stdout);
        let asset = hits.get("example.com").expect("example.com has hits");
        // exposure+config both map to CONF-05 → counted twice on one line.
        assert_eq!(asset.get(WSTG_EXPOSURE_CONFIG).copied(), Some(1));
        assert_eq!(asset.get(GOLISH_NDAY).copied(), Some(1));
    }

    #[test]
    fn nuclei_wstg_hits_skip_untagged_or_unmapped() {
        // matched-at present but tags empty/unmapped → no credit (fail-closed).
        let stdout = r#"{"matched-at":"https://a.com/p","info":{"tags":["network","tech"]}}"#;
        assert!(nuclei_wstg_hits(stdout).is_empty());
    }

    #[test]
    fn vuln_scan_covered_techniques_dispatches_per_tool() {
        use golish_agent_kit::harness::wstg_mapping::{GOLISH_NDAY, WSTG_SQLI};
        // sqlmap = SQLi tool → always attempts WSTG-INPV-05.
        let sqlmap =
            vuln_scan_covered_techniques("sqlmap", "sqlmap -u https://a.com/?id=1 --batch");
        assert_eq!(sqlmap.iter().copied().collect::<Vec<_>>(), vec![WSTG_SQLI]);
        // wpscan = WordPress n-day tool → always attempts GOLISH-NDAY.
        let wpscan =
            vuln_scan_covered_techniques("wpscan", "wpscan --url https://a.com --format json");
        assert_eq!(
            wpscan.iter().copied().collect::<Vec<_>>(),
            vec![GOLISH_NDAY]
        );
        // nuclei defers to tag parsing.
        assert!(vuln_scan_covered_techniques("nuclei", "nuclei -u https://a.com").is_empty());
        // unknown tool → empty (fail-closed).
        assert!(vuln_scan_covered_techniques("nikto", "nikto -h https://a.com").is_empty());
    }

    #[test]
    fn vuln_scan_command_targets_reads_url_flags() {
        let sqlmap = vuln_scan_command_targets("sqlmap", "sqlmap -u https://a.com/?id=1 --batch");
        assert_eq!(sqlmap, vec!["https://a.com/?id=1".to_string()]);
        let wpscan =
            vuln_scan_command_targets("wpscan", "wpscan --url https://a.com,https://b.com");
        assert_eq!(
            wpscan,
            vec!["https://a.com".to_string(), "https://b.com".to_string()]
        );
    }

    #[test]
    fn sqlmap_confirmed_injection_credits_sqli_found() {
        use golish_agent_kit::harness::wstg_mapping::WSTG_SQLI;
        let stdout = "[INFO] testing connection\nsqlmap identified the following injection point(s) with a total of 42 HTTP(s) requests:\n---\nParameter: id (GET)";
        let targets = vec!["https://a.com/?id=1".to_string()];
        let hits = vuln_scan_wstg_hits("sqlmap", &targets, stdout);
        // canonical_asset_key normalizes the URL target to its host key.
        assert_eq!(
            hits.get("a.com").and_then(|h| h.get(WSTG_SQLI)).copied(),
            Some(1)
        );
    }

    #[test]
    fn sqlmap_no_injection_is_empty_hits() {
        let stdout = "[INFO] testing connection\n[WARNING] all tested parameters do not appear to be injectable";
        let targets = vec!["https://a.com/?id=1".to_string()];
        assert!(vuln_scan_wstg_hits("sqlmap", &targets, stdout).is_empty());
    }

    #[test]
    fn wpscan_json_vulnerabilities_credit_nday_found() {
        use golish_agent_kit::harness::wstg_mapping::GOLISH_NDAY;
        let stdout = r#"{"version":{"number":"5.0","vulnerabilities":[{"title":"WP < 5.1 CSRF","references":{"cve":["2019-0000"]}}]},"plugins":{}}"#;
        let targets = vec!["https://a.com".to_string()];
        let hits = vuln_scan_wstg_hits("wpscan", &targets, stdout);
        assert_eq!(
            hits.get("a.com").and_then(|h| h.get(GOLISH_NDAY)).copied(),
            Some(1)
        );
    }

    #[test]
    fn wpscan_json_no_vulnerabilities_is_empty_hits() {
        let stdout = r#"{"version":{"number":"6.4","vulnerabilities":[]},"plugins":{"akismet":{"vulnerabilities":[]}}}"#;
        let targets = vec!["https://a.com".to_string()];
        assert!(vuln_scan_wstg_hits("wpscan", &targets, stdout).is_empty());
    }

    #[test]
    fn wpscan_non_json_output_is_fail_closed_empty() {
        // No --format json → free text → do not guess (fail-closed).
        let stdout = "[+] URL: https://a.com/\n[i] Plugin(s) Identified:\n [+] akismet";
        let targets = vec!["https://a.com".to_string()];
        assert!(vuln_scan_wstg_hits("wpscan", &targets, stdout).is_empty());
    }

    #[test]
    fn command_flag_value_space_and_equals_forms() {
        assert_eq!(
            command_flag_value("nuclei -tags sqli,xss -u x", &["-tags", "-tag"]).as_deref(),
            Some("sqli,xss")
        );
        assert_eq!(
            command_flag_value("nuclei -tags=cve -u x", &["-tags"]).as_deref(),
            Some("cve")
        );
        assert_eq!(command_flag_value("nuclei -u x", &["-tags"]), None);
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

    #[test]
    fn per_session_bridge_config_only_injects_memory_handle() {
        let source = include_str!("bridge_config.rs");
        assert!(source.contains("knowledge_memory: Some(state.knowledge_memory.clone())"));
        let forbidden_start = ["memory_supervisor", ".start("].concat();
        let forbidden_new = ["KnowledgeMemoryRuntime", "::new("].concat();
        let forbidden_settings = ["KnowledgeMemoryRuntime", "::from_settings("].concat();
        assert!(!source.contains(&forbidden_start));
        assert!(!source.contains(&forbidden_new));
        assert!(!source.contains(&forbidden_settings));
    }
}
