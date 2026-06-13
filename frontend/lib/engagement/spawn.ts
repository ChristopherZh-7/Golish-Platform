/**
 * Programmatic worker-session spawning (设计 2026-06-13-engagement-scoping-
 * fanout §6.1, Phase B).
 *
 * Replays the same primitive sequence a human doing "new chat → pick model →
 * task mode → type the task" triggers, with two worker extras: auto-approve
 * (unattended) and the pinned engagement worker scope (hard org + stage-slice
 * constraints). Pure module — every store access goes through
 * `useStore.getState()` (same pattern as `useCreateTerminalTab`), so the pool
 * loop can run outside the component tree.
 */

import { buildProviderConfig } from "@/components/AIChatPanel/providerConfig";
import { initAiSession, sendPromptSession, setAgentMode, setExecutionMode } from "@/lib/ai";
import {
  engagementSetWorkerScope,
  type EngagementWorkerScope as WorkerScopeDto,
} from "@/lib/api/engagement";
import { ptyCreate } from "@/lib/api/pty";
import { logger } from "@/lib/logger";
import { getSettings } from "@/lib/settings";
import { useStore } from "@/store";
import { createNewConversation } from "@/store/slices/conversation";
import {
  buildWorkerPrompt,
  classifyWorkerError,
  STAGE_SLICES,
  type WorkerOutcome,
  type WorkerUnit,
  workerTitle,
} from "./pool";
import { writeEngagementRole } from "./rolePersistence";

export interface SpawnWorkerOptions {
  /** Harness profile id the worker runs under (e.g. "red_team"). */
  profileId: string;
  model: string;
  provider: string;
  /** Recon family workers include subsidiaries; attack workers don't. */
  thresholdPct: number;
}

/**
 * Spawn a worker conversation for `unit`, seed its task, and block until the
 * worker's operation finishes (the Task-mode `sendPromptSession` resolves when
 * the stage slice completes; rejects on a BLOCK terminal or hard failure).
 *
 * Returns the worker's terminal outcome — never throws (failures are mapped
 * to `blocked` / `failed` outcomes so one worker can't take down the pool).
 */
export async function spawnWorkerAndRun(
  unit: WorkerUnit,
  opts: SpawnWorkerOptions
): Promise<WorkerOutcome> {
  const store = useStore.getState();

  // 1) Conversation shell (worker-tagged for tabs/overview).
  const conv = createNewConversation();
  conv.title = workerTitle(unit);
  conv.engagementRole = "worker";
  conv.workerMeta = { unitId: unit.id, unitKind: unit.kind, orgName: unit.orgName };
  store.addConversation(conv);
  writeEngagementRole(conv.id, {
    engagementRole: "worker",
    workerMeta: conv.workerMeta,
  });

  try {
    // 2) Owned terminal (1:1 conversation↔terminal model). Mirrors
    //    useCreateTerminalTab with skipConversationLink=true.
    try {
      const session = await ptyCreate(undefined);
      useStore.getState().addSession({
        id: session.id,
        logicalTerminalId: crypto.randomUUID(),
        name: "Terminal",
        workingDirectory: session.working_directory,
        createdAt: new Date().toISOString(),
        mode: "terminal",
      });
      useStore.getState().addTerminalToConversation(conv.id, session.id);
    } catch (e) {
      logger.warn("[engagement] worker terminal creation failed (continuing)", e);
    }

    // 3) AI session init with the caller's model/provider.
    const settings = await getSettings();
    const workspace = useStore.getState().currentProjectPath || ".";
    const providerConfig = buildProviderConfig(opts.provider, opts.model, workspace, settings);
    if (!providerConfig) {
      return { unitId: unit.id, status: "failed", detail: "no provider config (model not set)" };
    }
    await initAiSession(conv.aiSessionId, providerConfig);
    useStore.getState().updateConversation(conv.id, { aiInitialized: true });

    // 4) Unattended worker: auto-approve + task engine with the profile.
    await setAgentMode(conv.aiSessionId, "auto-approve");
    await setExecutionMode(conv.aiSessionId, opts.profileId);

    // 5) Pin the hard constraints BEFORE the seed prompt.
    const slice = STAGE_SLICES[unit.kind];
    const scope: WorkerScopeDto = {
      orgId: unit.orgId,
      from: slice.from,
      to: slice.to,
      includeSubsidiaries: unit.kind === "recon_family",
      subsidiaryThresholdPct: opts.thresholdPct,
    };
    await engagementSetWorkerScope(conv.aiSessionId, scope);

    // 6) Seed the task into the chat (visible) and run to completion.
    const prompt = buildWorkerPrompt(unit, {
      includeSubsidiaries: scope.includeSubsidiaries,
      thresholdPct: opts.thresholdPct,
    });
    useStore.getState().addConversationMessage(conv.id, {
      id: `user-${Date.now()}`,
      role: "user",
      content: prompt,
      timestamp: Date.now(),
    });
    useStore.getState().setConversationStreaming(conv.id, true);

    try {
      await sendPromptSession(conv.aiSessionId, prompt);
      return { unitId: unit.id, status: "passed" };
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      return { unitId: unit.id, status: classifyWorkerError(message), detail: message };
    } finally {
      useStore.getState().finalizeStreamingMessage(conv.id);
    }
  } catch (err) {
    // Setup-phase failure (init/mode/scope) — report, don't throw.
    const message = err instanceof Error ? err.message : String(err);
    logger.error(`[engagement] worker spawn failed for ${unit.id}:`, err);
    return { unitId: unit.id, status: "failed", detail: `spawn failed: ${message}` };
  }
}

/** Conversation id of the worker spawned for a unit, if present. */
export function findWorkerConvId(unitId: string): string | null {
  const state = useStore.getState();
  for (const conv of Object.values(state.conversations)) {
    if (conv.workerMeta?.unitId === unitId) return conv.id;
  }
  return null;
}
