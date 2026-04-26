/**
 * When the app is about to close, clear in-memory "live run" state so reopening
 * is not stuck as "running". Uses the same `userStoppedGeneration` path as the
 * Stop button (task plan, sub-agents, timeline, plan-pipeline block) so it stays
 * in sync with manual cancel — including `flush-state` (Tauri) + `beforeunload`
 * save.
 */
import {
  discardAllPendingBatchedDeltas,
  discardPendingBatchedDeltasForAiSession,
} from "@/lib/ai/streaming-buffer";
import { cancelAiGeneration, suppressGenerationForAiSession } from "@/lib/ai";
import { getAllLeafPanes } from "@/lib/pane-utils";
import { useStore } from "@/store";

export type ResetLiveAiUiReason = "window_closing";

function collectSessionIds(): Set<string> {
  const s = useStore.getState() as any;
  const ids = new Set<string>();
  for (const k of Object.keys(s.sessions ?? {})) ids.add(k);
  for (const terms of Object.values(s.conversationTerminals ?? {})) {
    for (const t of terms as string[]) ids.add(t);
  }
  for (const layout of Object.values(s.tabLayouts ?? {})) {
    if (layout && (layout as any).root) {
      for (const p of getAllLeafPanes((layout as any).root)) ids.add(p.sessionId);
    }
  }
  return ids;
}

/**
 * Resets chat streaming flags and, for every known terminal session, applies
 * the same UI finalization as Stop (`userStoppedGeneration`).
 */
export function resetLiveAiUiState(opts: {
  reason: ResetLiveAiUiReason;
  cancelBackend?: boolean;
}): void {
  const { cancelBackend = true } = opts;
  const store = useStore.getState() as any;

  discardAllPendingBatchedDeltas();

  for (const conv of Object.values(store.conversations) as {
    id: string;
    isStreaming: boolean;
    aiSessionId: string;
  }[]) {
    if (!conv.isStreaming) continue;
    if (conv.aiSessionId) {
      suppressGenerationForAiSession(conv.aiSessionId);
      discardPendingBatchedDeltasForAiSession(conv.aiSessionId);
      if (cancelBackend) {
        void cancelAiGeneration(conv.aiSessionId);
      }
    }
    store.finalizeStreamingMessage(conv.id);
  }

  const sessionIds = collectSessionIds();
  for (const sid of sessionIds) {
    store.userStoppedGeneration(sid);
  }

  for (const sid of sessionIds) {
    store.clearAgentStreaming(sid);
    store.setAgentResponding(sid, false);
    store.setAgentThinking(sid, false);
    store.clearActiveToolCalls(sid);
    store.clearThinkingContent(sid);
  }
}
