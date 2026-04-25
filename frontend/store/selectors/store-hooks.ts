/**
 * Thin store-hook selectors.
 *
 * Each hook is a 1–3-line `useStore((state) => …)` wrapper around a single
 * slice/field of the unified store. They were previously inlined at the
 * bottom of `store/index.ts`; lifted out here so the store factory file
 * stays focused on slice composition.
 *
 * For richer, memoised combined selectors see the sibling files
 * (`session.ts`, `app.ts`, `tab-bar.ts`, …) in this directory.
 */

import { findPaneById, getAllLeafPanes } from "@/lib/pane-utils";
import {
  memoizedSelectAgentMessages,
  memoizedSelectCommandBlocks,
} from "@/lib/timeline/selectors";

import {
  selectActiveConversation,
  selectActiveConversationTerminals,
  selectAllConversations,
  selectContextMetrics,
} from "../slices";
import {
  type ActiveToolCall,
  type StreamingBlock,
  type UnifiedBlock,
  useStore,
} from "../index";

// ─── Stable empty arrays ─────────────────────────────────────────────────
//
// Frozen so component-level reference equality holds across re-renders and
// any accidental mutation panics in dev. Use these in `??` fall-throughs
// instead of `[]` to avoid a fresh reference on every selector call.
const EMPTY_TIMELINE = Object.freeze([]) as unknown as UnifiedBlock[];
const EMPTY_TOOL_CALLS = Object.freeze([]) as unknown as ActiveToolCall[];
const EMPTY_STREAMING_BLOCKS = Object.freeze([]) as unknown as StreamingBlock[];

// ─── Session ─────────────────────────────────────────────────────────────

export const useActiveSession = () =>
  useStore((state) => {
    const id = state.activeSessionId;
    return id ? state.sessions[id] : null;
  });

export const useSessionMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.mode ?? "terminal");

export const useInputMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.inputMode ?? "terminal");

export const useAgentMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.agentMode ?? "default");

export const useUseAgents = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.useAgents ?? true);

export const useExecutionMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.executionMode ?? "chat");

export const useRenderMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.renderMode ?? "timeline");

// ─── Terminal ────────────────────────────────────────────────────────────

export const usePendingCommand = (sessionId: string) =>
  useStore((state) => state.pendingCommand[sessionId]);

export const useTerminalClearRequest = (sessionId: string) =>
  useStore((state) => state.terminalClearRequest[sessionId] ?? 0);

// ─── Timeline ────────────────────────────────────────────────────────────

export const useSessionTimeline = (sessionId: string) =>
  useStore((state) => state.timelines[sessionId] ?? EMPTY_TIMELINE);

/**
 * Get command blocks for a session.
 * Derives from the unified timeline (Single Source of Truth).
 */
export const useSessionBlocks = (sessionId: string) =>
  useStore((state) => memoizedSelectCommandBlocks(sessionId, state.timelines[sessionId]));

/**
 * Get agent messages for a session.
 * Derives from the unified timeline (Single Source of Truth).
 */
export const useAgentMessages = (sessionId: string) =>
  useStore((state) => memoizedSelectAgentMessages(sessionId, state.timelines[sessionId]));

// ─── Agent state ─────────────────────────────────────────────────────────

export const useAgentStreaming = (sessionId: string) =>
  useStore((state) => state.agentStreaming[sessionId] ?? "");

export const useAgentInitialized = (sessionId: string) =>
  useStore((state) => state.agentInitialized[sessionId] ?? false);

export const usePendingToolApproval = (sessionId: string) =>
  useStore((state) => state.pendingToolApproval[sessionId] ?? null);

export const usePendingAskHuman = (sessionId: string) =>
  useStore((state) => state.pendingAskHuman[sessionId] ?? null);

export const useActiveToolCalls = (sessionId: string) =>
  useStore((state) => state.activeToolCalls[sessionId] ?? EMPTY_TOOL_CALLS);

export const useStreamingBlocks = (sessionId: string) =>
  useStore((state) => state.streamingBlocks[sessionId] ?? EMPTY_STREAMING_BLOCKS);

/** Streaming text length — primarily for auto-scroll triggers. */
export const useStreamingTextLength = (sessionId: string) =>
  useStore((state) => state.agentStreaming[sessionId]?.length ?? 0);

export const useIsAgentThinking = (sessionId: string) =>
  useStore((state) => state.isAgentThinking[sessionId] ?? false);

/** True from "agent started" through "agent completed". */
export const useIsAgentResponding = (sessionId: string) =>
  useStore((state) => state.isAgentResponding[sessionId] ?? false);

// Extended thinking content (for models like Opus 4.5).
export const useThinkingContent = (sessionId: string) =>
  useStore((state) => state.thinkingContent[sessionId] ?? "");

export const useIsThinkingExpanded = (sessionId: string) =>
  useStore((state) => state.isThinkingExpanded[sessionId] ?? true);

// ─── AI config ───────────────────────────────────────────────────────────

/** Global AI config selector (kept for backwards compatibility). */
export const useAiConfig = () => useStore((state) => state.aiConfig);

export const useSessionAiConfig = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.aiConfig);

// ─── Git (per-session) ───────────────────────────────────────────────────

export const useGitBranch = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.gitBranch ?? null);

export const useGitStatus = (sessionId: string) =>
  useStore((state) => state.gitStatus[sessionId] ?? null);

export const useGitStatusLoading = (sessionId: string) =>
  useStore((state) => state.gitStatusLoading[sessionId] ?? false);

export const useGitCommitMessage = (sessionId: string) =>
  useStore((state) => state.gitCommitMessage[sessionId] ?? "");

// ─── Context metrics ─────────────────────────────────────────────────────

export const useContextMetrics = (sessionId: string) =>
  useStore((state) => selectContextMetrics(state, sessionId));

// ─── Conversation ────────────────────────────────────────────────────────

export const useActiveConversation = () =>
  useStore((state) => selectActiveConversation(state));

export const useAllConversations = () =>
  useStore((state) => selectAllConversations(state));

export const useActiveConversationTerminals = () =>
  useStore((state) => selectActiveConversationTerminals(state));

// ─── Pane / tab focus helpers ────────────────────────────────────────────

/**
 * Get the session ID of the currently focused pane.
 * Falls back to tabId if no layout exists (backward compatibility).
 *
 * Uses a module-level cache keyed on (tabId, focusedPaneId) to avoid
 * running findPaneById() on every store mutation. This prevents App
 * from re-rendering unless the focused pane actually changes.
 */
let _focusedSessionCache: {
  tabId: string | null;
  focusedPaneId: string | null;
  result: string | null;
} | null = null;

export const useFocusedSessionId = (tabId: string | null) =>
  useStore((state) => {
    if (!tabId) return null;
    const layout = state.tabLayouts[tabId];
    if (!layout) return tabId; // Fallback to tab session for backward compatibility

    // Short-circuit if inputs haven't changed
    if (
      _focusedSessionCache &&
      _focusedSessionCache.tabId === tabId &&
      _focusedSessionCache.focusedPaneId === layout.focusedPaneId
    ) {
      return _focusedSessionCache.result;
    }

    const pane = findPaneById(layout.root, layout.focusedPaneId);
    const result = pane?.type === "leaf" ? pane.sessionId : tabId;
    _focusedSessionCache = { tabId, focusedPaneId: layout.focusedPaneId, result };
    return result;
  });

/**
 * Get the owning tab ID for a given session ID.
 *
 * A session may be the tab's root session, or a pane session within the tab.
 * Returns the tab ID (root session ID) that contains this session, or null
 * if the session is not found in any tab.
 */
export function getOwningTabId(sessionId: string): string | null {
  const state = useStore.getState();

  // Check if this sessionId is itself a tab (root session)
  if (state.tabLayouts[sessionId]) {
    return sessionId;
  }

  // Search through all tab layouts to find which one contains this session
  for (const [tabId, layout] of Object.entries(state.tabLayouts)) {
    const panes = getAllLeafPanes(layout.root);
    if (panes.some((pane) => pane.sessionId === sessionId)) {
      return tabId;
    }
  }

  return null;
}
