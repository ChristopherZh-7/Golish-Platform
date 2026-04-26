/**
 * Session core actions: lifecycle (add / remove / switch) and property setters.
 */

import { logger } from "@/lib/logger";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import type {
  AgentMode,
  DetailViewMode,
  ExecutionMode,
  InputMode,
  RenderMode,
  Session,
  SessionMode,
  TabType,
} from "../store-types";
import { deleteOutputBuffer, purgeSessionStateInDraft } from "./session-helpers";
import type { ImmerSet, StateGet } from "./types";

export function createSessionCoreActions(set: ImmerSet<any>, get: StateGet<any>) {
  return {
    addSession: (session: Session, options?: { isPaneSession?: boolean }) =>
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

    removeSession: (sessionId: string) => {
      TerminalInstanceManager.dispose(sessionId);
      deleteOutputBuffer(sessionId);

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

    setActiveSession: (sessionId: string) =>
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

    // --- Property setters ---

    updateWorkingDirectory: (sessionId: string, path: string) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].workingDirectory = path;
        }
      }),

    updateVirtualEnv: (sessionId: string, name: string | null) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].virtualEnv = name;
        }
      }),

    updateGitBranch: (sessionId: string, branch: string | null) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].gitBranch = branch;
        }
      }),

    setSessionMode: (sessionId: string, mode: SessionMode) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].mode = mode;
        }
      }),

    setInputMode: (sessionId: string, mode: InputMode) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].inputMode = mode;
        }
      }),

    setAgentMode: (sessionId: string, mode: AgentMode) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].agentMode = mode;
        }
      }),

    setUseAgents: (sessionId: string, enabled: boolean) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].useAgents = enabled;
        }
      }),

    setExecutionMode: (sessionId: string, mode: ExecutionMode) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].executionMode = mode;
        }
      }),

    setCustomTabName: (sessionId: string, customName: string | null) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].customName = customName ?? undefined;
        }
      }),

    setProcessName: (sessionId: string, processName: string | null) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          if (!state.sessions[sessionId].customName) {
            state.sessions[sessionId].processName = processName ?? undefined;
          }
        }
      }),

    setRenderMode: (sessionId: string, mode: RenderMode) =>
      set((state: any) => {
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
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].detailViewMode = mode;
          if (mode !== "tool-detail") {
            state.sessions[sessionId].toolDetailRequestIds = null;
          }
        }
      }),

    setToolDetailRequestIds: (sessionId: string, requestIds: string[] | null) =>
      set((state: any) => {
        if (state.sessions[sessionId]) {
          state.sessions[sessionId].toolDetailRequestIds = requestIds;
        }
      }),
  };
}
