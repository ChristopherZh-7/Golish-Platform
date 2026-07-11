//! Restore and persist the sub-agent's conversation chain via the
//! [`SubAgentChainPersistence`] trait.
//!
//! This is the PentAGI-style persistent message-chain pattern: the chain
//! survives across invocations of the same agent within a session/task so
//! cross-invocation context (memory) can be injected later via the briefing
//! system.

use rig::completion::Message;

use crate::definition::SubAgentContext;
use crate::executor_helpers::{deserialize_chat_history, serialize_chat_history};
use crate::executor_types::{
    SubAgentChainError, SubAgentChainPersistence, SubAgentExecutorContext,
};

use super::history_compaction::compact_history_for_provider;

async fn prepare_loaded_chain(
    persistence: &dyn SubAgentChainPersistence,
    chain_id: uuid::Uuid,
    stored_json: &serde_json::Value,
) -> anyhow::Result<Vec<Message>> {
    let messages = deserialize_chat_history(stored_json)
        .map_err(|error| anyhow::anyhow!("stored history is invalid: {error:#}"))?;
    anyhow::ensure!(!messages.is_empty(), "stored history is empty");

    let (messages, stats) = compact_history_for_provider(messages)?;
    let compacted_json = serialize_chat_history(&messages)?;
    if &compacted_json != stored_json {
        persistence
            .chain_update(chain_id, &compacted_json)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to durably rewrite provider-safe restored history: {error:#}"
                )
            })?;
        tracing::info!(
            chain_id = %chain_id,
            before_bytes = stats.before_bytes,
            after_bytes = stats.after_bytes,
            compacted_tool_results = stats.compacted_tool_results,
            collapsed_repair_directives = stats.collapsed_repair_directives,
            omitted_messages = stats.omitted_messages,
            "compacted durable sub-agent history before exact-chain replay"
        );
    }
    Ok(messages)
}

/// Resolve the chain for this delegation, honoring the AI-controlled
/// `ctx.resume`:
/// - `Some("<uuid>")` → continue THAT exact prior chain (replay its messages).
/// - `Some("latest")` → continue this agent's most recent chain.
/// - any other non-UUID selector → fail closed.
/// - `None` → create a fresh chain (legacy behavior).
///
/// Returns `(chain_id, prior_messages)`. `prior_messages` is empty for a fresh
/// chain; for a resume it is the full-fidelity replay (incl. tool results /
/// evidence ids) so the worker remembers what it already did.
pub(super) async fn maybe_restore_chain(
    ctx: &SubAgentExecutorContext<'_>,
    parent_context: &SubAgentContext,
    agent_id: &str,
) -> anyhow::Result<(Option<uuid::Uuid>, Vec<Message>)> {
    let requested_exact = ctx
        .resume
        .as_deref()
        .and_then(|resume| uuid::Uuid::parse_str(resume).ok());
    let requested_latest = ctx.resume.as_deref() == Some("latest");
    if let Some(resume) = ctx.resume.as_deref() {
        if requested_exact.is_none() && !requested_latest {
            return Err(SubAgentChainError::LatestResumeUnavailable {
                agent_id: agent_id.to_string(),
                reason: format!("invalid resume selector '{resume}'; expected a UUID or 'latest'"),
            }
            .into());
        }
    }

    let Some(persistence) = ctx.chain_persistence else {
        return match requested_exact {
            Some(chain_id) => Err(SubAgentChainError::ExactResumeUnavailable {
                chain_id,
                reason: "chain persistence backend is unavailable".to_string(),
            }
            .into()),
            None if requested_latest => Err(SubAgentChainError::LatestResumeUnavailable {
                agent_id: agent_id.to_string(),
                reason: "chain persistence backend is unavailable".to_string(),
            }
            .into()),
            None => Ok((None, Vec::new())),
        };
    };
    let Some(session_uuid) = ctx
        .persistence_session_id
        .or_else(|| ctx.session_id.and_then(|s| uuid::Uuid::parse_str(s).ok()))
    else {
        return match requested_exact {
            Some(chain_id) => Err(SubAgentChainError::ExactResumeUnavailable {
                chain_id,
                reason: "database session identity is unavailable".to_string(),
            }
            .into()),
            None if requested_latest => Err(SubAgentChainError::LatestResumeUnavailable {
                agent_id: agent_id.to_string(),
                reason: "database session identity is unavailable".to_string(),
            }
            .into()),
            None => Ok((None, Vec::new())),
        };
    };
    let task_uuid = parent_context
        .task_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    // Resume: continue a prior worker the AI named.
    if let Some(chain_id) = requested_exact {
        let json = match persistence
            .chain_load_by_id(chain_id, session_uuid, agent_id)
            .await
        {
            Ok(Some(json)) => json,
            Ok(None) => {
                return Err(SubAgentChainError::ExactResumeUnavailable {
                    chain_id,
                    reason: "not found in the current session and agent scope".to_string(),
                }
                .into())
            }
            Err(error) => {
                return Err(SubAgentChainError::ExactResumeUnavailable {
                    chain_id,
                    reason: format!("load failed: {error:#}"),
                }
                .into())
            }
        };
        let msgs = prepare_loaded_chain(persistence.as_ref(), chain_id, &json)
            .await
            .map_err(|error| SubAgentChainError::ExactResumeUnavailable {
                chain_id,
                reason: error.to_string(),
            })?;
        tracing::info!(
            "[sub-agent:{}] Resumed chain {} ({} prior messages)",
            agent_id,
            chain_id,
            msgs.len()
        );
        return Ok((Some(chain_id), msgs));
    }

    if requested_latest {
        let (chain_id, json) = match persistence
            .chain_load_latest(session_uuid, task_uuid, agent_id)
            .await
        {
            Ok(Some(chain)) => chain,
            Ok(None) => {
                return Err(SubAgentChainError::LatestResumeUnavailable {
                    agent_id: agent_id.to_string(),
                    reason: "no persisted chain exists in the current session".to_string(),
                }
                .into())
            }
            Err(error) => {
                return Err(SubAgentChainError::LatestResumeUnavailable {
                    agent_id: agent_id.to_string(),
                    reason: format!("load failed: {error:#}"),
                }
                .into())
            }
        };
        let msgs = prepare_loaded_chain(persistence.as_ref(), chain_id, &json)
            .await
            .map_err(|error| SubAgentChainError::LatestResumeUnavailable {
                agent_id: agent_id.to_string(),
                reason: error.to_string(),
            })?;
        tracing::info!(
            "[sub-agent:{}] Resumed latest chain {} ({} prior messages)",
            agent_id,
            chain_id,
            msgs.len()
        );
        return Ok((Some(chain_id), msgs));
    }

    // Fresh chain only when no resume was requested. Explicit resume failures
    // returned above and are never allowed to fall through to this branch.
    match persistence
        .chain_create(session_uuid, task_uuid, None, agent_id, None, None)
        .await
    {
        Ok(cid) => {
            tracing::info!("[sub-agent:{}] Created persistent chain {}", agent_id, cid);
            Ok((Some(cid), Vec::new()))
        }
        Err(e) => Err(SubAgentChainError::CreateFreshFailed {
            agent_id: agent_id.to_string(),
            reason: format!("chain create failed: {e:#}"),
        }
        .into()),
    }
}

/// Durably write a provider-valid chain body without finalizing usage.
///
/// The executor calls this immediately after a complete assistant tool-call
/// turn and its matching user ToolResult turn have both been appended. Keeping
/// usage out of this path lets long workers checkpoint every completed batch
/// while recording final duration exactly once during teardown.
pub(super) async fn checkpoint_chain(
    ctx: &SubAgentExecutorContext<'_>,
    chain_id: Option<uuid::Uuid>,
    chat_history: &[Message],
    agent_id: &str,
) -> anyhow::Result<Option<uuid::Uuid>> {
    let (Some(persistence), Some(cid)) = (ctx.chain_persistence, chain_id) else {
        return Ok(None);
    };

    let (compacted_history, stats) =
        compact_history_for_provider(chat_history.to_vec()).map_err(|error| {
            SubAgentChainError::FinalizeFailed {
                chain_id: cid,
                checkpointed_chain_id: None,
                reason: format!("history compaction failed: {error:#}"),
            }
        })?;
    let chain_json = serialize_chat_history(&compacted_history).map_err(|error| {
        SubAgentChainError::FinalizeFailed {
            chain_id: cid,
            checkpointed_chain_id: None,
            reason: format!("history serialization failed: {error:#}"),
        }
    })?;
    persistence
        .chain_update(cid, &chain_json)
        .await
        .map_err(|error| SubAgentChainError::FinalizeFailed {
            chain_id: cid,
            checkpointed_chain_id: None,
            reason: format!("chain update failed: {error:#}"),
        })?;
    tracing::info!(
        "[sub-agent:{}] Checkpointed {} provider-safe messages to chain {} ({} -> {} bytes)",
        agent_id,
        compacted_history.len(),
        cid,
        stats.before_bytes,
        stats.after_bytes,
    );
    Ok(Some(cid))
}

pub(super) async fn persist_chain(
    ctx: &SubAgentExecutorContext<'_>,
    chain_id: Option<uuid::Uuid>,
    chat_history: &[Message],
    duration_ms: u64,
    agent_id: &str,
) -> anyhow::Result<Option<uuid::Uuid>> {
    let durable_chain_id = checkpoint_chain(ctx, chain_id, chat_history, agent_id).await?;
    let (Some(persistence), Some(cid)) = (ctx.chain_persistence, durable_chain_id) else {
        return Ok(None);
    };

    if let Err(e) = persistence
        .chain_update_usage(cid, 0, 0, 0, 0.0, 0.0, duration_ms as i32)
        .await
    {
        tracing::warn!(
            "[sub-agent:{}] Failed to update chain usage {}: {}",
            agent_id,
            cid,
            e
        );
    }
    Ok(Some(cid))
}

pub(super) fn append_durable_chain_marker(
    final_response: String,
    durable_chain_id: Option<uuid::Uuid>,
) -> String {
    match durable_chain_id {
        Some(cid) => format!("{final_response}\n\n[sub_agent_session_id: {cid}]"),
        None => final_response,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use golish_core::events::AiEvent;
    use golish_tools::ToolRegistry;
    use rig::completion::{AssistantContent, Message};
    use rig::message::{Text, ToolCall, ToolFunction, ToolResult, ToolResultContent, UserContent};
    use rig::one_or_many::OneOrMany;
    use tokio::sync::{mpsc, RwLock};
    use uuid::Uuid;

    use super::{
        append_durable_chain_marker, checkpoint_chain, maybe_restore_chain, persist_chain,
    };
    use crate::definition::SubAgentContext;
    use crate::executor_types::{
        SubAgentChainError, SubAgentChainPersistence, SubAgentExecutorContext,
    };

    enum ExactLoad {
        Missing,
        Error,
        Present(serde_json::Value),
    }

    enum LatestLoad {
        Missing,
        Error,
        Present(Uuid, serde_json::Value),
    }

    struct RecordingChainPersistence {
        chain_id: Uuid,
        created_for_sessions: Mutex<Vec<Uuid>>,
        exact_load: ExactLoad,
        latest_load: LatestLoad,
        exact_session: Option<Uuid>,
        exact_agent: Option<String>,
        fail_create: bool,
        fail_update: bool,
        updates: Mutex<Vec<serde_json::Value>>,
        usage_updates: Mutex<Vec<i32>>,
    }

    impl RecordingChainPersistence {
        fn new(chain_id: Uuid) -> Self {
            Self {
                chain_id,
                created_for_sessions: Mutex::new(Vec::new()),
                exact_load: ExactLoad::Missing,
                latest_load: LatestLoad::Missing,
                exact_session: None,
                exact_agent: None,
                fail_create: false,
                fail_update: false,
                updates: Mutex::new(Vec::new()),
                usage_updates: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl SubAgentChainPersistence for RecordingChainPersistence {
        async fn chain_create(
            &self,
            session_id: Uuid,
            _task_id: Option<Uuid>,
            _subtask_id: Option<Uuid>,
            _agent_type: &str,
            _parent_chain_id: Option<Uuid>,
            _model: Option<&str>,
        ) -> anyhow::Result<Uuid> {
            if self.fail_create {
                anyhow::bail!("synthetic chain create failure")
            }
            self.created_for_sessions
                .lock()
                .expect("recording mutex")
                .push(session_id);
            Ok(self.chain_id)
        }

        async fn chain_update(
            &self,
            _id: Uuid,
            chain_json: &serde_json::Value,
        ) -> anyhow::Result<()> {
            if self.fail_update {
                anyhow::bail!("synthetic chain update failure")
            }
            self.updates
                .lock()
                .expect("recording mutex")
                .push(chain_json.clone());
            Ok(())
        }

        async fn chain_update_usage(
            &self,
            _id: Uuid,
            _input_tokens: i32,
            _output_tokens: i32,
            _cache_read_tokens: i32,
            _input_cost: f64,
            _output_cost: f64,
            duration_ms: i32,
        ) -> anyhow::Result<()> {
            self.usage_updates
                .lock()
                .expect("recording mutex")
                .push(duration_ms);
            Ok(())
        }

        async fn chain_load_latest(
            &self,
            _session_id: Uuid,
            _task_id: Option<Uuid>,
            _agent_type: &str,
        ) -> anyhow::Result<Option<(Uuid, serde_json::Value)>> {
            match &self.latest_load {
                LatestLoad::Missing => Ok(None),
                LatestLoad::Error => anyhow::bail!("synthetic latest chain load failure"),
                LatestLoad::Present(chain_id, value) => Ok(Some((*chain_id, value.clone()))),
            }
        }

        async fn chain_load_by_id(
            &self,
            _chain_id: Uuid,
            session_id: Uuid,
            agent_type: &str,
        ) -> anyhow::Result<Option<serde_json::Value>> {
            if self
                .exact_session
                .is_some_and(|expected| expected != session_id)
                || self
                    .exact_agent
                    .as_deref()
                    .is_some_and(|expected| expected != agent_type)
            {
                return Ok(None);
            }
            match &self.exact_load {
                ExactLoad::Missing => Ok(None),
                ExactLoad::Error => anyhow::bail!("synthetic exact chain load failure"),
                ExactLoad::Present(value) => Ok(Some(value.clone())),
            }
        }

        async fn load_prompt_template_overrides(&self) -> Vec<(String, String)> {
            Vec::new()
        }
    }

    fn test_history() -> Vec<Message> {
        vec![Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: "continue exact worker".to_string(),
            })),
        }]
    }

    fn serialized_test_history() -> serde_json::Value {
        serde_json::to_value(test_history()).expect("test history serializes")
    }

    fn dangling_tool_history() -> Vec<Message> {
        vec![Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                id: "tool-id".to_string(),
                call_id: Some("call-provider-123".to_string()),
                function: ToolFunction {
                    name: "submit_result".to_string(),
                    arguments: serde_json::json!({"result": "done"}),
                },
                signature: None,
                additional_params: None,
            })),
        }]
    }

    fn completed_tool_batch_history() -> Vec<Message> {
        let mut history = dangling_tool_history();
        history.push(Message::User {
            content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: "tool-id".to_string(),
                call_id: Some("call-provider-123".to_string()),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: r#"{"status":"ok"}"#.to_string(),
                })),
            })),
        });
        history
    }

    fn partial_multi_tool_batch_history() -> Vec<Message> {
        let mut history = completed_tool_batch_history();
        let Message::Assistant { content, .. } = &mut history[0] else {
            unreachable!("fixture starts with an assistant tool-call turn")
        };
        let mut calls = content.iter().cloned().collect::<Vec<_>>();
        calls.push(AssistantContent::ToolCall(ToolCall {
            id: "tool-id-2".to_string(),
            call_id: Some("call-provider-456".to_string()),
            function: ToolFunction {
                name: "query_target_data".to_string(),
                arguments: serde_json::json!({"kind": "targets"}),
            },
            signature: None,
            additional_params: None,
        }));
        *content = OneOrMany::many(calls).expect("two assistant tool calls");
        history
    }

    fn bulky_worklist_history() -> Vec<Message> {
        let call_id = "call-worklist-bulky";
        let items = (0..200)
            .map(|index| {
                serde_json::json!({
                    "work_item_id": format!("target-{index}:https://host-{index}.example:443:GOLISH-ENUM-DIR"),
                    "target_id": format!("target-{index}"),
                    "asset": format!("https://host-{index}.example:443"),
                    "base_url": format!("https://host-{index}.example:443/"),
                    "technique": "GOLISH-ENUM-DIR",
                    "state": "partial",
                    "evidence_refs": [],
                    "note": "n".repeat(4096),
                    "details": "d".repeat(4096),
                    "suggested_tools": ["route_probe_paths"],
                })
            })
            .collect::<Vec<_>>();
        let result = serde_json::json!({
            "tool": "stage_worklist_next",
            "stage": "enumeration",
            "ready_to_submit": false,
            "cell_summary": {
                "total_cells": 1488,
                "pending_cells": 900,
                "partial_cells": 55,
                "error_cells": 0,
            },
            "items": items,
            "next_action": "Resume only the exact pending and partial roots from DB truth.",
        });
        let repair_actions = (0..1176)
            .map(|index| {
                format!(
                    "{index}. asset=https://host-{index}.example:443 technique=GOLISH-ENUM-DIR reason={} suggested_tools=route_probe_paths",
                    "coverage gap ".repeat(32)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let repair = |generation: usize| {
            Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: format!(
                    "RESUME REPAIR DIRECTIVE (deterministic): generation={generation}; \
                     run ONLY these 1176 target/technique pairs.\n{repair_actions}\n\
                     Allowed next tools: [stage_worklist_status, stage_worklist_next, route_probe_paths]."
                ),
            })),
        }
        };
        vec![
            repair(1),
            Message::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                    id: call_id.to_string(),
                    call_id: Some(call_id.to_string()),
                    function: ToolFunction {
                        name: "stage_worklist_next".to_string(),
                        arguments: serde_json::json!({
                            "limit": 200,
                            "prefer": ["pending", "error", "partial"],
                        }),
                    },
                    signature: None,
                    additional_params: None,
                })),
            },
            Message::User {
                content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                    id: call_id.to_string(),
                    call_id: Some(call_id.to_string()),
                    content: OneOrMany::one(ToolResultContent::Text(Text {
                        text: serde_json::to_string(&result).expect("result serializes"),
                    })),
                })),
            },
            repair(2),
        ]
    }

    fn first_tool_result_json(messages: &[Message]) -> serde_json::Value {
        messages
            .iter()
            .find_map(|message| {
                let Message::User { content } = message else {
                    return None;
                };
                content.iter().find_map(|item| {
                    let UserContent::ToolResult(result) = item else {
                        return None;
                    };
                    result.content.iter().find_map(|content| {
                        let ToolResultContent::Text(text) = content else {
                            return None;
                        };
                        serde_json::from_str(&text.text).ok()
                    })
                })
            })
            .expect("history contains a JSON tool result")
    }

    fn repair_directive_texts(messages: &[Message]) -> Vec<&str> {
        messages
            .iter()
            .filter_map(|message| {
                let Message::User { content } = message else {
                    return None;
                };
                let mut items = content.iter();
                let UserContent::Text(text) = items.next()? else {
                    return None;
                };
                (items.next().is_none()
                    && text
                        .text
                        .starts_with("RESUME REPAIR DIRECTIVE (deterministic):"))
                .then_some(text.text.as_str())
            })
            .collect()
    }

    async fn run_restore(
        recording: Arc<RecordingChainPersistence>,
        resume: Option<String>,
        event_session_id: &str,
        persistence_session_id: Option<Uuid>,
    ) -> anyhow::Result<(Option<Uuid>, Vec<Message>)> {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = Arc::new(RwLock::new(temp.path().to_path_buf()));
        let registry = Arc::new(RwLock::new(
            ToolRegistry::new(temp.path().to_path_buf()).await,
        ));
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<AiEvent>();
        let persistence: Arc<dyn SubAgentChainPersistence> = recording;
        let ctx = SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: Some(event_session_id),
            persistence_session_id,
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: None,
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: Some(&persistence),
            sub_agent_registry: None,
            post_shell_hook: None,
            resume,
            sub_tool_router: None,
            active_org_id_source: None,
            active_org_id_override: None,
            operation_id: None,
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };

        maybe_restore_chain(&ctx, &SubAgentContext::default(), "enumerator").await
    }

    async fn run_persist(
        recording: Arc<RecordingChainPersistence>,
        chain_id: Uuid,
    ) -> anyhow::Result<Option<Uuid>> {
        run_persist_with_history(recording, chain_id, test_history()).await
    }

    async fn run_persist_with_history(
        recording: Arc<RecordingChainPersistence>,
        chain_id: Uuid,
        history: Vec<Message>,
    ) -> anyhow::Result<Option<Uuid>> {
        run_persist_with_history_and_duration(recording, chain_id, history, 10).await
    }

    async fn run_persist_with_history_and_duration(
        recording: Arc<RecordingChainPersistence>,
        chain_id: Uuid,
        history: Vec<Message>,
        duration_ms: u64,
    ) -> anyhow::Result<Option<Uuid>> {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = Arc::new(RwLock::new(temp.path().to_path_buf()));
        let registry = Arc::new(RwLock::new(
            ToolRegistry::new(temp.path().to_path_buf()).await,
        ));
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<AiEvent>();
        let persistence: Arc<dyn SubAgentChainPersistence> = recording;
        let ctx = SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: Some("stage-run-test"),
            persistence_session_id: Some(Uuid::new_v4()),
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: None,
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: Some(&persistence),
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: None,
            active_org_id_override: None,
            operation_id: None,
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };
        persist_chain(&ctx, Some(chain_id), &history, duration_ms, "enumerator").await
    }

    async fn run_checkpoint_with_history(
        recording: Arc<RecordingChainPersistence>,
        chain_id: Uuid,
        history: Vec<Message>,
    ) -> anyhow::Result<Option<Uuid>> {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = Arc::new(RwLock::new(temp.path().to_path_buf()));
        let registry = Arc::new(RwLock::new(
            ToolRegistry::new(temp.path().to_path_buf()).await,
        ));
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<AiEvent>();
        let persistence: Arc<dyn SubAgentChainPersistence> = recording;
        let ctx = SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: Some("stage-run-test"),
            persistence_session_id: Some(Uuid::new_v4()),
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: None,
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: Some(&persistence),
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: None,
            active_org_id_override: None,
            operation_id: None,
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };
        checkpoint_chain(&ctx, Some(chain_id), &history, "enumerator").await
    }

    #[tokio::test]
    async fn fresh_chain_uses_persistence_session_id_when_event_session_is_not_uuid() {
        let db_session_id = Uuid::new_v4();
        let expected_chain_id = Uuid::new_v4();
        let recording = Arc::new(RecordingChainPersistence::new(expected_chain_id));

        let (chain_id, prior_messages) = run_restore(
            recording.clone(),
            None,
            "stage-run-8ed23f97-f839-4a49-9e9a-77c65d1b6129",
            Some(db_session_id),
        )
        .await
        .expect("fresh chain create succeeds");

        assert_eq!(chain_id, Some(expected_chain_id));
        assert!(prior_messages.is_empty());
        assert_eq!(
            *recording
                .created_for_sessions
                .lock()
                .expect("recording mutex"),
            vec![db_session_id]
        );
    }

    #[tokio::test]
    async fn fresh_chain_create_error_fails_before_worker_execution() {
        let mut persistence = RecordingChainPersistence::new(Uuid::new_v4());
        persistence.fail_create = true;
        let recording = Arc::new(persistence);

        let error = run_restore(
            recording.clone(),
            None,
            "stage-run-test",
            Some(Uuid::new_v4()),
        )
        .await
        .expect_err("fresh chain create failure must stop setup");

        assert!(matches!(
            error.downcast_ref::<SubAgentChainError>(),
            Some(SubAgentChainError::CreateFreshFailed { .. })
        ));
        assert!(recording
            .created_for_sessions
            .lock()
            .expect("recording mutex")
            .is_empty());
    }

    #[tokio::test]
    async fn exact_resume_missing_fails_without_creating_fresh_chain() {
        let requested_chain_id = Uuid::new_v4();
        let recording = Arc::new(RecordingChainPersistence::new(Uuid::new_v4()));

        let error = run_restore(
            recording.clone(),
            Some(requested_chain_id.to_string()),
            "stage-run-test",
            Some(Uuid::new_v4()),
        )
        .await
        .expect_err("missing exact chain must fail closed");

        assert!(error.to_string().contains(&requested_chain_id.to_string()));
        assert!(
            recording
                .created_for_sessions
                .lock()
                .expect("recording mutex")
                .is_empty(),
            "exact resume miss must not create a fresh chain"
        );
    }

    #[tokio::test]
    async fn exact_resume_load_error_fails_without_creating_fresh_chain() {
        let requested_chain_id = Uuid::new_v4();
        let mut persistence = RecordingChainPersistence::new(Uuid::new_v4());
        persistence.exact_load = ExactLoad::Error;
        let recording = Arc::new(persistence);

        let error = run_restore(
            recording.clone(),
            Some(requested_chain_id.to_string()),
            "stage-run-test",
            Some(Uuid::new_v4()),
        )
        .await
        .expect_err("exact chain load error must fail closed");

        assert!(error.to_string().contains(&requested_chain_id.to_string()));
        assert!(
            recording
                .created_for_sessions
                .lock()
                .expect("recording mutex")
                .is_empty(),
            "exact resume load error must not create a fresh chain"
        );
    }

    #[tokio::test]
    async fn exact_resume_matching_session_and_agent_restores_history() {
        let chain_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut persistence = RecordingChainPersistence::new(Uuid::new_v4());
        persistence.exact_load = ExactLoad::Present(serialized_test_history());
        persistence.exact_session = Some(session_id);
        persistence.exact_agent = Some("enumerator".to_string());

        let (restored_chain_id, prior_messages) = run_restore(
            Arc::new(persistence),
            Some(chain_id.to_string()),
            "stage-run-test",
            Some(session_id),
        )
        .await
        .expect("matching exact scope restores");

        assert_eq!(restored_chain_id, Some(chain_id));
        assert_eq!(prior_messages.len(), 1);
    }

    #[tokio::test]
    async fn exact_resume_compacts_bulky_tool_output_before_provider_and_persists_it() {
        const MAX_EXPECTED_RESTORED_HISTORY_BYTES: usize = 64 * 1024;
        let chain_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let raw_history = bulky_worklist_history();
        let raw_bytes = serde_json::to_vec(&raw_history)
            .expect("raw history serializes")
            .len();
        assert!(
            raw_bytes > 2_000_000,
            "fixture must reproduce a bulky chain"
        );

        let mut persistence = RecordingChainPersistence::new(Uuid::new_v4());
        persistence.exact_load =
            ExactLoad::Present(serde_json::to_value(&raw_history).expect("history serializes"));
        persistence.exact_session = Some(session_id);
        persistence.exact_agent = Some("enumerator".to_string());
        let recording = Arc::new(persistence);

        let (_, restored) = run_restore(
            recording.clone(),
            Some(chain_id.to_string()),
            "stage-run-test",
            Some(session_id),
        )
        .await
        .expect("exact resume compacts before returning history");

        let restored_bytes = serde_json::to_vec(&restored)
            .expect("restored history serializes")
            .len();
        assert!(
            restored_bytes <= MAX_EXPECTED_RESTORED_HISTORY_BYTES,
            "restored provider history must be bounded, got {restored_bytes} bytes"
        );
        crate::executor_helpers::serialize_chat_history(&restored)
            .expect("compaction must preserve tool call/result pairing");
        let result = first_tool_result_json(&restored);
        assert_eq!(result["ready_to_submit"], false);
        assert_eq!(result["cell_summary"]["pending_cells"], 900);
        assert_eq!(
            result["next_action"],
            "Resume only the exact pending and partial roots from DB truth."
        );
        assert!(result["exact_origin_page"]
            .as_array()
            .is_some_and(|roots| !roots.is_empty()));
        let directives = repair_directive_texts(&restored);
        assert_eq!(
            directives.len(),
            1,
            "historical duplicate repair directives must collapse to the newest one"
        );
        assert!(directives[0].contains("generation=2"));
        assert!(directives[0].contains("1176 target/technique pairs"));
        assert!(directives[0].contains("Allowed next tools"));
        assert!(
            directives[0].len() <= 12 * 1024,
            "the retained repair directive must be a bounded projection"
        );

        let updates = recording.updates.lock().expect("recording mutex");
        assert_eq!(
            updates.len(),
            1,
            "compacted body must be durable before LLM I/O"
        );
        assert_eq!(updates[0], serde_json::to_value(&restored).unwrap());
    }

    #[tokio::test]
    async fn repeated_exact_resume_of_compacted_body_is_byte_stable_and_does_not_rewrite() {
        let chain_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut first = RecordingChainPersistence::new(Uuid::new_v4());
        first.exact_load = ExactLoad::Present(
            serde_json::to_value(bulky_worklist_history()).expect("history serializes"),
        );
        first.exact_session = Some(session_id);
        first.exact_agent = Some("enumerator".to_string());
        let first = Arc::new(first);
        let (_, restored_once) = run_restore(
            first.clone(),
            Some(chain_id.to_string()),
            "stage-run-test",
            Some(session_id),
        )
        .await
        .expect("first restore succeeds");
        let persisted = first
            .updates
            .lock()
            .expect("recording mutex")
            .first()
            .cloned()
            .expect("first restore persists compacted body");

        let mut second = RecordingChainPersistence::new(Uuid::new_v4());
        second.exact_load = ExactLoad::Present(persisted);
        second.exact_session = Some(session_id);
        second.exact_agent = Some("enumerator".to_string());
        let second = Arc::new(second);
        let (_, restored_twice) = run_restore(
            second.clone(),
            Some(chain_id.to_string()),
            "stage-run-test",
            Some(session_id),
        )
        .await
        .expect("second restore succeeds");

        assert_eq!(
            serde_json::to_vec(&restored_twice).unwrap(),
            serde_json::to_vec(&restored_once).unwrap(),
            "provider context must not grow across context-error retries"
        );
        assert!(
            second.updates.lock().expect("recording mutex").is_empty(),
            "already compacted durable history must not be rewritten"
        );
    }

    #[tokio::test]
    async fn exact_resume_wrong_session_or_agent_fails_closed() {
        let chain_id = Uuid::new_v4();
        let allowed_session = Uuid::new_v4();
        for (session_id, allowed_agent) in
            [(Uuid::new_v4(), "enumerator"), (allowed_session, "browser")]
        {
            let mut persistence = RecordingChainPersistence::new(Uuid::new_v4());
            persistence.exact_load = ExactLoad::Present(serialized_test_history());
            persistence.exact_session = Some(allowed_session);
            persistence.exact_agent = Some(allowed_agent.to_string());
            let recording = Arc::new(persistence);

            let error = run_restore(
                recording.clone(),
                Some(chain_id.to_string()),
                "stage-run-test",
                Some(session_id),
            )
            .await
            .expect_err("wrong exact scope must fail closed");

            assert!(error.to_string().contains(&chain_id.to_string()));
            assert!(recording
                .created_for_sessions
                .lock()
                .expect("recording mutex")
                .is_empty());
        }
    }

    #[tokio::test]
    async fn exact_resume_malformed_history_fails_closed() {
        let chain_id = Uuid::new_v4();
        let mut persistence = RecordingChainPersistence::new(Uuid::new_v4());
        persistence.exact_load = ExactLoad::Present(serde_json::json!({"legacy": "bad"}));

        let error = run_restore(
            Arc::new(persistence),
            Some(chain_id.to_string()),
            "stage-run-test",
            Some(Uuid::new_v4()),
        )
        .await
        .expect_err("malformed exact history must fail closed");

        assert!(error.to_string().contains("stored history is invalid"));
    }

    #[tokio::test]
    async fn exact_resume_dangling_tool_call_history_fails_closed() {
        let chain_id = Uuid::new_v4();
        let mut persistence = RecordingChainPersistence::new(Uuid::new_v4());
        persistence.exact_load = ExactLoad::Present(
            serde_json::to_value(dangling_tool_history()).expect("history serializes"),
        );

        let error = run_restore(
            Arc::new(persistence),
            Some(chain_id.to_string()),
            "stage-run-test",
            Some(Uuid::new_v4()),
        )
        .await
        .expect_err("semantically invalid exact history must fail closed");

        assert!(matches!(
            error.downcast_ref::<SubAgentChainError>(),
            Some(SubAgentChainError::ExactResumeUnavailable { .. })
        ));
        assert!(error.to_string().contains("stored history is invalid"));
    }

    #[tokio::test]
    async fn latest_resume_missing_or_load_error_fails_without_fresh_fallback() {
        for latest_load in [LatestLoad::Missing, LatestLoad::Error] {
            let mut persistence = RecordingChainPersistence::new(Uuid::new_v4());
            persistence.latest_load = latest_load;
            let recording = Arc::new(persistence);

            let error = run_restore(
                recording.clone(),
                Some("latest".to_string()),
                "stage-run-test",
                Some(Uuid::new_v4()),
            )
            .await
            .expect_err("explicit latest failure must fail closed");

            assert!(error.to_string().contains("latest sub-agent chain"));
            assert!(recording
                .created_for_sessions
                .lock()
                .expect("recording mutex")
                .is_empty());
        }
    }

    #[tokio::test]
    async fn latest_resume_success_restores_history() {
        let chain_id = Uuid::new_v4();
        let mut persistence = RecordingChainPersistence::new(Uuid::new_v4());
        persistence.latest_load = LatestLoad::Present(chain_id, serialized_test_history());

        let (restored_chain_id, prior_messages) = run_restore(
            Arc::new(persistence),
            Some("latest".to_string()),
            "stage-run-test",
            Some(Uuid::new_v4()),
        )
        .await
        .expect("latest chain restores");

        assert_eq!(restored_chain_id, Some(chain_id));
        assert_eq!(prior_messages.len(), 1);
    }

    #[tokio::test]
    async fn invalid_resume_selector_fails_without_fresh_fallback() {
        let recording = Arc::new(RecordingChainPersistence::new(Uuid::new_v4()));

        let error = run_restore(
            recording.clone(),
            Some("lateset".to_string()),
            "stage-run-test",
            Some(Uuid::new_v4()),
        )
        .await
        .expect_err("invalid explicit resume selector must fail closed");

        assert!(error.to_string().contains("invalid resume selector"));
        assert!(recording
            .created_for_sessions
            .lock()
            .expect("recording mutex")
            .is_empty());
    }

    #[tokio::test]
    async fn persist_failure_omits_resume_marker() {
        let chain_id = Uuid::new_v4();
        let mut persistence = RecordingChainPersistence::new(chain_id);
        persistence.fail_update = true;
        let recording = Arc::new(persistence);

        let durable_chain_id = run_persist(recording, chain_id)
            .await
            .expect_err("chain update failure must be observable");
        let response = append_durable_chain_marker("done".to_string(), None);

        assert!(durable_chain_id
            .to_string()
            .contains("synthetic chain update failure"));
        assert!(!response.contains("sub_agent_session_id"));
    }

    #[tokio::test]
    async fn persist_success_appends_resume_marker() {
        let chain_id = Uuid::new_v4();
        let recording = Arc::new(RecordingChainPersistence::new(chain_id));

        let durable_chain_id = run_persist(recording, chain_id)
            .await
            .expect("chain update succeeds");
        let response = append_durable_chain_marker("done".to_string(), durable_chain_id);

        assert!(response.contains(&format!("[sub_agent_session_id: {chain_id}]")));
    }

    #[tokio::test]
    async fn completed_batch_checkpoint_then_finalization_updates_usage_once() {
        let chain_id = Uuid::new_v4();
        let recording = Arc::new(RecordingChainPersistence::new(chain_id));

        run_checkpoint_with_history(recording.clone(), chain_id, completed_tool_batch_history())
            .await
            .expect("completed batch checkpoint succeeds");
        run_persist_with_history_and_duration(
            recording.clone(),
            chain_id,
            completed_tool_batch_history(),
            17,
        )
        .await
        .expect("final persistence succeeds");

        assert_eq!(recording.updates.lock().expect("recording mutex").len(), 2);
        assert_eq!(
            *recording.usage_updates.lock().expect("recording mutex"),
            vec![17],
            "a durable batch checkpoint must not duplicate the one final usage update"
        );
    }

    #[tokio::test]
    async fn checkpoint_rejects_partial_multi_tool_batch_without_database_update() {
        let chain_id = Uuid::new_v4();
        let recording = Arc::new(RecordingChainPersistence::new(chain_id));

        let error = run_checkpoint_with_history(
            recording.clone(),
            chain_id,
            partial_multi_tool_batch_history(),
        )
        .await
        .expect_err("a partial multi-tool batch must never become durable");

        assert!(matches!(
            error.downcast_ref::<SubAgentChainError>(),
            Some(SubAgentChainError::FinalizeFailed { .. })
        ));
        assert!(error.to_string().contains("call-provider-456"));
        assert!(recording
            .updates
            .lock()
            .expect("recording mutex")
            .is_empty());
        assert!(recording
            .usage_updates
            .lock()
            .expect("recording mutex")
            .is_empty());
    }

    #[tokio::test]
    async fn final_persist_applies_the_same_provider_history_budget() {
        let chain_id = Uuid::new_v4();
        let recording = Arc::new(RecordingChainPersistence::new(chain_id));

        run_persist_with_history(recording.clone(), chain_id, bulky_worklist_history())
            .await
            .expect("final persistence compacts before writing");

        let updates = recording.updates.lock().expect("recording mutex");
        assert_eq!(updates.len(), 1);
        let persisted: Vec<Message> =
            serde_json::from_value(updates[0].clone()).expect("persisted history decodes");
        assert!(serde_json::to_vec(&persisted).unwrap().len() <= 64 * 1024);
        crate::executor_helpers::serialize_chat_history(&persisted)
            .expect("final compaction preserves provider tool pairs");
        assert_eq!(repair_directive_texts(&persisted).len(), 1);
    }

    #[tokio::test]
    async fn persist_rejects_dangling_tool_call_before_database_update() {
        let chain_id = Uuid::new_v4();
        let recording = Arc::new(RecordingChainPersistence::new(chain_id));

        let error = run_persist_with_history(recording.clone(), chain_id, dangling_tool_history())
            .await
            .expect_err("invalid history must not be written");

        assert!(matches!(
            error.downcast_ref::<SubAgentChainError>(),
            Some(SubAgentChainError::FinalizeFailed { .. })
        ));
        assert!(recording
            .updates
            .lock()
            .expect("recording mutex")
            .is_empty());
    }
}
