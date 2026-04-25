/**
 * Session slice for the Zustand store.
 *
 * Owns the per-session terminal/tab state: the `Session` records themselves,
 * the unified `timelines` (single source of truth for all on-screen blocks),
 * the live `streamingBlocks` interleave, the pending-command bookkeeping, and
 * the tab ordering / activation history. Session lifecycle (`addSession` /
 * `removeSession`) cascades into many other slice fields via untyped writes
 * so this slice never imports another slice.
 */

import { logger } from "@/lib/logger";
import { countLeafPanes, getAllLeafPanes } from "@/lib/pane-utils";
import { sendNotification } from "@/lib/systemNotifications";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import type {
  CommandBlock,
  ExecutionMode,
  InputMode,
  PendingCommand,
  RenderMode,
  Session,
  SessionMode,
  StreamingBlock,
  TabType,
  ToolCallSource,
  AgentMode,
  DetailViewMode,
  UnifiedBlock,
} from "../store-types";
import type { SliceCreator } from "./types";

const _outputBuffer = new Map<string, string>();
const MAX_OUTPUT_BUFFER_BYTES = 512 * 1024; // 512 KB

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

/**
 * Mark a tab as having new activity by resolving the owning tab from a
 * session id. Operates directly on the Immer draft to avoid nested set()
 * calls. Pulls `tabLayouts` (lives in the pane slice) via the merged state.
 */
function markTabNewActivityInDraft(state: any, sessionId: string): void {
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
function getOwningTabIdFromState(state: any, sessionId: string): string | null {
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
function purgeSessionStateInDraft(state: unknown, sessionId: string): void {
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

export const createSessionSlice: SliceCreator<SessionSlice> = (set, get) => ({
  ...initialSessionState,

  addSession: (session, options) =>
    set((state: any) => {
      const isPaneSession = options?.isPaneSession ?? false;

      state.sessions[session.id] = {
        ...session,
        logicalTerminalId: session.logicalTerminalId || crypto.randomUUID(),
        tabType: session.tabType ?? ("terminal" as TabType),
        inputMode: session.inputMode ?? "terminal",
      };

      if (!isPaneSession) {
        state.activeSessionId = session.id;
        const histIdx = state.tabActivationHistory.indexOf(session.id);
        if (histIdx !== -1) {
          state.tabActivationHistory.splice(histIdx, 1);
        }
        state.tabActivationHistory.push(session.id);

        if ((session.tabType ?? "terminal") === "terminal") {
          import("@/lib/tauri").then(({ setActiveTerminalSession }) => {
            setActiveTerminalSession(session.id).catch(() => {});
          });
        }
      }

      state.timelines[session.id] = [];
      state.pendingCommand[session.id] = null;
      state.lastSentCommand[session.id] = null;
      state.agentStreamingBuffer = state.agentStreamingBuffer ?? {};
      state.agentStreamingBuffer[session.id] = [];
      state.agentStreaming = state.agentStreaming ?? {};
      state.agentStreaming[session.id] = "";
      state.streamingBlocks[session.id] = [];
      state.streamingTextOffset[session.id] = 0;
      state.agentInitialized = state.agentInitialized ?? {};
      state.agentInitialized[session.id] = false;
      state.isAgentThinking = state.isAgentThinking ?? {};
      state.isAgentThinking[session.id] = false;
      state.isAgentResponding = state.isAgentResponding ?? {};
      state.isAgentResponding[session.id] = false;
      state.pendingToolApproval = state.pendingToolApproval ?? {};
      state.pendingToolApproval[session.id] = null;
      state.pendingAskHuman = state.pendingAskHuman ?? {};
      state.pendingAskHuman[session.id] = null;
      state.activeToolCalls = state.activeToolCalls ?? {};
      state.activeToolCalls[session.id] = [];
      state.thinkingContent = state.thinkingContent ?? {};
      state.thinkingContent[session.id] = "";
      state.isThinkingExpanded = state.isThinkingExpanded ?? {};
      state.isThinkingExpanded[session.id] = true;
      state.activeWorkflows = state.activeWorkflows ?? {};
      state.activeWorkflows[session.id] = null;
      state.workflowHistory = state.workflowHistory ?? {};
      state.workflowHistory[session.id] = [];
      state.activeSubAgents = state.activeSubAgents ?? {};
      state.activeSubAgents[session.id] = [];
      state.contextMetrics = state.contextMetrics ?? {};
      state.contextMetrics[session.id] = {
        utilization: 0,
        usedTokens: 0,
        maxTokens: 0,
        isWarning: false,
      };
      state.compactionCount = state.compactionCount ?? {};
      state.compactionCount[session.id] = 0;
      state.isCompacting = state.isCompacting ?? {};
      state.isCompacting[session.id] = false;
      state.isSessionDead = state.isSessionDead ?? {};
      state.isSessionDead[session.id] = false;
      state.compactionError = state.compactionError ?? {};
      state.compactionError[session.id] = null;
      state.gitStatus = state.gitStatus ?? {};
      state.gitStatus[session.id] = null;
      state.gitStatusLoading = state.gitStatusLoading ?? {};
      state.gitStatusLoading[session.id] = true;
      state.gitCommitMessage = state.gitCommitMessage ?? {};
      state.gitCommitMessage[session.id] = "";

      if (!isPaneSession) {
        state.tabLayouts = state.tabLayouts ?? {};
        state.tabLayouts[session.id] = {
          root: { type: "leaf", id: session.id, sessionId: session.id },
          focusedPaneId: session.id,
        };
        state.tabOrder.push(session.id);
        state.tabHasNewActivity[session.id] = false;
      }
    }),

  removeSession: (sessionId) => {
    TerminalInstanceManager.dispose(sessionId);
    _outputBuffer.delete(sessionId);

    import("@/hooks/useAiEvents").then(({ resetSessionSequence }) => {
      resetSessionSequence(sessionId);
    });

    set((state: any) => {
      purgeSessionStateInDraft(state, sessionId);
      if (state.tabLayouts) delete state.tabLayouts[sessionId];

      const tabOrderIdx = state.tabOrder.indexOf(sessionId);
      if (tabOrderIdx !== -1) {
        state.tabOrder.splice(tabOrderIdx, 1);
      }

      if (state.activeSessionId === sessionId) {
        state.tabActivationHistory = state.tabActivationHistory.filter(
          (id: string) => id !== sessionId,
        );
        state.activeSessionId =
          state.tabActivationHistory[state.tabActivationHistory.length - 1] ?? null;
      } else {
        state.tabActivationHistory = state.tabActivationHistory.filter(
          (id: string) => id !== sessionId,
        );
      }
    });
  },

  setActiveSession: (sessionId) =>
    set((state: any) => {
      state.activeSessionId = sessionId;
      state.tabHasNewActivity[sessionId] = false;
      const idx = state.tabActivationHistory.indexOf(sessionId);
      if (idx !== -1) {
        state.tabActivationHistory.splice(idx, 1);
      }
      state.tabActivationHistory.push(sessionId);

      if (state.conversationTerminals) {
        for (const [convId, terminals] of Object.entries<any>(state.conversationTerminals)) {
          if (terminals.includes(sessionId) && state.activeConversationId !== convId) {
            state.activeConversationId = convId;
            break;
          }
        }
      }

      const session = state.sessions[sessionId];
      if (session && (session.tabType ?? "terminal") === "terminal") {
        import("@/lib/tauri").then(({ setActiveTerminalSession }) => {
          setActiveTerminalSession(sessionId).catch(() => {});
        });
      }
    }),

  updateWorkingDirectory: (sessionId, path) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].workingDirectory = path;
      }
    }),

  updateVirtualEnv: (sessionId, name) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].virtualEnv = name;
      }
    }),

  updateGitBranch: (sessionId, branch) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].gitBranch = branch;
      }
    }),

  setSessionMode: (sessionId, mode) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].mode = mode;
      }
    }),

  setInputMode: (sessionId, mode) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].inputMode = mode;
      }
    }),

  setAgentMode: (sessionId, mode) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].agentMode = mode;
      }
    }),

  setUseAgents: (sessionId, enabled) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].useAgents = enabled;
      }
    }),

  setExecutionMode: (sessionId, mode) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].executionMode = mode;
      }
    }),

  setCustomTabName: (sessionId, customName) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].customName = customName ?? undefined;
      }
    }),

  setProcessName: (sessionId, processName) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        if (!state.sessions[sessionId].customName) {
          state.sessions[sessionId].processName = processName ?? undefined;
        }
      }
    }),

  setRenderMode: (sessionId, mode) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        logger.info("[store] setRenderMode:", {
          sessionId,
          from: state.sessions[sessionId].renderMode,
          to: mode,
        });
        state.sessions[sessionId].renderMode = mode;
      }
    }),

  setDetailViewMode: (sessionId, mode) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].detailViewMode = mode;
        if (mode !== "tool-detail") {
          state.sessions[sessionId].toolDetailRequestIds = null;
        }
      }
    }),

  setToolDetailRequestIds: (sessionId, requestIds) =>
    set((state) => {
      if (state.sessions[sessionId]) {
        state.sessions[sessionId].toolDetailRequestIds = requestIds;
      }
    }),

  handlePromptStart: (sessionId) => {
    const drainedOutput = _drainOutputBuffer(sessionId);
    set((state: any) => {
      const pending = state.pendingCommand[sessionId];
      if (pending?.command) {
        const session = state.sessions[sessionId];
        if (session?.renderMode === "fullterm") {
          state.pendingCommand[sessionId] = null;
          return;
        }
        const currentWorkingDir = session?.workingDirectory || pending.workingDirectory;
        const blockId = crypto.randomUUID();
        const block: CommandBlock = {
          id: blockId,
          sessionId,
          command: pending.command,
          output: drainedOutput,
          exitCode: null,
          startTime: pending.startTime,
          durationMs: null,
          workingDirectory: currentWorkingDir,
          isCollapsed: false,
        };

        if (!state.timelines[sessionId]) {
          state.timelines[sessionId] = [];
        }
        const source = state.pipelineCommandSource[sessionId]
          ? ("pipeline" as const)
          : undefined;
        state.timelines[sessionId].push({
          id: blockId,
          type: "command",
          timestamp: new Date().toISOString(),
          data: { ...block, source },
        });
      }

      if (pending) {
        markTabNewActivityInDraft(state, sessionId);
      }
      state.pendingCommand[sessionId] = null;
    });
  },

  handlePromptEnd: (_sessionId) => {
    // Ready for input - nothing to do for now
  },

  handleCommandStart: (sessionId, command) => {
    _outputBuffer.delete(sessionId);
    set((state) => {
      const session = state.sessions[sessionId];
      const effectiveCommand = command || state.lastSentCommand[sessionId] || null;
      state.pendingCommand[sessionId] = {
        command: effectiveCommand,
        output: "",
        startTime: new Date().toISOString(),
        workingDirectory: session?.workingDirectory || "",
      };
      state.lastSentCommand[sessionId] = null;
    });
  },

  handleCommandEnd: (sessionId, exitCode, endTime?) => {
    const currentState = get() as any;
    const pending = currentState.pendingCommand[sessionId];
    const command = pending?.command;
    const session = currentState.sessions[sessionId];
    const isFullterm = session?.renderMode === "fullterm";
    const shouldNotify = pending && command && !isFullterm;
    const drainedOutput = _drainOutputBuffer(sessionId);
    const owningTabId = shouldNotify ? getOwningTabIdFromState(currentState, sessionId) : null;

    set((state: any) => {
      const pending = state.pendingCommand[sessionId];
      if (pending) {
        const session = state.sessions[sessionId];
        const isFullterm = session?.renderMode === "fullterm";

        if (pending.command && !isFullterm) {
          const blockId = crypto.randomUUID();
          const currentWorkingDir = session?.workingDirectory || pending.workingDirectory;
          const block: CommandBlock = {
            id: blockId,
            sessionId,
            command: pending.command,
            output: drainedOutput,
            exitCode,
            startTime: pending.startTime,
            durationMs: (endTime ?? Date.now()) - new Date(pending.startTime).getTime(),
            workingDirectory: currentWorkingDir,
            isCollapsed: false,
          };

          if (!state.timelines[sessionId]) {
            state.timelines[sessionId] = [];
          }
          const source = state.pipelineCommandSource[sessionId]
            ? ("pipeline" as const)
            : undefined;
          state.timelines[sessionId].push({
            id: blockId,
            type: "command",
            timestamp: new Date().toISOString(),
            data: { ...block, source },
          });
          if (source === "pipeline") {
            state.pipelineCommandSource[sessionId] = false;
          }
        }

        markTabNewActivityInDraft(state, sessionId);

        state.pendingCommand[sessionId] = null;
      }
    });

    if (shouldNotify && owningTabId) {
      const exitStatus = exitCode === 0 ? "✓" : `✗ ${exitCode}`;
      sendNotification({
        title: "Command completed",
        body: `${exitStatus} ${command}`,
        tabId: owningTabId,
      }).catch((err) => {
        logger.debug("Failed to send command notification:", err);
      });
    }

    if (pending && command && pending.output) {
      window.dispatchEvent(
        new CustomEvent("tool-output-completed", {
          detail: { command, output: pending.output, sessionId },
        }),
      );
    }
  },

  appendOutput: (sessionId, data) => {
    let current = (_outputBuffer.get(sessionId) ?? "") + data;
    if (current.length > MAX_OUTPUT_BUFFER_BYTES * 2) {
      current = current.slice(current.length - MAX_OUTPUT_BUFFER_BYTES);
    }
    _outputBuffer.set(sessionId, current);
    if (!get().pendingCommand[sessionId]) {
      set((state) => {
        if (state.pendingCommand[sessionId]) return;
        const session = state.sessions[sessionId];
        state.pendingCommand[sessionId] = {
          command: null,
          output: "",
          startTime: new Date().toISOString(),
          workingDirectory: session?.workingDirectory || "",
        };
      });
    }
  },

  setPendingOutput: (sessionId, output) => {
    _outputBuffer.set(sessionId, output);
  },

  toggleBlockCollapse: (blockId) =>
    set((state) => {
      for (const timeline of Object.values(state.timelines)) {
        const unifiedBlock = timeline.find((b) => b.type === "command" && b.id === blockId);
        if (unifiedBlock && unifiedBlock.type === "command") {
          unifiedBlock.data.isCollapsed = !unifiedBlock.data.isCollapsed;
          break;
        }
      }
    }),

  setLastSentCommand: (sessionId, command) =>
    set((state) => {
      state.lastSentCommand[sessionId] = command;
    }),

  clearBlocks: (sessionId) => {
    _outputBuffer.delete(sessionId);
    set((state) => {
      const timeline = state.timelines[sessionId];
      if (timeline) {
        state.timelines[sessionId] = timeline.filter((block) => block.type !== "command");
      }
      state.pendingCommand[sessionId] = null;
    });
  },

  requestTerminalClear: (sessionId) =>
    set((state) => {
      state.terminalClearRequest[sessionId] = (state.terminalClearRequest[sessionId] ?? 0) + 1;
    }),

  setPipelineCommandSource: (sessionId, isPipeline) =>
    set((state) => {
      state.pipelineCommandSource[sessionId] = isPipeline;
    }),

  addStreamingToolBlock: (sessionId, toolCall) =>
    set((state) => {
      if (!state.streamingBlocks[sessionId]) {
        state.streamingBlocks[sessionId] = [];
      }
      state.streamingBlocks[sessionId].push({
        type: "tool",
        toolCall: {
          ...toolCall,
          status: "running",
          startedAt: new Date().toISOString(),
        },
      });
    }),

  updateStreamingToolBlock: (sessionId, toolId, success, result) =>
    set((state) => {
      const blocks = state.streamingBlocks[sessionId];
      if (blocks) {
        for (const block of blocks) {
          if (block.type === "tool" && block.toolCall.id === toolId) {
            block.toolCall.status = success ? "completed" : "error";
            block.toolCall.result = result;
            block.toolCall.completedAt = new Date().toISOString();
            break;
          }
        }
        state.streamingBlockRevision[sessionId] =
          (state.streamingBlockRevision[sessionId] ?? 0) + 1;
      }
    }),

  clearStreamingBlocks: (sessionId) =>
    set((state) => {
      state.streamingBlocks[sessionId] = [];
    }),

  addUdiffResultBlock: (sessionId, response, durationMs) =>
    set((state) => {
      if (!state.streamingBlocks[sessionId]) {
        state.streamingBlocks[sessionId] = [];
      }
      state.streamingBlocks[sessionId].push({
        type: "udiff_result",
        response,
        durationMs,
      });
    }),

  addStreamingSystemHooksBlock: (sessionId, hooks) =>
    set((state) => {
      if (!state.streamingBlocks[sessionId]) {
        state.streamingBlocks[sessionId] = [];
      }
      state.streamingBlocks[sessionId].push({
        type: "system_hooks",
        hooks,
      });
    }),

  appendToolStreamingOutput: (sessionId, toolId, chunk) =>
    set((state: any) => {
      const tools = state.activeToolCalls?.[sessionId];
      if (tools) {
        const toolIndex = tools.findIndex((t: any) => t.id === toolId);
        if (toolIndex !== -1) {
          tools[toolIndex] = {
            ...tools[toolIndex],
            streamingOutput: (tools[toolIndex].streamingOutput ?? "") + chunk,
          };
        }
      }
      const blocks = state.streamingBlocks[sessionId];
      if (blocks) {
        for (let i = 0; i < blocks.length; i++) {
          const block = blocks[i];
          if (block.type === "tool" && block.toolCall.id === toolId) {
            blocks[i] = {
              ...block,
              toolCall: {
                ...block.toolCall,
                streamingOutput: (block.toolCall.streamingOutput ?? "") + chunk,
              },
            };
            break;
          }
        }
        state.streamingBlockRevision[sessionId] =
          (state.streamingBlockRevision[sessionId] ?? 0) + 1;
      }
    }),

  addSystemHookBlock: (sessionId, hooks) =>
    set((state) => {
      if (!state.timelines[sessionId]) {
        state.timelines[sessionId] = [];
      }
      state.timelines[sessionId].push({
        id: crypto.randomUUID(),
        type: "system_hook",
        timestamp: new Date().toISOString(),
        data: { hooks },
      });
    }),

  clearTimeline: (sessionId) => {
    _outputBuffer.delete(sessionId);
    set((state: any) => {
      state.timelines[sessionId] = [];
      state.pendingCommand[sessionId] = null;
      if (state.agentStreamingBuffer) state.agentStreamingBuffer[sessionId] = [];
      if (state.agentStreaming) state.agentStreaming[sessionId] = "";
      state.streamingBlocks[sessionId] = [];
    });
  },

  openSettingsTab: () =>
    set((state: any) => {
      const existingSettingsTab = Object.values<any>(state.sessions).find(
        (session) => session.tabType === "settings",
      );

      if (existingSettingsTab) {
        state.activeSessionId = existingSettingsTab.id;
        state.tabHasNewActivity[existingSettingsTab.id] = false;
        const histIdx = state.tabActivationHistory.indexOf(existingSettingsTab.id);
        if (histIdx !== -1) {
          state.tabActivationHistory.splice(histIdx, 1);
        }
        state.tabActivationHistory.push(existingSettingsTab.id);
        return;
      }

      const settingsId = `settings-${Date.now()}`;
      state.sessions[settingsId] = {
        id: settingsId,
        tabType: "settings",
        name: "Settings",
        workingDirectory: "",
        createdAt: new Date().toISOString(),
        mode: "terminal",
      };
      state.activeSessionId = settingsId;
      state.tabLayouts = state.tabLayouts ?? {};
      state.tabLayouts[settingsId] = {
        root: { type: "leaf", id: settingsId, sessionId: settingsId },
        focusedPaneId: settingsId,
      };
      state.tabHasNewActivity[settingsId] = false;
      state.tabOrder.push(settingsId);
      state.tabActivationHistory.push(settingsId);
    }),

  openHomeTab: () =>
    set((state: any) => {
      const existingHomeTab = Object.values<any>(state.sessions).find(
        (session) => session.tabType === "home",
      );

      if (existingHomeTab) {
        state.activeSessionId = existingHomeTab.id;
        state.tabHasNewActivity[existingHomeTab.id] = false;
        const histIdx = state.tabActivationHistory.indexOf(existingHomeTab.id);
        if (histIdx !== -1) {
          state.tabActivationHistory.splice(histIdx, 1);
        }
        state.tabActivationHistory.push(existingHomeTab.id);
        return;
      }

      const homeId = `home-${Date.now()}`;
      state.sessions[homeId] = {
        id: homeId,
        tabType: "home",
        name: "Home",
        workingDirectory: "",
        createdAt: new Date().toISOString(),
        mode: "terminal",
      };
      state.activeSessionId = homeId;
      state.homeTabId = homeId;
      state.tabLayouts = state.tabLayouts ?? {};
      state.tabLayouts[homeId] = {
        root: { type: "leaf", id: homeId, sessionId: homeId },
        focusedPaneId: homeId,
      };
      state.tabHasNewActivity[homeId] = false;
      state.tabOrder.unshift(homeId);
      state.tabActivationHistory.push(homeId);
    }),

  openBrowserTab: (url?: string) =>
    set((state: any) => {
      const existingBrowserTab = Object.values<any>(state.sessions).find(
        (session) => session.tabType === "browser",
      );

      if (existingBrowserTab) {
        state.activeSessionId = existingBrowserTab.id;
        state.tabHasNewActivity[existingBrowserTab.id] = false;
        const histIdx = state.tabActivationHistory.indexOf(existingBrowserTab.id);
        if (histIdx !== -1) {
          state.tabActivationHistory.splice(histIdx, 1);
        }
        state.tabActivationHistory.push(existingBrowserTab.id);
        return;
      }

      const browserId = `browser-${Date.now()}`;
      state.sessions[browserId] = {
        id: browserId,
        tabType: "browser",
        name: "Browser",
        workingDirectory: url || "",
        createdAt: new Date().toISOString(),
        mode: "terminal",
      };
      state.activeSessionId = browserId;
      state.tabLayouts = state.tabLayouts ?? {};
      state.tabLayouts[browserId] = {
        root: { type: "leaf", id: browserId, sessionId: browserId },
        focusedPaneId: browserId,
      };
      state.tabHasNewActivity[browserId] = false;
      state.tabOrder.push(browserId);
      state.tabActivationHistory.push(browserId);
    }),

  openSecurityTab: () =>
    set((state: any) => {
      const existingTab = Object.values<any>(state.sessions).find(
        (session) => session.tabType === "security",
      );

      if (existingTab) {
        state.activeSessionId = existingTab.id;
        state.tabHasNewActivity[existingTab.id] = false;
        const histIdx = state.tabActivationHistory.indexOf(existingTab.id);
        if (histIdx !== -1) {
          state.tabActivationHistory.splice(histIdx, 1);
        }
        state.tabActivationHistory.push(existingTab.id);
        return;
      }

      const securityId = `security-${Date.now()}`;
      state.sessions[securityId] = {
        id: securityId,
        tabType: "security",
        name: "Security",
        workingDirectory: "",
        createdAt: new Date().toISOString(),
        mode: "terminal",
      };
      state.activeSessionId = securityId;
      state.tabLayouts = state.tabLayouts ?? {};
      state.tabLayouts[securityId] = {
        root: { type: "leaf", id: securityId, sessionId: securityId },
        focusedPaneId: securityId,
      };
      state.tabHasNewActivity[securityId] = false;
      state.tabOrder.push(securityId);
      state.tabActivationHistory.push(securityId);
    }),

  getTabSessionIds: (tabId) => {
    const layout = (get() as any).tabLayouts?.[tabId];
    if (!layout) return [];
    return getAllLeafPanes(layout.root).map((pane) => pane.sessionId);
  },

  closeTab: (tabId) => {
    const currentState = get() as any;
    const layout = currentState.tabLayouts?.[tabId];
    const sessionIdsToClean: string[] = [];

    if (!layout) {
      sessionIdsToClean.push(tabId);
    } else {
      const panes = getAllLeafPanes(layout.root);
      for (const pane of panes) {
        sessionIdsToClean.push(pane.sessionId);
      }
    }

    for (const sessionId of sessionIdsToClean) {
      TerminalInstanceManager.dispose(sessionId);
    }

    import("@/hooks/useAiEvents").then(({ resetSessionSequence }) => {
      for (const sessionId of sessionIdsToClean) {
        resetSessionSequence(sessionId);
      }
    });

    set((state: any) => {
      const layout = state.tabLayouts?.[tabId];
      if (!layout) {
        purgeSessionStateInDraft(state, tabId);

        state.tabActivationHistory = state.tabActivationHistory.filter(
          (id: string) => id !== tabId,
        );
        if (state.activeSessionId === tabId) {
          state.activeSessionId =
            state.tabActivationHistory[state.tabActivationHistory.length - 1] ?? null;
        }
        return;
      }

      const panes = getAllLeafPanes(layout.root);
      for (const pane of panes) {
        purgeSessionStateInDraft(state, pane.sessionId);
      }

      delete state.tabLayouts[tabId];
      delete state.tabHasNewActivity[tabId];
      const tabOrderIdx = state.tabOrder.indexOf(tabId);
      if (tabOrderIdx !== -1) {
        state.tabOrder.splice(tabOrderIdx, 1);
      }

      state.tabActivationHistory = state.tabActivationHistory.filter(
        (id: string) => id !== tabId,
      );
      if (state.activeSessionId === tabId) {
        state.activeSessionId =
          state.tabActivationHistory[state.tabActivationHistory.length - 1] ?? null;
      }
    });
  },

  markTabNewActivityBySession: (sessionId) =>
    set((state: any) => {
      markTabNewActivityInDraft(state, sessionId);
    }),

  clearTabNewActivity: (tabId) =>
    set((state) => {
      state.tabHasNewActivity[tabId] = false;
    }),

  moveTab: (tabId, direction) =>
    set((state) => {
      const idx = state.tabOrder.indexOf(tabId);
      if (idx === -1) return;
      if (idx === 0) return;
      const targetIdx = direction === "left" ? idx - 1 : idx + 1;
      if (targetIdx < 1 || targetIdx >= state.tabOrder.length) return;
      const temp = state.tabOrder[targetIdx];
      state.tabOrder[targetIdx] = state.tabOrder[idx];
      state.tabOrder[idx] = temp;
    }),

  reorderTab: (draggedTabId, targetTabId) =>
    set((state) => {
      if (draggedTabId === targetTabId) return;
      const fromIdx = state.tabOrder.indexOf(draggedTabId);
      const toIdx = state.tabOrder.indexOf(targetTabId);
      if (fromIdx < 1 || toIdx < 1) return;
      state.tabOrder.splice(fromIdx, 1);
      state.tabOrder.splice(toIdx, 0, draggedTabId);
    }),

  moveTabToPane: (sourceTabId, destTabId, location) =>
    set((state: any) => {
      logger.info("[store] moveTabToPane: start", {
        sourceTabId,
        destTabId,
        location,
      });
      const sourceLayout = state.tabLayouts?.[sourceTabId];
      const destLayout = state.tabLayouts?.[destTabId];
      if (!sourceLayout || !destLayout) {
        logger.warn("[store] moveTabToPane: missing layout", {
          hasSourceLayout: !!sourceLayout,
          hasDestLayout: !!destLayout,
        });
        return;
      }
      const sourceSession = state.sessions[sourceTabId];
      if (!sourceSession) {
        logger.warn("[store] moveTabToPane: source session missing", { sourceTabId });
        return;
      }
      const sourceTabType = sourceSession.tabType ?? "terminal";
      if (sourceTabType !== "terminal") {
        logger.warn("[store] moveTabToPane: source not terminal", {
          sourceTabId,
          sourceTabType,
        });
        return;
      }
      const destSession = state.sessions[destTabId];
      if (!destSession) {
        logger.warn("[store] moveTabToPane: destination session missing", { destTabId });
        return;
      }
      const destTabType = destSession.tabType ?? "terminal";
      if (destTabType !== "terminal") {
        logger.warn("[store] moveTabToPane: destination not terminal", {
          destTabId,
          destTabType,
        });
        return;
      }
      const destPaneCount = countLeafPanes(destLayout.root);
      const sourcePaneCount = countLeafPanes(sourceLayout.root);
      if (destPaneCount + sourcePaneCount > 4) {
        logger.warn("[store] moveTabToPane: pane limit exceeded", {
          destPaneCount,
          sourcePaneCount,
        });
        return;
      }

      const direction = location === "left" || location === "right" ? "vertical" : "horizontal";
      const newPaneId = crypto.randomUUID();

      if (location === "right" || location === "bottom") {
        state.tabLayouts[destTabId].root = {
          type: "split",
          id: crypto.randomUUID(),
          direction,
          children: [destLayout.root, { type: "leaf", id: newPaneId, sessionId: sourceTabId }],
          ratio: 0.5,
        };
      } else {
        state.tabLayouts[destTabId].root = {
          type: "split",
          id: crypto.randomUUID(),
          direction,
          children: [{ type: "leaf", id: newPaneId, sessionId: sourceTabId }, destLayout.root],
          ratio: 0.5,
        };
      }

      delete state.tabLayouts[sourceTabId];

      const tabOrderIdx = state.tabOrder.indexOf(sourceTabId);
      if (tabOrderIdx !== -1) {
        state.tabOrder.splice(tabOrderIdx, 1);
      }

      delete state.tabHasNewActivity[sourceTabId];

      if (state.activeSessionId === sourceTabId) {
        state.activeSessionId = destTabId;
        const histIdx = state.tabActivationHistory.indexOf(destTabId);
        if (histIdx !== -1) {
          state.tabActivationHistory.splice(histIdx, 1);
        }
        state.tabActivationHistory.push(destTabId);
      }

      state.tabLayouts[destTabId].focusedPaneId = newPaneId;
      logger.info("[store] moveTabToPane: completed", {
        sourceTabId,
        destTabId,
        newPaneId,
        direction,
      });
    }),
});

export const selectActiveSessionId = <T extends SessionState>(state: T): string | null =>
  state.activeSessionId;

export const selectSession = <T extends SessionState>(state: T, sessionId: string) =>
  state.sessions[sessionId];

export const selectTabOrder = <T extends SessionState>(state: T): string[] => state.tabOrder;
