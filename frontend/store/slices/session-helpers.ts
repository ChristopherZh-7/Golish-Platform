/**
 * Shared helpers for the session slice sub-modules.
 *
 * Contains the output buffer, session-state purge logic, and tab-activity
 * helpers that are referenced from multiple session sub-modules.
 */

import { getAllLeafPanes } from "@/lib/pane-utils";

// ---------------------------------------------------------------------------
// Output buffer (module-scoped, shared across terminal / core actions)
// ---------------------------------------------------------------------------

const _outputBuffer = new Map<string, string>();
const MAX_OUTPUT_BUFFER_BYTES = 512 * 1024; // 512 KB

export { MAX_OUTPUT_BUFFER_BYTES };

export function _drainOutputBufferSize(sessionId: string): number {
  return _outputBuffer.get(sessionId)?.length ?? 0;
}

export function _drainOutputBuffer(sessionId: string): string {
  let buf = _outputBuffer.get(sessionId) ?? "";
  _outputBuffer.delete(sessionId);
  if (buf.length > MAX_OUTPUT_BUFFER_BYTES) {
    buf =
      `[Output truncated, showing last ${MAX_OUTPUT_BUFFER_BYTES} bytes]\n` +
      buf.slice(buf.length - MAX_OUTPUT_BUFFER_BYTES);
  }
  return buf;
}

export function getOutputBuffer(sessionId: string): string {
  return _outputBuffer.get(sessionId) ?? "";
}

export function setOutputBuffer(sessionId: string, value: string): void {
  _outputBuffer.set(sessionId, value);
}

export function deleteOutputBuffer(sessionId: string): void {
  _outputBuffer.delete(sessionId);
}

// ---------------------------------------------------------------------------
// Draft helpers (operate on the Immer draft directly)
// ---------------------------------------------------------------------------

/**
 * Mark a tab as having new activity by resolving the owning tab from a
 * session id. Operates directly on the Immer draft to avoid nested set()
 * calls. Pulls `tabLayouts` (lives in the pane slice) via the merged state.
 */
export function markTabNewActivityInDraft(state: any, sessionId: string): void {
  const tabLayouts = state.tabLayouts as Record<string, { root: any }> | undefined;
  if (!tabLayouts) return;
  for (const [tabId, layout] of Object.entries(tabLayouts)) {
    const leaves = getAllLeafPanes(layout.root);
    if (leaves.some((leaf) => leaf.sessionId === sessionId)) {
      const rootSession = state.sessions[tabId];
      const isTerminalTab = (rootSession?.tabType ?? "terminal") === "terminal";
      if (isTerminalTab && state.activeSessionId !== tabId) {
        state.tabHasNewActivity[tabId] = true;
      }
      return;
    }
  }
}

/**
 * Resolve the owning tab id for a session id. Used outside Immer (in
 * `handleCommandEnd` notification dispatch). Reads `tabLayouts` from the
 * passed-in state snapshot.
 */
export function getOwningTabIdFromState(state: any, sessionId: string): string | null {
  if (state.tabLayouts?.[sessionId]) return sessionId;
  for (const [tabId, layout] of Object.entries<any>(state.tabLayouts ?? {})) {
    const panes = getAllLeafPanes(layout.root);
    if (panes.some((pane: any) => pane.sessionId === sessionId)) return tabId;
  }
  return null;
}

/**
 * Drop every per-session field associated with the given session id. Called
 * during session/tab teardown. Works against the merged state via untyped
 * writes so this slice does not depend on other slices.
 */
export function purgeSessionStateInDraft(state: unknown, sessionId: string): void {
  const s = state as Record<string, Record<string, unknown> | undefined>;
  for (const key of [
    "sessions",
    "timelines",
    "pendingCommand",
    "lastSentCommand",
    "agentStreamingBuffer",
    "agentStreaming",
    "streamingBlocks",
    "streamingTextOffset",
    "streamingBlockRevision",
    "agentInitialized",
    "isAgentThinking",
    "isAgentResponding",
    "pendingToolApproval",
    "pendingAskHuman",
    "processedToolRequests",
    "activeToolCalls",
    "thinkingContent",
    "isThinkingExpanded",
    "gitStatus",
    "gitStatusLoading",
    "gitCommitMessage",
    "contextMetrics",
    "compactionCount",
    "isCompacting",
    "isSessionDead",
    "compactionError",
    "tabHasNewActivity",
  ]) {
    const bucket = s[key];
    if (bucket && typeof bucket === "object") {
      delete bucket[sessionId];
    }
  }
}
