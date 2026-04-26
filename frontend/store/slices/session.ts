/**
 * Session slice for the Zustand store.
 *
 * Owns the per-session terminal/tab state: the `Session` records themselves,
 * the unified `timelines` (single source of truth for all on-screen blocks),
 * the live `streamingBlocks` interleave, the pending-command bookkeeping, and
 * the tab ordering / activation history. Session lifecycle (`addSession` /
 * `removeSession`) cascades into many other slice fields via untyped writes
 * so this slice never imports another slice.
 *
 * Implementation is split across sub-modules by responsibility:
 *   session-core.ts      — lifecycle (add / remove / switch) + property setters
 *   session-terminal.ts  — command handling, timeline, output buffers
 *   session-streaming.ts — streaming-block management
 *   session-tabs.ts      — tab / pane management
 *   session-helpers.ts   — shared helpers (purge, activity draft, output buffer)
 */

import type {
  AgentMode,
  DetailViewMode,
  ExecutionMode,
  InputMode,
  PendingCommand,
  RenderMode,
  Session,
  SessionMode,
  StreamingBlock,
  ToolCallSource,
  UnifiedBlock,
} from "../store-types";
import type { SliceCreator } from "./types";

// Re-export output-buffer helpers (consumed by store/index.ts)
export { _drainOutputBuffer, _drainOutputBufferSize } from "./session-helpers";

// Sub-module creators
import { createSessionCoreActions } from "./session-core";
import { createSessionStreamingActions } from "./session-streaming";
import { createSessionTabActions } from "./session-tabs";
import { createSessionTerminalActions } from "./session-terminal";

// ---------------------------------------------------------------------------
// State & Action interfaces (unchanged public API)
// ---------------------------------------------------------------------------

export interface SessionState {
  sessions: Record<string, Session>;
  activeSessionId: string | null;
  homeTabId: string | null;
  /** Unified timeline blocks per session (Single Source of Truth). */
  timelines: Record<string, UnifiedBlock[]>;
  /** Interleaved live-streaming blocks (text/tool/thinking) per session. */
  streamingBlocks: Record<string, StreamingBlock[]>;
  /** Tracks how much streaming text has been assigned to blocks. */
  streamingTextOffset: Record<string, number>;
  /** Bumped when streaming blocks are mutated in-place (auto-scroll trigger). */
  streamingBlockRevision: Record<string, number>;
  /** Currently-pending command block per session. */
  pendingCommand: Record<string, PendingCommand | null>;
  /** Last command actually sent to the PTY per session. */
  lastSentCommand: Record<string, string | null>;
  /** Marks the next CommandBlock as pipeline-sourced. */
  pipelineCommandSource: Record<string, boolean>;
  /** Bumped to ask the terminal renderer to clear (per session). */
  terminalClearRequest: Record<string, number>;
  /** Ordered list of tab IDs (home tab always at index 0 when present). */
  tabOrder: string[];
  /** MRU tab activation history (most-recent last). */
  tabActivationHistory: string[];
  /** True for tabs with new activity since last activation. */
  tabHasNewActivity: Record<string, boolean>;
}

export interface SessionActions {
  addSession: (session: Session, options?: { isPaneSession?: boolean }) => void;
  removeSession: (sessionId: string) => void;
  setActiveSession: (sessionId: string) => void;

  updateWorkingDirectory: (sessionId: string, path: string) => void;
  updateVirtualEnv: (sessionId: string, name: string | null) => void;
  updateGitBranch: (sessionId: string, branch: string | null) => void;
  setSessionMode: (sessionId: string, mode: SessionMode) => void;
  setInputMode: (sessionId: string, mode: InputMode) => void;
  setAgentMode: (sessionId: string, mode: AgentMode) => void;
  setUseAgents: (sessionId: string, enabled: boolean) => void;
  setExecutionMode: (sessionId: string, mode: ExecutionMode) => void;
  setCustomTabName: (sessionId: string, customName: string | null) => void;
  setProcessName: (sessionId: string, processName: string | null) => void;
  setRenderMode: (sessionId: string, mode: RenderMode) => void;
  setDetailViewMode: (sessionId: string, mode: DetailViewMode) => void;
  setToolDetailRequestIds: (sessionId: string, requestIds: string[] | null) => void;

  // Terminal lifecycle
  handlePromptStart: (sessionId: string) => void;
  handlePromptEnd: (sessionId: string) => void;
  handleCommandStart: (sessionId: string, command: string | null) => void;
  handleCommandEnd: (sessionId: string, exitCode: number, endTime?: number) => void;
  appendOutput: (sessionId: string, data: string) => void;
  setPendingOutput: (sessionId: string, output: string) => void;
  toggleBlockCollapse: (blockId: string) => void;
  setLastSentCommand: (sessionId: string, command: string | null) => void;
  clearBlocks: (sessionId: string) => void;
  requestTerminalClear: (sessionId: string) => void;
  setPipelineCommandSource: (sessionId: string, isPipeline: boolean) => void;

  // Streaming-block helpers
  addStreamingToolBlock: (
    sessionId: string,
    toolCall: {
      id: string;
      name: string;
      args: Record<string, unknown>;
      executedByAgent?: boolean;
      source?: ToolCallSource;
    },
  ) => void;
  updateStreamingToolBlock: (
    sessionId: string,
    toolId: string,
    success: boolean,
    result?: unknown,
  ) => void;
  clearStreamingBlocks: (sessionId: string) => void;
  addUdiffResultBlock: (sessionId: string, response: string, durationMs: number) => void;
  addStreamingSystemHooksBlock: (sessionId: string, hooks: string[]) => void;
  appendToolStreamingOutput: (sessionId: string, toolId: string, chunk: string) => void;

  // Timeline helpers
  addSystemHookBlock: (sessionId: string, hooks: string[]) => void;
  clearTimeline: (sessionId: string) => void;

  // Tab/pane bridge actions
  openSettingsTab: () => void;
  openHomeTab: () => void;
  openBrowserTab: (url?: string) => void;
  openSecurityTab: () => void;
  getTabSessionIds: (tabId: string) => string[];
  closeTab: (tabId: string) => void;
  markTabNewActivityBySession: (sessionId: string) => void;
  clearTabNewActivity: (tabId: string) => void;
  moveTab: (tabId: string, direction: "left" | "right") => void;
  reorderTab: (draggedTabId: string, targetTabId: string) => void;
  moveTabToPane: (
    sourceTabId: string,
    destTabId: string,
    location: "left" | "right" | "top" | "bottom",
  ) => void;
}

export interface SessionSlice extends SessionState, SessionActions {}

// ---------------------------------------------------------------------------
// Initial state
// ---------------------------------------------------------------------------

export const initialSessionState: SessionState = {
  sessions: {},
  activeSessionId: null,
  homeTabId: null,
  timelines: {},
  streamingBlocks: {},
  streamingTextOffset: {},
  streamingBlockRevision: {},
  pendingCommand: {},
  lastSentCommand: {},
  pipelineCommandSource: {},
  terminalClearRequest: {},
  tabOrder: [],
  tabActivationHistory: [],
  tabHasNewActivity: {},
};

// ---------------------------------------------------------------------------
// Slice creator (composes sub-modules)
// ---------------------------------------------------------------------------

export const createSessionSlice: SliceCreator<SessionSlice> = (set, get) => ({
  ...initialSessionState,
  ...createSessionCoreActions(set, get),
  ...createSessionTerminalActions(set, get),
  ...createSessionStreamingActions(set, get),
  ...createSessionTabActions(set, get),
});

// ---------------------------------------------------------------------------
// Selectors
// ---------------------------------------------------------------------------

export const selectActiveSessionId = <T extends SessionState>(state: T): string | null =>
  state.activeSessionId;

export const selectSession = <T extends SessionState>(state: T, sessionId: string) =>
  state.sessions[sessionId];

export const selectTabOrder = <T extends SessionState>(state: T): string[] => state.tabOrder;
