/**
 * Session core actions: lifecycle (add / remove / switch) and property setters.
 */

import type { StageRunRow, StageRunSummary } from "@/components/Engagement/StageRunOrgRows";
import { logger } from "@/lib/logger";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import { resetSessionSequence } from "@/services/ai-events/session-sequence";
import type {
  AgentMode,
  DetailViewMode,
  ExecutionMode,
  InputMode,
  RenderMode,
  Session,
  SessionMode,
  SessionStageRun,
  TabType,
} from "../store-types";
import type { SessionStoreDraft } from "./session-draft-types";
import { deleteOutputBuffer, purgeSessionStateInDraft } from "./session-helpers";
import type { ImmerSet, StateGet } from "./types";

/** Recompute a stage-run summary from its per-org rows (status tallies). */
function computeStageRunSummary(rows: StageRunRow[]): StageRunSummary {
  return {
    total: rows.length,
    covered: rows.filter((r) => r.status === "passed").length,
    active: rows.filter((r) => r.status === "running").length,
    queued: rows.filter((r) => r.status === "queued").length,
    blocked: rows.filter((r) => r.status === "blocked").length,
  };
}

function latestStageRunRequestId(state: SessionStoreDraft, sessionId: string): string | undefined {
  const timeline = state.timelines[sessionId] ?? [];
  for (let i = timeline.length - 1; i >= 0; i--) {
    const block = timeline[i];
    if (block.type === "ai_tool_execution" && block.data.toolName === "stage_run") {
      return block.data.requestId;
    }
  }
  return undefined;
}

function emptyStageRun(
  requestId: string | undefined,
  meta: {
    stageLabel: string;
    roleLabel: string;
    coverageAxis: string[];
  }
): SessionStageRun {
  return {
    rows: [],
    summary: { total: 0, covered: 0, active: 0, queued: 0, blocked: 0 },
    stageLabel: meta.stageLabel,
    roleLabel: meta.roleLabel,
    coverageAxis: meta.coverageAxis,
    requestId,
  };
}

export function createSessionCoreActions(
  set: ImmerSet<SessionStoreDraft>,
  _get: StateGet<SessionStoreDraft>
) {
  return {
    addSession: (session: Session, options?: { isPaneSession?: boolean }) =>
      set((state) => {
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
            // Catch the dynamic import too — partial mocks that omit
            // `setActiveTerminalSession` (common in unit tests) would otherwise
            // surface as noisy unhandled rejections.
            import("@/lib/api/pty")
              .then(({ setActiveTerminalSession }) => {
                setActiveTerminalSession?.(session.id).catch(() => {});
              })
              .catch(() => {});
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

    removeSession: (sessionId: string) => {
      TerminalInstanceManager.dispose(sessionId);
      deleteOutputBuffer(sessionId);

      resetSessionSequence(sessionId);

      set((state) => {
        purgeSessionStateInDraft(state, sessionId);
        if (state.tabLayouts) delete state.tabLayouts[sessionId];

        const tabOrderIdx = state.tabOrder.indexOf(sessionId);
        if (tabOrderIdx !== -1) {
          state.tabOrder.splice(tabOrderIdx, 1);
        }

        if (state.activeSessionId === sessionId) {
          state.tabActivationHistory = state.tabActivationHistory.filter(
            (id: string) => id !== sessionId
          );
          state.activeSessionId =
            state.tabActivationHistory[state.tabActivationHistory.length - 1] ?? null;
        } else {
          state.tabActivationHistory = state.tabActivationHistory.filter(
            (id: string) => id !== sessionId
          );
        }
      });
    },

    setActiveSession: (sessionId: string) =>
      set((state) => {
        state.activeSessionId = sessionId;
        state.tabHasNewActivity[sessionId] = false;
        const idx = state.tabActivationHistory.indexOf(sessionId);
        if (idx !== -1) {
          state.tabActivationHistory.splice(idx, 1);
        }
        state.tabActivationHistory.push(sessionId);

        if (state.conversationTerminals) {
          for (const [convId, terminals] of Object.entries(state.conversationTerminals)) {
            if (terminals.includes(sessionId) && state.activeConversationId !== convId) {
              state.activeConversationId = convId;
              break;
            }
          }
        }

        const session = state.sessions[sessionId];
        if (session && (session.tabType ?? "terminal") === "terminal") {
          // See note above on the import("@/lib/api/pty") swallow — keeps
          // partial test mocks from emitting unhandled rejections.
          import("@/lib/api/pty")
            .then(({ setActiveTerminalSession }) => {
              setActiveTerminalSession?.(sessionId).catch(() => {});
            })
            .catch(() => {});
        }
      }),

    // --- Property setters ---

    updateWorkingDirectory: (sessionId: string, path: string) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].workingDirectory = path;
        }
      }),

    updateVirtualEnv: (sessionId: string, name: string | null) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].virtualEnv = name;
        }
      }),

    setSessionMode: (sessionId: string, mode: SessionMode) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].mode = mode;
        }
      }),

    setInputMode: (sessionId: string, mode: InputMode) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].inputMode = mode;
        }
      }),

    setAgentMode: (sessionId: string, mode: AgentMode) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].agentMode = mode;
        }
      }),

    setExecutionMode: (sessionId: string, mode: ExecutionMode) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].executionMode = mode;
        }
      }),

    setCustomTabName: (sessionId: string, customName: string | null) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].customName = customName ?? undefined;
        }
      }),

    setProcessName: (sessionId: string, processName: string | null) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          if (!state.sessions[sessionId].customName) {
            state.sessions[sessionId].processName = processName ?? undefined;
          }
        }
      }),

    setRenderMode: (sessionId: string, mode: RenderMode) =>
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

    setDetailViewMode: (sessionId: string, mode: DetailViewMode) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].detailViewMode = mode;
          if (mode !== "tool-detail" && mode !== "sub-agent-detail") {
            state.sessions[sessionId].toolDetailRequestIds = null;
          }
        }
      }),

    setToolDetailRequestIds: (sessionId: string, requestIds: string[] | null) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].toolDetailRequestIds = requestIds;
        }
      }),

    setSessionStageRun: (sessionId: string, stageRun: SessionStageRun | null) =>
      set((state) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].stageRun = stageRun;
          if (!stageRun) return;
          if (stageRun.requestId) {
            state.sessions[sessionId].stageRuns = state.sessions[sessionId].stageRuns ?? {};
            state.sessions[sessionId].stageRuns[stageRun.requestId] = stageRun;
          }
        }
      }),

    upsertStageRunRow: (
      sessionId: string,
      row: StageRunRow,
      meta: {
        stageLabel: string;
        roleLabel: string;
        coverageAxis: string[];
        requestId?: string | null;
      }
    ) =>
      set((state) => {
        const sess = state.sessions[sessionId];
        if (!sess) return;

        const requestId =
          meta.requestId ?? sess.stageRun?.requestId ?? latestStageRunRequestId(state, sessionId);
        const latestRequestId = latestStageRunRequestId(state, sessionId);

        let sr: SessionStageRun;
        if (requestId) {
          sess.stageRuns = sess.stageRuns ?? {};
          sr = sess.stageRuns[requestId] ?? emptyStageRun(requestId, meta);
          sess.stageRuns[requestId] = sr;
        } else {
          sr = sess.stageRun ?? emptyStageRun(undefined, meta);
        }

        // The first real frame carries the stage labels/axis — keep them fresh.
        if (meta.stageLabel) sr.stageLabel = meta.stageLabel;
        if (meta.roleLabel) sr.roleLabel = meta.roleLabel;
        if (meta.coverageAxis.length) sr.coverageAxis = meta.coverageAxis;
        if (requestId) sr.requestId = requestId;

        const idx = sr.rows.findIndex((r) => r.id === row.id);
        if (idx >= 0) {
          sr.rows[idx] = row;
        } else {
          sr.rows.push(row);
        }
        sr.summary = computeStageRunSummary(sr.rows);

        if (!requestId || !latestRequestId || latestRequestId === requestId) {
          sess.stageRun = sr;
        }
      }),
  };
}
