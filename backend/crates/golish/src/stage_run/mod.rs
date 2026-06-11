//! 方案 2 · headless single/range stage runner (`golish --stage-run`).
//!
//! Boots the real backend **without the GUI** (embedded Postgres + the full
//! pentest tool surface + a real LLM), runs one harness stage — or a
//! `--from`..=`--to` slice of the stage DAG — drives any scoping HITL
//! automatically (`--auto-approve`), prints a structured report (gate
//! PASS/BLOCK + reasons, tools called, evidence booked), and exits. The run's
//! full `transcript.json` is written exactly like a GUI run, so
//! `golish --replay <session>` (and the GUI) can replay the same timeline.
//!
//! See `docs/design/2026-06-06-headless-single-stage-runner.md`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::sync::mpsc;

use golish_agent_kit::harness::{
    active_profile_id, base_operation_graph, load_embedded_profile, StageKind,
};
use golish_core::agent_mode::AgentMode;
use golish_core::events::{AiEvent, HarnessTraceKind};
use golish_core::hitl::ApprovalDecision;
use golish_core::runtime::{GolishRuntime, RuntimeEvent};

use crate::ai::agent_bridge::AgentBridge;
use crate::cli::Args;
use crate::runtime::CliRuntime;

/// Resolve the `(entry_stage, allowlist)` for a `--stage-run` slice against the
/// profile-projected DAG. `entry_stage` is the slice's entry point (where the
/// `operation_state` cursor begins); `allowlist` is fed to
/// [`TaskOrchestrator::set_stage_allowlist`](golish_agent_kit::task_orchestrator::TaskOrchestrator::set_stage_allowlist).
fn resolve_slice(
    profile_id: &str,
    from: Option<StageKind>,
    to: StageKind,
) -> Result<(StageKind, HashSet<StageKind>)> {
    let graph = base_operation_graph().map_err(|e| anyhow!("load operation graph: {e}"))?;
    let profile = load_embedded_profile(profile_id)
        .map_err(|e| anyhow!("load profile {profile_id}: {e}"))?
        .ok_or_else(|| anyhow!("unknown harness profile: {profile_id}"))?;
    let allowed = profile.allowed_stage_set();
    let dag = graph.project(&allowed);
    let allowlist = dag
        .slice(from, to)
        .map_err(|e| anyhow!("stage slice ({profile_id}): {e}"))?;
    // Entry = the sliced sub-DAG's entry point (the cursor start).
    let sliced_allowed: HashSet<StageKind> = allowed.intersection(&allowlist).copied().collect();
    let entry = graph
        .project(&sliced_allowed)
        .entry_points()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("sliced DAG has no entry point"))?;
    Ok((entry, allowlist))
}

/// Parse `--from`/`--to`/`--only` into `(from, to)` stages.
fn resolve_from_to(args: &Args) -> Result<(Option<StageKind>, StageKind)> {
    let parse = |s: &str| StageKind::try_parse(s).ok_or_else(|| anyhow!("unknown stage: {s}"));
    if let Some(only) = args.only.as_deref() {
        let s = parse(only)?;
        return Ok((Some(s), s));
    }
    let to = args
        .to
        .as_deref()
        .ok_or_else(|| anyhow!("--stage-run requires --to <stage> (or --only <stage>)"))?;
    let to = parse(to)?;
    let from = match args.from.as_deref() {
        Some(f) => Some(parse(f)?),
        None => None,
    };
    Ok((from, to))
}

/// Headless entry point for `golish --stage-run`.
pub async fn run(args: Args) -> Result<()> {
    // 1) Resolve profile + stage slice up front (cheap, fails fast on bad input).
    let profile_id = args
        .profile
        .clone()
        .unwrap_or_else(|| active_profile_id().to_string());
    let (from_opt, to_stage) = resolve_from_to(&args)?;
    let (entry_stage, allowlist) = resolve_slice(&profile_id, from_opt, to_stage)?;

    let mut slice_sorted: Vec<&str> = allowlist.iter().map(|s| s.as_str()).collect();
    slice_sorted.sort_unstable();
    eprintln!(
        "[stage-run] profile={profile_id} entry={} to={} slice={slice_sorted:?} auto_approve={}",
        entry_stage.as_str(),
        to_stage.as_str(),
        args.auto_approve
    );

    // 2) Settings + tracing (so backend.log captures the run like the GUI does).
    let workspace = args.resolve_workspace().context("resolve workspace")?;
    let settings_manager = Arc::new(
        crate::settings::SettingsManager::new()
            .await
            .context("init settings manager")?,
    );
    settings_manager.ensure_settings_file().await.ok();
    let settings = settings_manager.get().await;
    golish_settings::apply_proxy_env(&settings);
    init_tracing_best_effort(&settings, args.verbose);

    // 3) Boot embedded Postgres (lazy pool + ready gate, mirroring the GUI) and
    //    build a headless AppState — AppState::new takes no Tauri AppHandle.
    let (db_pool, db_ready) = crate::app::bootstrap::create_lazy_db_pool();
    // Own the PG handle (don't leak it like the GUI) so we can stop the server
    // on exit — otherwise each run orphans a postgres holding port 15432 and
    // blocks the next --stage-run.
    let pg_handle_rx = crate::app::bootstrap::spawn_embedded_pg_owned(db_ready.clone());
    eprintln!("[stage-run] waiting for embedded Postgres (first run may download pg-embed)...");
    if !wait_for_db(&db_ready).await {
        return Err(anyhow!("embedded Postgres did not become ready in time"));
    }
    eprintln!("[stage-run] database ready.");

    // P1 · seed minimal upstream (org + in-scope targets) so an isolated
    // downstream stage (e.g. --only target_intel) has real data to work on.
    // Scoped to the workspace project_path the agent's manage_targets /
    // manage_organizations tools use; the seeded org id is then bound to the
    // orchestrator so the gate's in_scope_assets(org_id) only sees THIS org's
    // targets (coverage asset-axis isolation, design 2026-06-09).
    let workspace_str = workspace.to_string_lossy().to_string();
    let seed = maybe_seed(&db_pool, &workspace_str, &args).await;

    let app_state = crate::state::AppState::new(
        settings_manager.clone(),
        false,
        None,
        db_pool.clone(),
        db_ready,
    )
    .await;
    let agent_state = app_state.extract_agent_state();

    // 4) Build a CliRuntime whose event stream we own (for HITL auto-approve +
    //    report), then build + configure the bridge exactly like a GUI session.
    let (rt_tx, rt_rx) = mpsc::unbounded_channel::<RuntimeEvent>();
    let runtime: Arc<dyn GolishRuntime> =
        Arc::new(CliRuntime::new(rt_tx, args.auto_approve, args.json));

    let session_id = format!("stage-run-{}", uuid::Uuid::new_v4());

    let (mut bridge, mcp_manager) = match crate::cli::initialize_agent(
        &workspace,
        &settings,
        &args,
        runtime,
        app_state.indexer_state.clone(),
        app_state.sidecar_state.clone(),
    )
    .await
    .context("build agent bridge")
    {
        Ok(v) => v,
        Err(e) => {
            // Don't orphan the embedded PG we just started.
            stop_embedded_pg(pg_handle_rx).await;
            return Err(e);
        }
    };

    crate::ai::commands::configure_bridge(&mut bridge, &agent_state, &session_id, None).await;

    // Persist this run's transcript exactly where the GUI / `--replay` look.
    let transcripts_dir = golish_events::op_trace::resolve_transcript_base(Some(&workspace));
    match golish_events::TranscriptWriter::new(&transcripts_dir, &session_id).await {
        Ok(writer) => bridge.set_transcript_writer(writer, transcripts_dir.clone()),
        Err(e) => tracing::warn!("stage-run: transcript writer init failed: {e}"),
    }

    bridge.set_session_id(Some(session_id.clone())).await;
    bridge
        .set_execution_mode(golish_agent_kit::execution_mode::ExecutionMode::Task)
        .await;
    bridge.set_harness_profile(Some(profile_id.clone())).await;
    if args.auto_approve {
        bridge.set_agent_mode(AgentMode::AutoApprove).await;
    }

    // Unify the DB tracker's session with the orchestrator's session id (both
    // resolve the SAME chat-session key) so the harness gate's session-scoped
    // `tool_calls` cross-check (red_team scoping flow) reads THIS run's tool
    // calls instead of fail-opening. `set_db_backend` built the tracker with a
    // random uuid; override it here with the chat-key-resolved session row id —
    // the same id `orchestrate()` uses (upsert is idempotent on the key).
    {
        let model_name = bridge.model_name().to_string();
        let provider_name = bridge.provider_name().to_string();
        match golish_db::repo::sessions::upsert_by_chat_key(
            &db_pool,
            &session_id,
            golish_db::models::NewSession {
                title: Some(format!("stage-run {}", entry_stage.as_str())),
                workspace_path: None,
                workspace_label: None,
                model: Some(model_name),
                provider: Some(provider_name),
                project_path: None,
            },
        )
        .await
        {
            Ok(row) => bridge.set_tracker_session_uuid(row.id),
            Err(e) => {
                tracing::warn!("stage-run: tracker/orchestrator session unify failed: {e}")
            }
        }
    }

    let bridge = Arc::new(bridge);
    // Flush + enable live event emission so the coordinator forwards events to
    // our CliRuntime stream (otherwise they buffer waiting for a "frontend").
    bridge.mark_frontend_ready().await;

    // 5) Consume the event stream: auto-resolve scoping HITL and collect events
    //    for the report.
    let collected: Arc<Mutex<Vec<AiEvent>>> = Arc::new(Mutex::new(Vec::new()));
    spawn_event_consumer(rt_rx, bridge.clone(), collected.clone(), args.auto_approve);

    // 6) Orchestrate the slice (mirrors execute_task_mode, headless).
    let task_input = build_objective(&args, to_stage, seed.as_ref());
    let result = orchestrate(
        &bridge,
        &db_pool,
        &session_id,
        &profile_id,
        entry_stage,
        allowlist,
        &task_input,
        seed.as_ref().and_then(|s| s.org_id),
    )
    .await;

    // Give the event consumer a moment to drain trailing events.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 7) Report. Also write the replay artifacts so the timeline is on disk.
    let _ = golish_events::op_trace::write_trace_artifacts(&transcripts_dir, &session_id);
    let events = collected.lock().map(|v| v.clone()).unwrap_or_default();
    let report = format_report(
        &events,
        &result,
        &profile_id,
        entry_stage,
        to_stage,
        &session_id,
        &transcripts_dir,
    );
    if args.json {
        for ev in &events {
            if let Ok(s) = serde_json::to_string(ev) {
                println!("{s}");
            }
        }
    } else {
        println!("{report}");
    }

    if let Some(mgr) = mcp_manager {
        mgr.shutdown().await;
    }

    // Stop the embedded PG we started so we don't orphan it (headless cleanup;
    // the GUI keeps PG alive on purpose via the leaking `spawn_embedded_pg`).
    stop_embedded_pg(pg_handle_rx).await;

    result.map(|_| ())
}

/// Best-effort: stop the embedded PostgreSQL server started for this run.
///
/// The handle arrives via the oneshot from
/// [`spawn_embedded_pg_owned`](crate::app::bootstrap::spawn_embedded_pg_owned).
/// If startup failed (sender dropped) there is nothing to stop.
async fn stop_embedded_pg(rx: tokio::sync::oneshot::Receiver<golish_db::GolishDb>) {
    match rx.await {
        Ok(mut db) => {
            db.stop().await;
            eprintln!("[stage-run] embedded Postgres stopped.");
        }
        Err(_) => {
            tracing::debug!("stage-run: no embedded PG handle to stop (startup failed)");
        }
    }
}

/// Build the [`TaskOrchestrator`] and run the slice via `run_stage`. `org_id`
/// (from the upstream seed) binds the coverage gate's asset axis to THIS run's
/// organization (coverage asset-axis isolation, design 2026-06-09).
#[allow(clippy::too_many_arguments)]
async fn orchestrate(
    bridge: &Arc<AgentBridge>,
    db_pool: &Arc<sqlx::PgPool>,
    session_id: &str,
    profile_id: &str,
    entry_stage: StageKind,
    allowlist: HashSet<StageKind>,
    task_input: &str,
    org_id: Option<uuid::Uuid>,
) -> Result<String> {
    use golish_agent_bridge::bridge_executor::BridgeAgentExecutor;
    use golish_agent_kit::task_orchestrator::TaskOrchestrator;
    use golish_db::{models::NewSession, repo::sessions};

    let session_row = sessions::upsert_by_chat_key(
        db_pool,
        session_id,
        NewSession {
            title: Some(format!("stage-run {}", entry_stage.as_str())),
            workspace_path: None,
            workspace_label: None,
            model: Some(bridge.model_name().to_string()),
            provider: Some(bridge.provider_name().to_string()),
            project_path: None,
        },
    )
    .await
    .context("upsert session row (FK precondition for tasks)")?;

    let event_tx = bridge.get_or_create_event_tx();
    let db_repo: Arc<dyn golish_agent_kit::db_traits::DbRepoProvider> = Arc::new(
        crate::ai::db_bridge::GolishDbRepoProvider::new(db_pool.clone()),
    );

    let mut orchestrator = TaskOrchestrator::new(db_repo, session_row.id, event_tx);
    orchestrator.set_profile_override(Some(profile_id.to_string()));
    orchestrator.set_chat_session_id(session_id);
    orchestrator.set_approval_coordinator(bridge.coordinator().cloned());
    orchestrator.set_stage_allowlist(Some(allowlist));
    orchestrator.set_harness_org_id(org_id);

    let executor = BridgeAgentExecutor::new(bridge.clone());
    orchestrator
        .run_stage(entry_stage, task_input, &executor)
        .await
}

/// Watch the runtime event stream: auto-resolve `ask_human` requests (scoping
/// HITL) when `--auto-approve`, and collect events for the post-run report.
fn spawn_event_consumer(
    mut rx: mpsc::UnboundedReceiver<RuntimeEvent>,
    bridge: Arc<AgentBridge>,
    collected: Arc<Mutex<Vec<AiEvent>>>,
    auto_approve: bool,
) {
    tokio::spawn(async move {
        while let Some(rt_ev) = rx.recv().await {
            let event = match rt_ev {
                RuntimeEvent::Ai { event, .. } => *event,
                RuntimeEvent::AiEnvelope { envelope, .. } => envelope.event,
                _ => continue,
            };

            if auto_approve {
                if let AiEvent::AskHumanRequest {
                    request_id,
                    input_type,
                    ..
                } = &event
                {
                    let decision = ApprovalDecision {
                        request_id: request_id.clone(),
                        approved: true,
                        reason: Some(format!(
                            "auto-approved (headless --stage-run, {input_type})"
                        )),
                        remember: false,
                        always_allow: false,
                    };
                    let b = bridge.clone();
                    tokio::spawn(async move {
                        if let Err(e) = b.respond_to_approval(decision).await {
                            tracing::warn!("stage-run: auto-approve failed: {e}");
                        }
                    });
                }
            }

            if let Ok(mut v) = collected.lock() {
                v.push(event);
            }
        }
    });
}

/// Outcome of [`seed_upstream`]: what was created, for the report + objective.
struct SeedResult {
    org_id: Option<uuid::Uuid>,
    org_name: Option<String>,
    targets_added: usize,
}

/// Run the P1 upstream seed if `--org`/`--target` were given. Best-effort: a
/// seed failure is logged and the run continues (the stage will surface the gap).
async fn maybe_seed(
    db_pool: &Arc<sqlx::PgPool>,
    project_path: &str,
    args: &Args,
) -> Option<SeedResult> {
    if args.org.is_none() && args.target.is_empty() {
        return None;
    }
    match seed_upstream(db_pool, project_path, args.org.as_deref(), &args.target).await {
        Ok(s) => {
            eprintln!(
                "[stage-run] seeded upstream: org={:?} (id={:?}) targets={} project_path={project_path}",
                s.org_name, s.org_id, s.targets_added
            );
            Some(s)
        }
        Err(e) => {
            eprintln!("[stage-run] upstream seed failed (continuing): {e:#}");
            None
        }
    }
}

/// Create an organization (if named) + in-scope targets bound to it, scoped to
/// `project_path` (matching `manage_targets`/`manage_organizations`). Mirrors the
/// `manage_targets add` path (`target_add`, which defaults `scope='in'`) so the
/// gate's `in_scope_assets` and the recon tools both see the seed.
async fn seed_upstream(
    db_pool: &Arc<sqlx::PgPool>,
    project_path: &str,
    org_name: Option<&str>,
    targets: &[String],
) -> Result<SeedResult> {
    use golish_app_core::ports::recon::{PgReconTargetsAdapter, ReconTargetsPort};

    let mut org_id: Option<uuid::Uuid> = None;
    let mut org_name_out: Option<String> = None;
    if let Some(name) = org_name.map(str::trim).filter(|s| !s.is_empty()) {
        // get-or-create: the embedded PG persists across runs, so a repeated
        // `--org` would hit the `uq_orgs_root_name` unique constraint, abort the
        // whole seed, drop `org_id`, and silently fall back to the legacy
        // whole-DB coverage axis (org isolation never exercised). Reuse the
        // existing root org by name when present.
        let id =
            match golish_db::repo::organizations::find_root_id_by_name(db_pool, project_path, name)
                .await
                .context("seed organization lookup")?
            {
                Some(existing) => existing,
                None => {
                    golish_db::repo::organizations::create(
                        db_pool,
                        project_path,
                        name,
                        None,
                        "",
                        "",
                    )
                    .await
                    .context("seed organization")?
                    .id
                }
            };
        org_id = Some(id);
        org_name_out = Some(name.to_string());
    }

    let adapter = PgReconTargetsAdapter::new(db_pool.clone());
    let mut targets_added = 0usize;
    for t in targets {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        adapter
            .target_add(
                t,                  // name
                t,                  // value
                None,               // target_type (auto-detect)
                None,               // grp
                None,               // owner
                None,               // time_window_start
                None,               // time_window_end
                org_id,             // organization_id
                Some(project_path), // project_path
                "stage-run-seed",   // source
                None,               // parent_id
            )
            .await
            .with_context(|| format!("seed target {t}"))?;
        targets_added += 1;
    }

    Ok(SeedResult {
        org_id,
        org_name: org_name_out,
        targets_added,
    })
}

/// Build the task objective. When `-e/--execute` is given it wins; otherwise
/// synthesize one that names the seeded organization (with its real id, so the
/// agent can call `recon_*` without first guessing the org) and in-scope targets.
fn build_objective(args: &Args, to: StageKind, seed: Option<&SeedResult>) -> String {
    if let Some(e) = args.execute.clone() {
        return e;
    }
    let mut s = format!("Run the {} stage for this engagement.", to.as_str());
    match seed {
        Some(sd) => {
            if let (Some(name), Some(id)) = (&sd.org_name, &sd.org_id) {
                s.push_str(&format!(" Organization: {name} (organization_id: {id})."));
            } else if let Some(name) = &sd.org_name {
                s.push_str(&format!(" Organization: {name}."));
            }
        }
        None => {
            if let Some(org) = &args.org {
                s.push_str(&format!(" Organization: {org}."));
            }
        }
    }
    if !args.target.is_empty() {
        s.push_str(&format!(" In-scope targets: {}.", args.target.join(", ")));
    }
    s
}

/// Wait (up to ~3 min) for the embedded DB to flip its ready gate.
async fn wait_for_db(db_ready: &golish_db::DbReadyGate) -> bool {
    for _ in 0..18 {
        if db_ready.is_ready() {
            return true;
        }
        if db_ready.is_failed() {
            return false;
        }
        if db_ready.wait_timeout(Duration::from_secs(10)).await {
            return true;
        }
    }
    db_ready.is_ready()
}

fn init_tracing_best_effort(settings: &golish_settings::GolishSettings, verbose: bool) {
    let log_level = if verbose { "debug" } else { "info" };
    let directives_owned: Vec<String> = [
        "golish",
        "golish_agent_kit",
        "golish_agent_runtime",
        "golish_agent_bridge",
        "golish_prompts",
        "harness",
    ]
    .iter()
    .map(|c| format!("{c}={log_level}"))
    .collect();
    let directives: Vec<&str> = directives_owned.iter().map(|s| s.as_str()).collect();
    let langfuse = crate::telemetry::LangfuseConfig::from_settings(&settings.telemetry.langfuse);
    let _ = crate::telemetry::init_tracing(langfuse, log_level, &directives);
}

/// Render the human-readable post-run report from collected events.
fn format_report(
    events: &[AiEvent],
    result: &Result<String>,
    profile: &str,
    entry: StageKind,
    to: StageKind,
    session_id: &str,
    transcripts_dir: &Path,
) -> String {
    let mut out = String::new();
    out.push_str("\n══════════════ stage-run report ══════════════\n");
    out.push_str(&format!(
        "profile = {profile}\nslice   = {} ..= {}\n",
        entry.as_str(),
        to.as_str()
    ));

    let mut gate_lines = Vec::new();
    let mut evidence_lines = Vec::new();
    let mut tool_lines = Vec::new();
    let mut askhuman = 0usize;
    let mut errors = Vec::new();

    for ev in events {
        match ev {
            AiEvent::HarnessTrace { stage, trace, .. } => match trace {
                HarnessTraceKind::GateDecision {
                    gate,
                    findings,
                    first_blocking_reason,
                    fabricated_evidence_refs,
                    ..
                } => {
                    let mut l = format!("  [{gate}] {stage} (findings={findings})");
                    if gate == "BLOCK" {
                        if let Some(r) = first_blocking_reason {
                            l.push_str(&format!("\n         reason: {r}"));
                        }
                        if !fabricated_evidence_refs.is_empty() {
                            l.push_str(&format!(
                                "\n         fabricated evidence refs: {fabricated_evidence_refs:?}"
                            ));
                        }
                    }
                    gate_lines.push(l);
                }
                HarnessTraceKind::EvidenceBooked {
                    tool,
                    evidence_id,
                    source,
                } => {
                    evidence_lines.push(format!("  #{evidence_id} from {tool} ({source})"));
                }
                _ => {}
            },
            AiEvent::ToolResult {
                tool_name, success, ..
            } => {
                tool_lines.push(format!(
                    "  {tool_name}: {}",
                    if *success { "ok" } else { "err" }
                ));
            }
            AiEvent::AskHumanRequest { .. } => askhuman += 1,
            AiEvent::Error { message, .. } => errors.push(format!("  {message}")),
            _ => {}
        }
    }

    out.push_str("\n-- gate decisions --\n");
    out.push_str(&if gate_lines.is_empty() {
        "  (none recorded)\n".to_string()
    } else {
        format!("{}\n", gate_lines.join("\n"))
    });

    out.push_str("\n-- tools invoked --\n");
    out.push_str(&if tool_lines.is_empty() {
        "  (none)\n".to_string()
    } else {
        format!("{}\n", tool_lines.join("\n"))
    });

    out.push_str("\n-- evidence booked --\n");
    out.push_str(&if evidence_lines.is_empty() {
        "  (none)\n".to_string()
    } else {
        format!("{}\n", evidence_lines.join("\n"))
    });

    if askhuman > 0 {
        out.push_str(&format!(
            "\n-- HITL --\n  {askhuman} ask_human request(s) (auto-approved)\n"
        ));
    }
    if !errors.is_empty() {
        out.push_str(&format!("\n-- errors --\n{}\n", errors.join("\n")));
    }

    out.push_str("\n-- result --\n");
    match result {
        Ok(r) => {
            let preview: String = r.chars().take(800).collect();
            out.push_str(&format!("  OK ({} chars)\n{preview}\n", r.len()));
        }
        Err(e) => out.push_str(&format!("  FAILED: {e:#}\n")),
    }

    out.push_str(&format!(
        "\nfull transcript: {}/{}\nreplay:          golish --replay {session_id}\n",
        transcripts_dir.display(),
        session_id
    ));
    out.push_str("══════════════════════════════════════════════\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn resolve_slice_only_single_stage() {
        let (entry, allowlist) = resolve_slice(
            "assessment",
            Some(StageKind::TargetIntel),
            StageKind::TargetIntel,
        )
        .expect("target_intel is in the assessment profile");
        assert_eq!(entry, StageKind::TargetIntel);
        assert_eq!(allowlist, HashSet::from([StageKind::TargetIntel]));
    }

    #[test]
    fn resolve_slice_to_target_intel_from_entry() {
        let (entry, allowlist) =
            resolve_slice("assessment", None, StageKind::TargetIntel).expect("reachable");
        assert_eq!(entry, StageKind::Scoping);
        assert_eq!(
            allowlist,
            HashSet::from([StageKind::Scoping, StageKind::TargetIntel])
        );
    }

    #[test]
    fn resolve_slice_unknown_profile_errs() {
        assert!(resolve_slice("does_not_exist", None, StageKind::Scoping).is_err());
    }

    #[test]
    fn resolve_slice_to_not_in_profile_errs() {
        // vuln_triage is forbidden in the assessment profile.
        assert!(resolve_slice("assessment", None, StageKind::VulnTriage).is_err());
    }

    #[test]
    fn resolve_from_to_only_sets_both() {
        let args = Args::parse_from(["golish", "--stage-run", "--only", "scoping"]);
        let (from, to) = resolve_from_to(&args).unwrap();
        assert_eq!(from, Some(StageKind::Scoping));
        assert_eq!(to, StageKind::Scoping);
    }

    #[test]
    fn resolve_from_to_requires_to_or_only() {
        let args = Args::parse_from(["golish", "--stage-run"]);
        assert!(resolve_from_to(&args).is_err());
    }

    #[test]
    fn format_report_includes_gate_and_evidence() {
        let events = vec![
            AiEvent::HarnessTrace {
                operation_id: "op".into(),
                stage: "scoping".into(),
                agent_path: "main".into(),
                trace: HarnessTraceKind::GateDecision {
                    gate: "PASS".into(),
                    findings: 0,
                    fabricated_evidence_refs: vec![],
                    available_real_ids: vec![],
                    first_blocking_reason: None,
                },
            },
            AiEvent::HarnessTrace {
                operation_id: "op".into(),
                stage: "target_intel".into(),
                agent_path: "main".into(),
                trace: HarnessTraceKind::EvidenceBooked {
                    tool: "recon_enrich_assets".into(),
                    evidence_id: 42,
                    source: "sync".into(),
                },
            },
        ];
        let result: Result<String> = Ok("done".into());
        let report = format_report(
            &events,
            &result,
            "assessment",
            StageKind::Scoping,
            StageKind::TargetIntel,
            "stage-run-x",
            Path::new("/tmp/t"),
        );
        assert!(report.contains("[PASS] scoping"));
        assert!(report.contains("#42 from recon_enrich_assets"));
        assert!(report.contains("golish --replay stage-run-x"));
    }

    #[test]
    fn format_report_shows_block_reason_and_failure() {
        let events = vec![AiEvent::HarnessTrace {
            operation_id: "op".into(),
            stage: "scoping".into(),
            agent_path: "main".into(),
            trace: HarnessTraceKind::GateDecision {
                gate: "BLOCK".into(),
                findings: 0,
                fabricated_evidence_refs: vec![1, 2],
                available_real_ids: vec![],
                first_blocking_reason: Some("missing scope_human_approved claim".into()),
            },
        }];
        let result: Result<String> = Err(anyhow!("stage blocked"));
        let report = format_report(
            &events,
            &result,
            "pentest",
            StageKind::Scoping,
            StageKind::Scoping,
            "s",
            Path::new("/tmp/t"),
        );
        assert!(report.contains("[BLOCK] scoping"));
        assert!(report.contains("missing scope_human_approved claim"));
        assert!(report.contains("fabricated evidence refs: [1, 2]"));
        assert!(report.contains("FAILED: stage blocked"));
    }

    #[test]
    fn build_objective_includes_seeded_org_id_and_targets() {
        let args = Args::parse_from([
            "golish",
            "--stage-run",
            "--only",
            "target_intel",
            "--org",
            "ACME",
            "--target",
            "acme.com",
        ]);
        let seed = SeedResult {
            org_id: Some(uuid::Uuid::nil()),
            org_name: Some("ACME".into()),
            targets_added: 1,
        };
        let obj = build_objective(&args, StageKind::TargetIntel, Some(&seed));
        assert!(obj.contains("organization_id: 00000000-0000-0000-0000-000000000000"));
        assert!(obj.contains("Organization: ACME"));
        assert!(obj.contains("In-scope targets: acme.com"));
    }

    #[test]
    fn build_objective_prefers_explicit_execute() {
        let args = Args::parse_from([
            "golish",
            "--stage-run",
            "--only",
            "scoping",
            "-e",
            "custom obj",
        ]);
        assert_eq!(
            build_objective(&args, StageKind::Scoping, None),
            "custom obj"
        );
    }
}
