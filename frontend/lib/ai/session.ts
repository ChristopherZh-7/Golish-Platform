import type { UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@/lib/api/client";
import { onEvent } from "@/lib/events";
import type {
  AiEvent,
  AiProvider,
  ProviderConfig,
  ReconToolCheck,
  SessionAiConfigInfo,
  WorkflowInfo,
} from "./types";

/**
 * Prefix used by silent title-generation sessions (no tools / sub-agents).
 *
 * Cross-process invariant: must stay in sync with
 * `backend/crates/golish-core/src/session_kind.rs::TITLE_GEN_SESSION_PREFIX`.
 */
export const TITLE_GEN_SESSION_PREFIX = "title-gen-";

/** Build a title-generation session id from a base id (e.g. conversation id). */
export function titleGenSessionId(base: string): string {
  return `${TITLE_GEN_SESSION_PREFIX}${base}`;
}

/** Returns `true` if the session id was minted for title generation. */
export function isTitleGenSessionId(sessionId: string): boolean {
  return sessionId.startsWith(TITLE_GEN_SESSION_PREFIX);
}

export async function retryCompaction(sessionId: string): Promise<void> {
  return invoke("retry_compaction", { sessionId });
}

export async function getAvailableWorkflows(): Promise<WorkflowInfo[]> {
  return invoke("list_workflows");
}

export async function setSubAgentModel(
  sessionId: string,
  agentId: string,
  provider: AiProvider | null,
  model: string | null
): Promise<void> {
  return invoke("set_sub_agent_model", { sessionId, agentId, provider, model });
}

export async function getSubAgentModel(
  sessionId: string,
  agentId: string
): Promise<[string, string] | null> {
  return invoke("get_sub_agent_model", { sessionId, agentId });
}

export async function clearSubAgentModel(sessionId: string, agentId: string): Promise<void> {
  return setSubAgentModel(sessionId, agentId, null, null);
}

export function onAiEvent(callback: (event: AiEvent) => void): Promise<UnlistenFn> {
  return onEvent("ai-event", callback);
}

export async function signalFrontendReady(sessionId: string): Promise<void> {
  return invoke("signal_frontend_ready", { sessionId });
}

export async function updateAiWorkspace(workspace: string, sessionId?: string): Promise<void> {
  return invoke("update_ai_workspace", { workspace, sessionId });
}

export async function clearAiConversation(sessionId: string): Promise<void> {
  return invoke("clear_ai_conversation", { sessionId });
}

export async function getAiConversationLength(sessionId: string): Promise<number> {
  return invoke("get_ai_conversation_length", { sessionId });
}

export async function initAiSession(sessionId: string, config: ProviderConfig): Promise<void> {
  await invoke("init_ai_session", { sessionId, config });
  await signalFrontendReady(sessionId);
}

export async function shutdownAiSession(sessionId: string): Promise<void> {
  return invoke("shutdown_ai_session", { sessionId });
}

export async function cancelAiGeneration(sessionId: string): Promise<void> {
  return invoke("cancel_ai_generation", { sessionId });
}

/**
 * Cancel a background job (a shell/pentest command that exceeded its soft
 * timeout and was detached to keep running). Resolves to `true` if the job was
 * found and a kill was requested. The originating tool card flips out of its
 * "backgrounded" state once the resulting `tool_background_completed` event
 * arrives.
 */
export async function cancelBackgroundJob(jobId: string): Promise<boolean> {
  return invoke("ai_cancel_background_job", { jobId });
}

export async function isAiSessionInitialized(sessionId: string): Promise<boolean> {
  return invoke("is_ai_session_initialized", { sessionId });
}

export async function getSessionAiConfig(sessionId: string): Promise<SessionAiConfigInfo | null> {
  return invoke("get_session_ai_config", { sessionId });
}

export async function sendPromptSession(sessionId: string, prompt: string): Promise<string> {
  return invoke("send_ai_prompt_session", { sessionId, prompt });
}

export async function startWorkflow(
  workflowName: string,
  input: Record<string, unknown>
): Promise<{ session_id: string; workflow_name: string }> {
  return invoke("start_workflow", { workflowName, input });
}

export async function runWorkflowToCompletion(sessionId: string): Promise<string> {
  return invoke("run_workflow_to_completion", { sessionId });
}

export async function runReconPipeline(
  targets: string[],
  projectName: string,
  projectPath: string,
  sessionId?: string
): Promise<string> {
  return invoke("run_recon_pipeline", {
    targets,
    projectName,
    projectPath,
    sessionId: sessionId ?? null,
  });
}

export async function checkReconTools(): Promise<ReconToolCheck> {
  return invoke("check_recon_tools_cmd");
}

export async function triggerAutoRecon(
  sessionId: string,
  targets: string[],
  projectName: string,
  projectPath: string = ""
): Promise<string> {
  const summary = await runReconPipeline(targets, projectName, projectPath, sessionId || undefined);
  console.log(`[recon] Pipeline complete for "${projectName}". Summary:\n`, summary);

  try {
    const { useStore } = await import("@/store");
    const store = useStore.getState();
    const activeConvId = store.activeConversationId;
    if (activeConvId) {
      const ts = Date.now();
      store.addConversationMessage(activeConvId, {
        id: `recon-summary-${ts}`,
        role: "assistant" as const,
        content: `**Recon Complete** — Project "${projectName}"\n\n${summary}`,
        timestamp: ts,
      });

      if (sessionId) {
        try {
          const aiReady = await isAiSessionInitialized(sessionId);
          if (aiReady) {
            const analysisPrompt =
              `Here are the reconnaissance results for project "${projectName}":\n\n` +
              `${summary}\n\n` +
              `Please analyze these findings, highlight any security concerns, and suggest concrete next steps for further investigation.`;
            store.addConversationMessage(activeConvId, {
              id: `recon-prompt-${ts}`,
              role: "user" as const,
              content: "Please analyze the findings and suggest next steps.",
              timestamp: ts + 1,
            });
            await sendPromptSession(sessionId, analysisPrompt);
          } else {
            console.log("[recon] AI session not initialized yet, skipping auto-analysis");
          }
        } catch (e) {
          console.warn("[recon] Failed to send recon summary to AI for analysis:", e);
        }
      }
    }
  } catch (e) {
    console.warn("[recon] Failed to inject recon results into chat:", e);
  }

  return summary;
}

export async function clearAiConversationSession(sessionId: string): Promise<void> {
  return invoke("clear_ai_conversation_session", { sessionId });
}

export async function getAiConversationLengthSession(sessionId: string): Promise<number> {
  return invoke("get_ai_conversation_length_session", { sessionId });
}

export async function getOpenRouterApiKey(): Promise<string | null> {
  return invoke("get_openrouter_api_key");
}

export async function loadEnvFile(path: string): Promise<number> {
  return invoke("load_env_file", { path });
}
