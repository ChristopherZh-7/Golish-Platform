import { enableMapSet } from "immer";
import { create } from "zustand";
import { devtools } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import { logger } from "@/lib/logger";
import {
  countLeafPanes,
  findPaneById,
  getAllLeafPanes,
  getFirstLeafPane,
  getPaneNeighbor,
  insertPaneAtPosition,
  type PaneId,
  type PaneNode,
  removePaneNode,
  type SplitDirection,
  splitPaneNode,
  type TabLayout,
  updatePaneRatio,
} from "@/lib/pane-utils";
import { sendNotification } from "@/lib/systemNotifications";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import {
  type AppearanceSlice,
  type ChatConversation,
  type ChatMessage,
  type ContextMetrics,
  type ContextSlice,
  type ConversationSlice,
  createAppearanceSlice,
  createContextSlice,
  createConversationSlice,
  createGitSlice,
  createNotificationSlice,
  createPanelSlice,
  type GitSlice,
  type Notification,
  type NotificationSlice,
  type NotificationType,
  type PanelSlice,
  selectActiveConversation,
  selectActiveConversationTerminals,
  selectAllConversations,
  selectContextMetrics,
} from "./slices";
import type {
  ActiveSubAgent,
  ActiveToolCall,
  ActiveWorkflow,
  AgentMessage,
  AgentMode,
  AiConfig,
  AiStatus,
  AskHumanRequest,
  CommandBlock,
  DetailViewMode,
  InputMode,
  PendingCommand,
  PipelineExecution,
  PipelineStepExecution,
  PipelineStepStatus,
  RenderMode,
  Session,
  SessionMode,
  StepStatus,
  StreamingBlock,
  TaskPlan,
  ToolCall,
  ToolCallSource,
  UnifiedBlock,
} from "./store-types";

// Re-export all domain types from store-types
export type {
  ActiveSubAgent,
  ActiveToolCall,
  ActiveWorkflow,
  AgentMessage,
  AiConfig,
  AiToolExecution,
  AskHumanRequest,
  CommandBlock,
  CompactionResult,
  DetailViewMode,
  FinalizedStreamingBlock,
  PendingCommand,
  PipelineExecution,
  PipelineStepExecution,
  PipelineStepStatus,
  PipelineSubTarget,
  Session,
  StreamingBlock,
  SubAgentEntry,
  SubAgentToolCall,
  TaskPlan,
  ToolCall,
  ToolCallSource,
  UnifiedBlock,
  WorkflowStep,
} from "./store-types";
export type {
  AgentMode,
  AiStatus,
  ApprovalPattern,
  InputMode,
  PlanStep,
  PlanSummary,
  ReasoningEffort,
  RenderMode,
  RiskLevel,
  SessionMode,
  StepStatus,
  TabType,
  WorkflowStatus,
} from "./store-types";
// Re-export pane types from the single source of truth
export type { PaneId, PaneNode, SplitDirection, TabLayout };
// Re-export conversation types
export type { ChatConversation, ChatMessage };
// Re-export slice types
export type { ContextMetrics, Notification, NotificationType };

// Enable Immer support for Set and Map (needed for processedToolRequests)
enableMapSet();

/**
 * Helper function to mark a tab as having new activity from within a draft state.
 * This avoids nested set() calls by operating directly on the draft state.
 */
function markTabNewActivityInDraft(state: GolishState, sessionId: string): void {
  // Find owning tab by checking tabLayouts for a leaf matching sessionId
  for (const [tabId, layout] of Object.entries(state.tabLayouts)) {
    const leaves = getAllLeafPanes(layout.root);
    if (leaves.some((leaf) => leaf.sessionId === sessionId)) {
      // Found the owning tab
      const rootSession = state.sessions[tabId];
      const isTerminalTab = (rootSession?.tabType ?? "terminal") === "terminal";
      if (isTerminalTab && state.activeSessionId !== tabId) {
        state.tabHasNewActivity[tabId] = true;
      }
      return;
    }
  }
}

interface GolishState
  extends AppearanceSlice,
    ContextSlice,
    ConversationSlice,
    GitSlice,
    NotificationSlice,
    PanelSlice {
  // App focus/visibility state
  appIsFocused: boolean;
  appIsVisible: boolean;

  // Terminal restore loading state
  terminalRestoreInProgress: boolean;
  setTerminalRestoreInProgress: (inProgress: boolean) => void;

  // Workspace data has been hydrated into localStorage from workspace.json
  workspaceDataReady: boolean;
  setWorkspaceDataReady: (ready: boolean) => void;

  // Sessions
  sessions: Record<string, Session>;
  activeSessionId: string | null;
  homeTabId: string | null;

  // Current project
  currentProjectName: string | null;
  currentProjectPath: string | null;

  // AI configuration
  aiConfig: AiConfig;

  // Unified timeline - single source of truth for all blocks
  timelines: Record<string, UnifiedBlock[]>;

  // Terminal state
  pendingCommand: Record<string, PendingCommand | null>;
  lastSentCommand: Record<string, string | null>;
  /** When true, next command block for this session will be tagged as pipeline-sourced */
  pipelineCommandSource: Record<string, boolean>;

  // Agent state
  /** Buffer of text deltas per session - avoids string concatenation in hot path */
  agentStreamingBuffer: Record<string, string[]>;
  /** Cached joined streaming text - call getAgentStreamingText() for current value */
  agentStreaming: Record<string, string>;
  streamingBlocks: Record<string, StreamingBlock[]>; // Interleaved text and tool blocks
  streamingTextOffset: Record<string, number>; // Tracks how much text has been assigned to blocks
  streamingBlockRevision: Record<string, number>; // Increments when blocks are modified in-place (for auto-scroll)
  agentInitialized: Record<string, boolean>;
  isAgentThinking: Record<string, boolean>; // True when waiting for first content from agent
  isAgentResponding: Record<string, boolean>; // True when agent is actively responding (from started to completed)
  pendingToolApproval: Record<string, ToolCall | null>;
  pendingAskHuman: Record<string, AskHumanRequest | null>;
  processedToolRequests: Record<string, Set<string>>; // Track processed request IDs per session to prevent duplicates
  activeToolCalls: Record<string, ActiveToolCall[]>; // Tool calls currently in progress per session

  // Extended thinking state (for models like Opus 4.5)
  thinkingContent: Record<string, string>; // Accumulated thinking content per session
  isThinkingExpanded: Record<string, boolean>; // Whether thinking section is expanded

  // Workflow state
  activeWorkflows: Record<string, ActiveWorkflow | null>; // Active workflow per session
  workflowHistory: Record<string, ActiveWorkflow[]>; // Completed workflows per session

  // Sub-agent state
  activeSubAgents: Record<string, ActiveSubAgent[]>;
  subAgentBatchCounter: Record<string, number>;
  /** Maps parentRequestId → pipeline block/step so sub-agent updates sync to the pipeline */
  subAgentPipelineMap: Record<string, { blockId: string; stepId: string }>;

  // Terminal clear request (incremented to trigger clear)
  terminalClearRequest: Record<string, number>;

  // Pane layouts for multi-pane support (keyed by tab's root session ID)
  tabLayouts: Record<string, TabLayout>;

  // Ordered list of tab IDs (home tab always at index 0)
  tabOrder: string[];

  // Tab activation history for MRU tab switching (most recent last)
  tabActivationHistory: string[];

  // Tab activity indicator: true when tab has new output since last activation
  tabHasNewActivity: Record<string, boolean>;

  // Pane move mode state (active when user is choosing a drop target)
  paneMoveState: {
    tabId: string;
    sourcePaneId: PaneId;
    sourceSessionId: string;
  } | null;

  // App focus/visibility actions
  setAppIsFocused: (focused: boolean) => void;
  setAppIsVisible: (visible: boolean) => void;

  // Session actions
  addSession: (session: Session, options?: { isPaneSession?: boolean }) => void;
  removeSession: (sessionId: string) => void;
  setActiveSession: (sessionId: string) => void;
  updateWorkingDirectory: (sessionId: string, path: string) => void;
  updateVirtualEnv: (sessionId: string, name: string | null) => void;
  updateGitBranch: (sessionId: string, branch: string | null) => void;
  setSessionMode: (sessionId: string, mode: SessionMode) => void;
  setInputMode: (sessionId: string, mode: InputMode) => void;
  setAgentMode: (sessionId: string, mode: AgentMode) => void;
  setCustomTabName: (sessionId: string, customName: string | null) => void;
  setProcessName: (sessionId: string, processName: string | null) => void;
  setRenderMode: (sessionId: string, mode: RenderMode) => void;

  // Terminal actions
  handlePromptStart: (sessionId: string) => void;
  handlePromptEnd: (sessionId: string) => void;
  handleCommandStart: (sessionId: string, command: string | null) => void;
  handleCommandEnd: (sessionId: string, exitCode: number) => void;
  appendOutput: (sessionId: string, data: string) => void;
  setPendingOutput: (sessionId: string, output: string) => void;
  toggleBlockCollapse: (blockId: string) => void;
  setLastSentCommand: (sessionId: string, command: string | null) => void;
  clearBlocks: (sessionId: string) => void;
  requestTerminalClear: (sessionId: string) => void;

  // Pipeline execution actions
  startPipelineExecution: (sessionId: string, execution: PipelineExecution) => void;
  updatePipelineStep: (
    sessionId: string,
    executionId: string,
    stepId: string,
    update: Partial<PipelineStepExecution>
  ) => void;
  completePipelineExecution: (
    sessionId: string,
    executionId: string,
    status: "completed" | "failed"
  ) => void;
  setPipelineCommandSource: (sessionId: string, isPipeline: boolean) => void;

  // Agent actions
  addAgentMessage: (sessionId: string, message: AgentMessage) => void;
  /** Append a text delta to the streaming buffer (efficient - no string concat) */
  updateAgentStreaming: (sessionId: string, delta: string) => void;
  /** Get the current streaming text (joins buffer if needed) */
  getAgentStreamingText: (sessionId: string) => string;
  clearAgentStreaming: (sessionId: string) => void;
  setAgentInitialized: (sessionId: string, initialized: boolean) => void;
  setAgentThinking: (sessionId: string, thinking: boolean) => void;
  setAgentResponding: (sessionId: string, responding: boolean) => void;
  setPendingToolApproval: (sessionId: string, tool: ToolCall | null) => void;
  setPendingAskHuman: (sessionId: string, request: AskHumanRequest) => void;
  clearPendingAskHuman: (sessionId: string) => void;
  markToolRequestProcessed: (sessionId: string, requestId: string) => void;
  isToolRequestProcessed: (sessionId: string, requestId: string) => boolean;
  updateToolCallStatus: (
    sessionId: string,
    toolId: string,
    status: ToolCall["status"],
    result?: unknown
  ) => void;
  clearAgentMessages: (sessionId: string) => void;
  restoreAgentMessages: (sessionId: string, messages: AgentMessage[]) => void;
  addActiveToolCall: (
    sessionId: string,
    toolCall: {
      id: string;
      name: string;
      args: Record<string, unknown>;
      executedByAgent?: boolean;
      source?: ToolCallSource;
    }
  ) => void;
  completeActiveToolCall: (
    sessionId: string,
    toolId: string,
    success: boolean,
    result?: unknown
  ) => void;
  clearActiveToolCalls: (sessionId: string) => void;
  // Streaming blocks actions
  addStreamingToolBlock: (
    sessionId: string,
    toolCall: {
      id: string;
      name: string;
      args: Record<string, unknown>;
      executedByAgent?: boolean;
      source?: ToolCallSource;
    }
  ) => void;
  updateStreamingToolBlock: (
    sessionId: string,
    toolId: string,
    success: boolean,
    result?: unknown
  ) => void;
  clearStreamingBlocks: (sessionId: string) => void;
  addUdiffResultBlock: (sessionId: string, response: string, durationMs: number) => void;
  addStreamingSystemHooksBlock: (sessionId: string, hooks: string[]) => void;
  /** Append streaming output chunk to a running tool call (for run_command) */
  appendToolStreamingOutput: (sessionId: string, toolId: string, chunk: string) => void;

  // Thinking content actions
  appendThinkingContent: (sessionId: string, content: string) => void;
  clearThinkingContent: (sessionId: string) => void;
  setThinkingExpanded: (sessionId: string, expanded: boolean) => void;

  // Timeline actions
  addSystemHookBlock: (sessionId: string, hooks: string[]) => void;
  clearTimeline: (sessionId: string) => void;

  // Workflow actions
  startWorkflow: (
    sessionId: string,
    workflow: { workflowId: string; workflowName: string; workflowSessionId: string }
  ) => void;
  workflowStepStarted: (
    sessionId: string,
    step: { stepName: string; stepIndex: number; totalSteps: number }
  ) => void;
  workflowStepCompleted: (
    sessionId: string,
    step: { stepName: string; output: string | null; durationMs: number }
  ) => void;
  completeWorkflow: (
    sessionId: string,
    result: { finalOutput: string; totalDurationMs: number }
  ) => void;
  failWorkflow: (sessionId: string, error: { stepName: string | null; error: string }) => void;
  clearActiveWorkflow: (sessionId: string) => void;
  /** Move workflow tool calls from activeToolCalls into the workflow for persistence */
  preserveWorkflowToolCalls: (sessionId: string) => void;

  // Sub-agent actions
  startPromptGeneration: (
    sessionId: string,
    agentId: string,
    parentRequestId: string,
    data: { architectSystemPrompt: string; architectUserMessage: string }
  ) => void;
  completePromptGeneration: (
    sessionId: string,
    agentId: string,
    parentRequestId: string,
    data: { generatedPrompt?: string; success: boolean; durationMs: number }
  ) => void;
  startSubAgent: (
    sessionId: string,
    agent: {
      agentId: string;
      agentName: string;
      parentRequestId: string;
      task: string;
      depth: number;
    }
  ) => void;
  addSubAgentToolCall: (
    sessionId: string,
    parentRequestId: string,
    toolCall: { id: string; name: string; args: Record<string, unknown> }
  ) => void;
  completeSubAgentToolCall: (
    sessionId: string,
    parentRequestId: string,
    toolId: string,
    success: boolean,
    result?: unknown
  ) => void;
  completeSubAgent: (
    sessionId: string,
    parentRequestId: string,
    result: { response: string; durationMs: number }
  ) => void;
  failSubAgent: (sessionId: string, parentRequestId: string, error: string) => void;
  updateSubAgentStreamingText: (sessionId: string, parentRequestId: string, text: string) => void;
  clearActiveSubAgents: (sessionId: string) => void;

  // AI tool execution timeline actions
  addToolExecutionBlock: (
    sessionId: string,
    execution: {
      requestId: string;
      toolName: string;
      args: Record<string, unknown>;
      autoApproved?: boolean;
      riskLevel?: string;
      source?: ToolCallSource;
    }
  ) => void;
  completeToolExecutionBlock: (
    sessionId: string,
    requestId: string,
    success: boolean,
    result?: unknown
  ) => void;
  appendToolExecutionOutput: (
    sessionId: string,
    requestId: string,
    chunk: string
  ) => void;

  // AI config actions
  setAiConfig: (config: Partial<AiConfig>) => void;
  // Per-session AI config actions
  setSessionAiConfig: (sessionId: string, config: Partial<AiConfig>) => void;
  getSessionAiConfig: (sessionId: string) => AiConfig | undefined;

  // Plan actions
  setPlan: (sessionId: string, plan: TaskPlan) => void;
  /** Bridge plan_updated events into a pipeline_progress timeline block */
  syncPlanToPipeline: (sessionId: string, plan: TaskPlan) => void;

  // Detail view actions
  setDetailViewMode: (sessionId: string, mode: DetailViewMode) => void;
  setToolDetailRequestIds: (sessionId: string, requestIds: string[] | null) => void;

  // Pane actions for multi-pane support
  splitPane: (
    tabId: string,
    paneId: PaneId,
    direction: SplitDirection,
    newPaneId: PaneId,
    newSessionId: string
  ) => void;
  closePane: (tabId: string, paneId: PaneId) => void;
  focusPane: (tabId: string, paneId: PaneId) => void;
  resizePane: (tabId: string, splitPaneId: PaneId, ratio: number) => void;
  navigatePane: (tabId: string, direction: "up" | "down" | "left" | "right") => void;
  /** Start move mode: user picks a drop zone on another pane */
  startPaneMove: (tabId: string, paneId: PaneId, sessionId: string) => void;
  /** Cancel move mode */
  cancelPaneMove: () => void;
  /** Complete a pane move: relocate source pane relative to target pane */
  completePaneMove: (targetPaneId: PaneId, direction: "top" | "right" | "bottom" | "left") => void;
  /** Extract a pane into its own new tab (preserving its session) */
  movePaneToNewTab: (tabId: string, paneId: PaneId) => void;
  /**
   * Open settings in a tab. If a settings tab already exists, focus it.
   * Otherwise, create a new settings tab.
   */
  openSettingsTab: () => void;
  /**
   * Open home in a tab. If a home tab already exists, focus it.
   * Otherwise, create a new home tab.
   */
  openHomeTab: () => void;
  /**
   * Open browser in a tab. If a browser tab already exists, focus it.
   * Otherwise, create a new browser tab.
   */
  openBrowserTab: (url?: string) => void;
  /**
   * Open security view in a tab. If a security tab already exists, focus it.
   * Otherwise, create a new security tab.
   */
  openSecurityTab: () => void;
  /**
   * Get all session IDs belonging to a tab (root + all pane sessions).
   * Used by TabBar to perform backend cleanup before removing state.
   */
  getTabSessionIds: (tabId: string) => string[];
  /**
   * Remove all state for a tab and its panes (frontend only).
   * Caller is responsible for backend cleanup (PTY/AI) before calling this.
   */
  closeTab: (tabId: string) => void;
  /**
   * Mark a tab as having new activity by resolving the owning tab from a session ID.
   */
  markTabNewActivityBySession: (sessionId: string) => void;
  /** Clear the new activity flag for a tab. */
  clearTabNewActivity: (tabId: string) => void;
  /** Move a tab left or right in the tab order. Home tab cannot be moved. */
  moveTab: (tabId: string, direction: "left" | "right") => void;
  /** Reorder a tab via drag-and-drop using tab IDs. Home tab is protected. */
  reorderTab: (draggedTabId: string, targetTabId: string) => void;
  /** Move a tab's content as a pane into another tab. */
  moveTabToPane: (
    sourceTabId: string,
    destTabId: string,
    location: "left" | "right" | "top" | "bottom"
  ) => void;
  /** Set the current project name and path (for workspace persistence). */
  setCurrentProject: (name: string | null, path?: string | null) => void;
}

/**
 * Sync a sub-agent's data to its timeline representation.
 * If the sub-agent is attached to a pipeline step, update it there;
 * otherwise update the standalone sub_agent_activity block.
 */
function syncSubAgentToTimeline(
  state: { subAgentPipelineMap: Record<string, { blockId: string; stepId: string }> },
  timeline: UnifiedBlock[],
  parentRequestId: string,
  agent: ActiveSubAgent,
): void {
  const mapping = state.subAgentPipelineMap[parentRequestId];
  if (mapping) {
    const pBlock = timeline.find((b) => b.id === mapping.blockId);
    if (pBlock && pBlock.type === "pipeline_progress") {
      const step = pBlock.data.steps.find((s) => s.stepId === mapping.stepId);
      if (step?.subAgents) {
        const idx = step.subAgents.findIndex((a) => a.parentRequestId === parentRequestId);
        if (idx >= 0) {
          step.subAgents[idx] = { ...agent };
        } else {
          step.subAgents.push({ ...agent });
        }
        return;
      }
    }
  }
  // Fallback: standalone block
  const block = timeline.find(
    (b) => b.type === "sub_agent_activity" && b.data.parentRequestId === parentRequestId,
  );
  if (block && block.type === "sub_agent_activity") {
    block.data = { ...agent };
  }
}

export const useStore = create<GolishState>()(
  devtools(
    immer((set, get, _store) => ({
      // Slices
      ...createAppearanceSlice(set, get),
      ...createContextSlice(set, get),
      ...createConversationSlice(set, get),
      ...createGitSlice(set, get),
      ...createNotificationSlice(set, get),
      ...createPanelSlice(set, get),

      // App focus/visibility state
      appIsFocused: true,
      appIsVisible: true,

      // Terminal restore loading
      terminalRestoreInProgress: false,
      setTerminalRestoreInProgress: (inProgress: boolean) =>
        set((state) => {
          state.terminalRestoreInProgress = inProgress;
        }),

      workspaceDataReady: false,
      setWorkspaceDataReady: (ready: boolean) =>
        set((state) => {
          state.workspaceDataReady = ready;
        }),

      // Core state
      sessions: {},
      activeSessionId: null,
      homeTabId: null,
      currentProjectName: null,
      currentProjectPath: null,
      aiConfig: {
        provider: "",
        model: "",
        status: "disconnected" as AiStatus,
      },
      timelines: {},
      pendingCommand: {},
      lastSentCommand: {},
      pipelineCommandSource: {},
      agentStreamingBuffer: {},
      agentStreaming: {},
      streamingBlocks: {},
      streamingTextOffset: {},
      streamingBlockRevision: {},
      agentInitialized: {},
      isAgentThinking: {},
      isAgentResponding: {},
      pendingToolApproval: {},
      pendingAskHuman: {},
      processedToolRequests: {},
      activeToolCalls: {},
      thinkingContent: {},
      isThinkingExpanded: {},
      activeWorkflows: {},
      tabLayouts: {},
      workflowHistory: {},
      activeSubAgents: {},
      subAgentBatchCounter: {},
      subAgentPipelineMap: {},
      terminalClearRequest: {},
      tabHasNewActivity: {},
      tabOrder: [],
      tabActivationHistory: [],
      paneMoveState: null,

      addSession: (session, options) =>
        set((state) => {
          const isPaneSession = options?.isPaneSession ?? false;

          state.sessions[session.id] = {
            ...session,
            tabType: session.tabType ?? "terminal",
            inputMode: session.inputMode ?? "terminal", // Default to terminal mode
          };

          // Only set as active and create tab layout for new tabs, not pane sessions
          if (!isPaneSession) {
            state.activeSessionId = session.id;
            // Track in activation history for MRU tab switching
            const histIdx = state.tabActivationHistory.indexOf(session.id);
            if (histIdx !== -1) {
              state.tabActivationHistory.splice(histIdx, 1);
            }
            state.tabActivationHistory.push(session.id);

            // Notify backend of active terminal session for visible command execution
            if ((session.tabType ?? "terminal") === "terminal") {
              import("@/lib/tauri").then(({ setActiveTerminalSession }) => {
                setActiveTerminalSession(session.id).catch(() => {});
              });
            }
          }

          state.timelines[session.id] = [];
          state.pendingCommand[session.id] = null;
          state.lastSentCommand[session.id] = null;
          state.agentStreamingBuffer[session.id] = [];
          state.agentStreaming[session.id] = "";
          state.streamingBlocks[session.id] = [];
          state.streamingTextOffset[session.id] = 0;
          state.agentInitialized[session.id] = false;
          state.isAgentThinking[session.id] = false;
          state.isAgentResponding[session.id] = false;
          state.pendingToolApproval[session.id] = null;
          state.pendingAskHuman[session.id] = null;
          state.activeToolCalls[session.id] = [];
          state.thinkingContent[session.id] = "";
          state.isThinkingExpanded[session.id] = true;
          state.activeWorkflows[session.id] = null;
          state.workflowHistory[session.id] = [];
          state.activeSubAgents[session.id] = [];
          // Initialize context metrics with default values
          state.contextMetrics[session.id] = {
            utilization: 0,
            usedTokens: 0,
            maxTokens: 0,
            isWarning: false,
          };
          // Initialize compaction state
          state.compactionCount[session.id] = 0;
          state.isCompacting[session.id] = false;
          state.isSessionDead[session.id] = false;
          state.compactionError[session.id] = null;
          state.gitStatus[session.id] = null;
          // Start with loading true so git badge shows loading spinner immediately
          state.gitStatusLoading[session.id] = true;
          state.gitCommitMessage[session.id] = "";

          // Only initialize pane layout for new tabs, not pane sessions
          // Pane sessions are added to an existing tab's layout via splitPane
          if (!isPaneSession) {
            state.tabLayouts[session.id] = {
              root: { type: "leaf", id: session.id, sessionId: session.id },
              focusedPaneId: session.id,
            };
            state.tabOrder.push(session.id);
          }
          if (!isPaneSession) {
            state.tabHasNewActivity[session.id] = false;
          }
        }),

      removeSession: (sessionId) => {
        // Dispose terminal instance (outside state update to avoid side effects in Immer)
        TerminalInstanceManager.dispose(sessionId);

        // Clean up AI event sequence tracking to prevent memory leak
        import("@/hooks/useAiEvents").then(({ resetSessionSequence }) => {
          resetSessionSequence(sessionId);
        });

        set((state) => {
          delete state.sessions[sessionId];
          delete state.timelines[sessionId];
          delete state.pendingCommand[sessionId];
          delete state.lastSentCommand[sessionId];
          delete state.agentStreamingBuffer[sessionId];
          delete state.agentStreaming[sessionId];
          delete state.streamingBlocks[sessionId];
          delete state.streamingTextOffset[sessionId];
          delete state.agentInitialized[sessionId];
          delete state.isAgentThinking[sessionId];
          delete state.isAgentResponding[sessionId];
          delete state.pendingToolApproval[sessionId];
          delete state.pendingAskHuman[sessionId];
          delete state.processedToolRequests[sessionId];
          delete state.activeToolCalls[sessionId];
          delete state.thinkingContent[sessionId];
          delete state.isThinkingExpanded[sessionId];
          delete state.gitStatus[sessionId];
          delete state.gitStatusLoading[sessionId];
          delete state.gitCommitMessage[sessionId];
          delete state.contextMetrics[sessionId];
          delete state.compactionCount[sessionId];
          delete state.isCompacting[sessionId];
          delete state.isSessionDead[sessionId];
          delete state.compactionError[sessionId];
          // Clean up tab layout if this is a tab's root session
          delete state.tabLayouts[sessionId];
          delete state.tabHasNewActivity[sessionId];
          const tabOrderIdx = state.tabOrder.indexOf(sessionId);
          if (tabOrderIdx !== -1) {
            state.tabOrder.splice(tabOrderIdx, 1);
          }

          if (state.activeSessionId === sessionId) {
            state.tabActivationHistory = state.tabActivationHistory.filter(
              (id) => id !== sessionId
            );
            state.activeSessionId =
              state.tabActivationHistory[state.tabActivationHistory.length - 1] ?? null;
          } else {
            state.tabActivationHistory = state.tabActivationHistory.filter(
              (id) => id !== sessionId
            );
          }
        });
      },

      setActiveSession: (sessionId) =>
        set((state) => {
          state.activeSessionId = sessionId;
          state.tabHasNewActivity[sessionId] = false;
          // Update MRU history: remove if already present, then push to end
          const idx = state.tabActivationHistory.indexOf(sessionId);
          if (idx !== -1) {
            state.tabActivationHistory.splice(idx, 1);
          }
          state.tabActivationHistory.push(sessionId);

          // 1:1 reverse sync: if this terminal belongs to a conversation, activate it
          for (const [convId, terminals] of Object.entries(state.conversationTerminals)) {
            if (terminals.includes(sessionId) && state.activeConversationId !== convId) {
              state.activeConversationId = convId;
              break;
            }
          }

          // Notify backend of active terminal session for visible command execution
          const session = state.sessions[sessionId];
          if (session && (session.tabType ?? "terminal") === "terminal") {
            import("@/lib/tauri").then(({ setActiveTerminalSession }) => {
              setActiveTerminalSession(sessionId).catch(() => {});
            });
          }
        }),

      setAppIsFocused: (focused) =>
        set((state) => {
          state.appIsFocused = focused;
        }),

      setAppIsVisible: (visible) =>
        set((state) => {
          state.appIsVisible = visible;
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

      setCustomTabName: (sessionId, customName) =>
        set((state) => {
          if (state.sessions[sessionId]) {
            state.sessions[sessionId].customName = customName ?? undefined;
          }
        }),

      setProcessName: (sessionId, processName) =>
        set((state) => {
          if (state.sessions[sessionId]) {
            // Only set process name if there's no custom name
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

      handlePromptStart: (sessionId) =>
        set((state) => {
          // Finalize any pending command without exit code
          const pending = state.pendingCommand[sessionId];
          if (pending?.command) {
            const session = state.sessions[sessionId];
            // Skip block creation for fullterm-mode commands (vim, claude, aider, etc.).
            // Fullterm output must never appear in the timeline regardless of how the
            // command lifecycle ended (missing command_end, null exit_code, etc.).
            if (session?.renderMode === "fullterm") {
              state.pendingCommand[sessionId] = null;
              return;
            }
            // Use the session's CURRENT working directory (at command end), not the one
            // captured at command start. This ensures that for commands like "cd foo && ls",
            // file paths in the output are resolved relative to the new directory.
            const currentWorkingDir = session?.workingDirectory || pending.workingDirectory;
            const blockId = crypto.randomUUID();
            const block: CommandBlock = {
              id: blockId,
              sessionId,
              command: pending.command,
              output: pending.output,
              exitCode: null,
              startTime: pending.startTime,
              durationMs: null,
              workingDirectory: currentWorkingDir,
              isCollapsed: false,
            };

            // Push to unified timeline (Single Source of Truth)
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
        }),

      handlePromptEnd: (_sessionId) => {
        // Ready for input - nothing to do for now
      },

      handleCommandStart: (sessionId, command) =>
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
        }),

      handleCommandEnd: (sessionId, exitCode) => {
        // Capture pending command info before state update for notification
        const currentState = get();
        const pending = currentState.pendingCommand[sessionId];
        const command = pending?.command;
        const session = currentState.sessions[sessionId];
        const isFullterm = session?.renderMode === "fullterm";
        const shouldNotify = pending && command && !isFullterm;

        set((state) => {
          const pending = state.pendingCommand[sessionId];
          if (pending) {
            // Skip creating command block for fullterm mode commands
            // Fullterm mode is for interactive apps (vim, ssh, etc.) that use
            // the full terminal - their output shouldn't appear in the timeline
            const session = state.sessions[sessionId];
            const isFullterm = session?.renderMode === "fullterm";

            // Only create a command block if:
            // 1. There was an actual command (not empty)
            // 2. NOT in fullterm mode (those sessions are handled by xterm directly)
            if (pending.command && !isFullterm) {
              const blockId = crypto.randomUUID();
              // Use the session's CURRENT working directory (at command end), not the one
              // captured at command start. This ensures that for commands like "cd foo && ls",
              // file paths in the output are resolved relative to the new directory.
              const currentWorkingDir = session?.workingDirectory || pending.workingDirectory;
              const block: CommandBlock = {
                id: blockId,
                sessionId,
                command: pending.command,
                output: pending.output,
                exitCode,
                startTime: pending.startTime,
                durationMs: Date.now() - new Date(pending.startTime).getTime(),
                workingDirectory: currentWorkingDir,
                isCollapsed: false,
              };

              // Push to unified timeline (Single Source of Truth)
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

        // Send native OS notification for command completion (outside Immer)
        if (shouldNotify) {
          const tabId = getOwningTabId(sessionId);
          if (tabId) {
            const exitStatus = exitCode === 0 ? "✓" : `✗ ${exitCode}`;
            sendNotification({
              title: "Command completed",
              body: `${exitStatus} ${command}`,
              tabId,
            }).catch((err) => {
              logger.debug("Failed to send command notification:", err);
            });
          }
        }

        if (pending && command && pending.output) {
          window.dispatchEvent(
            new CustomEvent("tool-output-completed", {
              detail: { command, output: pending.output, sessionId },
            })
          );
        }
      },

      appendOutput: (sessionId, data) =>
        set((state) => {
          let pending = state.pendingCommand[sessionId];
          // Auto-create pendingCommand if it doesn't exist (fallback for missing command_start)
          // This allows showing output even when OSC 133 shell integration isn't working
          if (!pending) {
            const session = state.sessions[sessionId];
            pending = {
              command: null, // Will show as "Running..." in the UI
              output: "",
              startTime: new Date().toISOString(),
              workingDirectory: session?.workingDirectory || "",
            };
            state.pendingCommand[sessionId] = pending;
          }
          pending.output += data;
        }),

      setPendingOutput: (sessionId, output) =>
        set((state) => {
          const pending = state.pendingCommand[sessionId];
          if (pending) {
            pending.output = output;
          }
        }),

      toggleBlockCollapse: (blockId) =>
        set((state) => {
          // Update in unified timeline
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

      clearBlocks: (sessionId) =>
        set((state) => {
          // Clear command blocks from timeline
          const timeline = state.timelines[sessionId];
          if (timeline) {
            state.timelines[sessionId] = timeline.filter((block) => block.type !== "command");
          }
          state.pendingCommand[sessionId] = null;
        }),

      requestTerminalClear: (sessionId) =>
        set((state) => {
          state.terminalClearRequest[sessionId] = (state.terminalClearRequest[sessionId] ?? 0) + 1;
        }),

      // Pipeline execution actions
      startPipelineExecution: (sessionId, execution) =>
        set((state) => {
          if (!state.timelines[sessionId]) state.timelines[sessionId] = [];
          state.timelines[sessionId].push({
            id: `pipeline-${execution.pipelineId}-${Date.now()}`,
            type: "pipeline_progress",
            timestamp: new Date().toISOString(),
            data: execution,
          });
        }),

      updatePipelineStep: (sessionId, executionId, stepId, update) =>
        set((state) => {
          const timeline = state.timelines[sessionId];
          if (!timeline) return;
          const block = timeline.find(
            (b) => b.type === "pipeline_progress" && b.id === executionId
          );
          if (!block || block.type !== "pipeline_progress") return;
          const step = block.data.steps.find((s) => s.stepId === stepId);
          if (step) Object.assign(step, update);
        }),

      completePipelineExecution: (sessionId, executionId, status) =>
        set((state) => {
          const timeline = state.timelines[sessionId];
          if (!timeline) return;
          const block = timeline.find(
            (b) => b.type === "pipeline_progress" && b.id === executionId
          );
          if (!block || block.type !== "pipeline_progress") return;
          block.data.status = status;
          block.data.finishedAt = new Date().toISOString();
        }),

      setPipelineCommandSource: (sessionId, isPipeline) =>
        set((state) => {
          state.pipelineCommandSource[sessionId] = isPipeline;
        }),

      // Agent actions
      addAgentMessage: (sessionId, message) =>
        set((state) => {
          // Snapshot the session's current working directory onto the message
          // so file links remain stable even if the user later navigates elsewhere.
          if (!message.workingDirectory) {
            const session = state.sessions[sessionId];
            if (session?.workingDirectory) {
              message.workingDirectory = session.workingDirectory;
            }
          }

          // Push to unified timeline (Single Source of Truth)
          if (!state.timelines[sessionId]) {
            state.timelines[sessionId] = [];
          }
          state.timelines[sessionId].push({
            id: message.id,
            type: "agent_message",
            timestamp: message.timestamp,
            data: message,
          });

          // Accumulate token usage for the session if available (input/output separately)
          if (message.inputTokens || message.outputTokens) {
            const current = state.sessionTokenUsage[sessionId] ?? { input: 0, output: 0 };
            state.sessionTokenUsage[sessionId] = {
              input: current.input + (message.inputTokens ?? 0),
              output: current.output + (message.outputTokens ?? 0),
            };
          }
        }),

      updateAgentStreaming: (sessionId, delta) =>
        set((state) => {
          // Push delta to buffer (O(1) amortized - avoids string reallocation)
          if (!state.agentStreamingBuffer[sessionId]) {
            state.agentStreamingBuffer[sessionId] = [];
          }
          state.agentStreamingBuffer[sessionId].push(delta);
          // Update cached joined text (for selectors that read agentStreaming directly)
          // Note: join() is called once per ~60fps throttle cycle, not per delta
          state.agentStreaming[sessionId] = state.agentStreamingBuffer[sessionId].join("");

          // Update streaming blocks - append delta to text block
          if (!state.streamingBlocks[sessionId]) {
            state.streamingBlocks[sessionId] = [];
          }
          const blocks = state.streamingBlocks[sessionId];

          // Just append or update the current text block with the new delta
          const lastBlock = blocks[blocks.length - 1];
          if (lastBlock && lastBlock.type === "text") {
            // Append delta to the text block (this concatenation happens at ~60fps, not per token)
            lastBlock.content += delta;
          } else if (delta) {
            // Add new text block (after a tool block or as first block)
            blocks.push({ type: "text", content: delta });
          }
        }),

      getAgentStreamingText: (sessionId) => {
        const state = get();
        const buffer = state.agentStreamingBuffer[sessionId];
        if (!buffer || buffer.length === 0) return "";
        // Return cached value from agentStreaming (updated on each delta)
        return state.agentStreaming[sessionId] ?? "";
      },

      clearAgentStreaming: (sessionId) =>
        set((state) => {
          state.agentStreamingBuffer[sessionId] = [];
          state.agentStreaming[sessionId] = "";
          state.streamingBlocks[sessionId] = [];
          state.streamingTextOffset[sessionId] = 0;
        }),

      setAgentInitialized: (sessionId, initialized) =>
        set((state) => {
          state.agentInitialized[sessionId] = initialized;
        }),

      setAgentThinking: (sessionId, thinking) =>
        set((state) => {
          state.isAgentThinking[sessionId] = thinking;
        }),

      setAgentResponding: (sessionId, responding) =>
        set((state) => {
          state.isAgentResponding[sessionId] = responding;
        }),

      setPendingToolApproval: (sessionId, tool) =>
        set((state) => {
          state.pendingToolApproval[sessionId] = tool;
        }),

      setPendingAskHuman: (sessionId, request) =>
        set((state) => {
          state.pendingAskHuman[sessionId] = request;
        }),

      clearPendingAskHuman: (sessionId) =>
        set((state) => {
          state.pendingAskHuman[sessionId] = null;
        }),

      markToolRequestProcessed: (sessionId, requestId) =>
        set((state) => {
          if (!state.processedToolRequests[sessionId]) {
            state.processedToolRequests[sessionId] = new Set<string>();
          }
          state.processedToolRequests[sessionId].add(requestId);
        }),

      isToolRequestProcessed: (sessionId, requestId) => {
        return get().processedToolRequests[sessionId]?.has(requestId) ?? false;
      },

      updateToolCallStatus: (sessionId, toolId, status, result) =>
        set((state) => {
          // Update tool call status in timeline (Single Source of Truth)
          const timeline = state.timelines[sessionId];
          if (timeline) {
            for (const block of timeline) {
              if (block.type === "agent_message") {
                const tool = block.data.toolCalls?.find((t) => t.id === toolId);
                if (tool) {
                  tool.status = status;
                  if (result !== undefined) tool.result = result;
                  return;
                }
              }
            }
          }
        }),

      clearAgentMessages: (sessionId) =>
        set((state) => {
          // Clear agent messages from timeline
          const timeline = state.timelines[sessionId];
          if (timeline) {
            state.timelines[sessionId] = timeline.filter((block) => block.type !== "agent_message");
          }
          state.agentStreamingBuffer[sessionId] = [];
          state.agentStreaming[sessionId] = "";
        }),

      restoreAgentMessages: (sessionId, messages) =>
        set((state) => {
          state.agentStreamingBuffer[sessionId] = [];
          state.agentStreaming[sessionId] = "";
          // Replace the timeline with restored messages (Single Source of Truth)
          state.timelines[sessionId] = [];
          for (const message of messages) {
            state.timelines[sessionId].push({
              id: message.id,
              type: "agent_message",
              timestamp: message.timestamp,
              data: message,
            });
          }
        }),

      addActiveToolCall: (sessionId, toolCall) =>
        set((state) => {
          if (!state.activeToolCalls[sessionId]) {
            state.activeToolCalls[sessionId] = [];
          }
          state.activeToolCalls[sessionId].push({
            ...toolCall,
            status: "running",
            startedAt: new Date().toISOString(),
          });
        }),

      completeActiveToolCall: (sessionId, toolId, success, result) =>
        set((state) => {
          const tools = state.activeToolCalls[sessionId];
          if (tools) {
            const tool = tools.find((t) => t.id === toolId);
            if (tool) {
              tool.status = success ? "completed" : "error";
              tool.result = result;
              tool.completedAt = new Date().toISOString();
            }
          }
        }),

      clearActiveToolCalls: (sessionId) =>
        set((state) => {
          state.activeToolCalls[sessionId] = [];
        }),

      // Streaming blocks actions
      addStreamingToolBlock: (sessionId, toolCall) =>
        set((state) => {
          if (!state.streamingBlocks[sessionId]) {
            state.streamingBlocks[sessionId] = [];
          }

          const blocks = state.streamingBlocks[sessionId];

          // Append the tool block (text is already added to last text block by updateAgentStreaming)
          blocks.push({
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
        set((state) => {
          // Update in activeToolCalls
          const tools = state.activeToolCalls[sessionId];
          if (tools) {
            const toolIndex = tools.findIndex((t) => t.id === toolId);
            if (toolIndex !== -1) {
              // Create new object to ensure reference change for React re-render
              tools[toolIndex] = {
                ...tools[toolIndex],
                streamingOutput: (tools[toolIndex].streamingOutput ?? "") + chunk,
              };
            }
          }
          // Also update in streamingBlocks
          const blocks = state.streamingBlocks[sessionId];
          if (blocks) {
            for (let i = 0; i < blocks.length; i++) {
              const block = blocks[i];
              if (block.type === "tool" && block.toolCall.id === toolId) {
                // Create new block and toolCall objects to ensure reference change
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

      // Thinking content actions
      appendThinkingContent: (sessionId, content) =>
        set((state) => {
          // Legacy support (keep accumulating full content)
          if (!state.thinkingContent[sessionId]) {
            state.thinkingContent[sessionId] = "";
          }
          state.thinkingContent[sessionId] += content;

          // New interleaved support
          if (!state.streamingBlocks[sessionId]) {
            state.streamingBlocks[sessionId] = [];
          }
          const blocks = state.streamingBlocks[sessionId];
          const lastBlock = blocks[blocks.length - 1];

          if (lastBlock && lastBlock.type === "thinking") {
            lastBlock.content += content;
          } else {
            // Remove any previous thinking blocks so only one is visible at a time
            state.streamingBlocks[sessionId] = blocks.filter((b) => b.type !== "thinking");
            // Start new thinking block
            state.streamingBlocks[sessionId].push({ type: "thinking", content });
          }
        }),

      clearThinkingContent: (sessionId) =>
        set((state) => {
          state.thinkingContent[sessionId] = "";
        }),

      setThinkingExpanded: (sessionId, expanded) =>
        set((state) => {
          state.isThinkingExpanded[sessionId] = expanded;
        }),

      // Timeline actions
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

      clearTimeline: (sessionId) =>
        set((state) => {
          state.timelines[sessionId] = [];
          state.pendingCommand[sessionId] = null;
          state.agentStreamingBuffer[sessionId] = [];
          state.agentStreaming[sessionId] = "";
          state.streamingBlocks[sessionId] = [];
        }),

      // Workflow actions
      startWorkflow: (sessionId, workflow) =>
        set((state) => {
          state.activeWorkflows[sessionId] = {
            workflowId: workflow.workflowId,
            workflowName: workflow.workflowName,
            sessionId: workflow.workflowSessionId,
            status: "running",
            steps: [],
            currentStepIndex: -1,
            totalSteps: 0,
            startedAt: new Date().toISOString(),
          };
        }),

      workflowStepStarted: (sessionId, step) =>
        set((state) => {
          const workflow = state.activeWorkflows[sessionId];
          if (!workflow) return;

          workflow.currentStepIndex = step.stepIndex;
          workflow.totalSteps = step.totalSteps;

          // Initialize step if not already present
          if (!workflow.steps[step.stepIndex]) {
            workflow.steps[step.stepIndex] = {
              name: step.stepName,
              index: step.stepIndex,
              status: "running",
              startedAt: new Date().toISOString(),
            };
          } else {
            workflow.steps[step.stepIndex].status = "running";
            workflow.steps[step.stepIndex].startedAt = new Date().toISOString();
          }
        }),

      workflowStepCompleted: (sessionId, step) =>
        set((state) => {
          const workflow = state.activeWorkflows[sessionId];
          if (!workflow) return;

          // Find the step by name (since index might not be exact)
          const stepData = workflow.steps.find((s) => s.name === step.stepName);
          if (stepData) {
            stepData.status = "completed";
            stepData.output = step.output;
            stepData.durationMs = step.durationMs;
            stepData.completedAt = new Date().toISOString();
          }
        }),

      completeWorkflow: (sessionId, result) =>
        set((state) => {
          const workflow = state.activeWorkflows[sessionId];
          if (!workflow) return;

          workflow.status = "completed";
          workflow.finalOutput = result.finalOutput;
          workflow.totalDurationMs = result.totalDurationMs;
          workflow.completedAt = new Date().toISOString();

          // Move to history (but keep visible in activeWorkflows for current message)
          if (!state.workflowHistory[sessionId]) {
            state.workflowHistory[sessionId] = [];
          }
          state.workflowHistory[sessionId].push({ ...workflow });
          // Note: We intentionally don't clear activeWorkflows here
          // The workflow tree stays visible until the AI response is finalized
        }),

      failWorkflow: (sessionId, error) =>
        set((state) => {
          const workflow = state.activeWorkflows[sessionId];
          if (!workflow) return;

          workflow.status = "error";
          workflow.error = error.error;
          workflow.completedAt = new Date().toISOString();

          // Mark current step as error if specified
          if (error.stepName) {
            const stepData = workflow.steps.find((s) => s.name === error.stepName);
            if (stepData) {
              stepData.status = "error";
            }
          }

          // Move to history (but keep visible in activeWorkflows for current message)
          if (!state.workflowHistory[sessionId]) {
            state.workflowHistory[sessionId] = [];
          }
          state.workflowHistory[sessionId].push({ ...workflow });
          // Note: We intentionally don't clear activeWorkflows here
          // The workflow tree stays visible until the AI response is finalized
        }),

      clearActiveWorkflow: (sessionId) =>
        set((state) => {
          state.activeWorkflows[sessionId] = null;
        }),

      preserveWorkflowToolCalls: (sessionId) =>
        set((state) => {
          const workflow = state.activeWorkflows[sessionId];
          const toolCalls = state.activeToolCalls[sessionId];

          if (!workflow || !toolCalls) return;

          // Filter tool calls that belong to this workflow
          const workflowToolCalls = toolCalls.filter((tool) => {
            const source = tool.source;
            return source?.type === "workflow" && source.workflowId === workflow.workflowId;
          });

          // Store them in the workflow
          workflow.toolCalls = workflowToolCalls;
        }),

      // Sub-agent actions
      startPromptGeneration: (sessionId, agentId, parentRequestId, data) =>
        set((state) => {
          if (!state.activeSubAgents[sessionId]) {
            state.activeSubAgents[sessionId] = [];
          }
          const now = new Date().toISOString();
          const agent = state.activeSubAgents[sessionId].find(
            (a) => a.parentRequestId === parentRequestId
          );
          if (agent) {
            agent.promptGeneration = {
              status: "generating",
              architectSystemPrompt: data.architectSystemPrompt,
              architectUserMessage: data.architectUserMessage,
            };
          } else {
            const newAgent: ActiveSubAgent = {
              agentId: agentId,
              agentName: "",
              parentRequestId: parentRequestId,
              task: "",
              depth: 0,
              status: "running",
              toolCalls: [],
              entries: [],
              startedAt: now,
              promptGeneration: {
                status: "generating",
                architectSystemPrompt: data.architectSystemPrompt,
                architectUserMessage: data.architectUserMessage,
              },
            };
            state.activeSubAgents[sessionId].push(newAgent);
          }
          // Create or sync timeline block
          if (!state.timelines[sessionId]) state.timelines[sessionId] = [];
          const timeline = state.timelines[sessionId];
          const blockId = `sub-agent-${parentRequestId}`;
          const agentData = state.activeSubAgents[sessionId].find(
            (a) => a.parentRequestId === parentRequestId
          );
          if (!agentData) return;
          const existingBlock = timeline.find((b) => b.id === blockId);
          if (existingBlock && existingBlock.type === "sub_agent_activity") {
            existingBlock.data = { ...agentData };
          } else if (!existingBlock) {
            timeline.push({
              id: blockId,
              type: "sub_agent_activity" as const,
              timestamp: now,
              data: { ...agentData },
            });
          }
        }),

      completePromptGeneration: (sessionId, _agentId, parentRequestId, data) =>
        set((state) => {
          const agents = state.activeSubAgents[sessionId];
          if (!agents) return;
          const agent = agents.find((a) => a.parentRequestId === parentRequestId);
          if (agent?.promptGeneration) {
            agent.promptGeneration.status = data.success ? "completed" : "failed";
            agent.promptGeneration.generatedPrompt = data.generatedPrompt;
            agent.promptGeneration.durationMs = data.durationMs;
          }
          // Sync to timeline
          const timeline = state.timelines[sessionId];
          if (timeline && agent) {
            const block = timeline.find(
              (b) => b.type === "sub_agent_activity" && b.data.parentRequestId === parentRequestId
            );
            if (block && block.type === "sub_agent_activity") {
              block.data = { ...agent };
            }
          }
        }),

      startSubAgent: (sessionId, agent) =>
        set((state) => {
          if (!state.activeSubAgents[sessionId]) {
            state.activeSubAgents[sessionId] = [];
          }
          const now = new Date().toISOString();
          const existing = agent.parentRequestId
            ? state.activeSubAgents[sessionId].find(
                (a) => a.parentRequestId === agent.parentRequestId
              )
            : undefined;
          if (existing) {
            existing.agentId = agent.agentId;
            existing.agentName = agent.agentName;
            existing.task = agent.task;
            existing.depth = agent.depth;
          } else {
            const newAgent: ActiveSubAgent = {
              agentId: agent.agentId,
              agentName: agent.agentName,
              parentRequestId: agent.parentRequestId,
              task: agent.task,
              depth: agent.depth,
              status: "running",
              toolCalls: [],
              entries: [],
              startedAt: now,
            };
            state.activeSubAgents[sessionId].push(newAgent);
          }

          if (!state.timelines[sessionId]) state.timelines[sessionId] = [];
          const timeline = state.timelines[sessionId];
          const agentData = state.activeSubAgents[sessionId].find(
            (a) => a.parentRequestId === agent.parentRequestId
          );
          if (!agentData) return;

          // Check if there's an active pipeline with a running AI step
          const AI_PREFIXES = ["AI:", "ai:"];
          let attachedToPipeline = false;
          for (const block of timeline) {
            if (block.type !== "pipeline_progress" || block.data.status !== "running") continue;
            const aiStep = block.data.steps.find(
              (s) =>
                s.status === "running" &&
                (AI_PREFIXES.some((p) => s.command.startsWith(p)) || s.name.includes("(AI)"))
            );
            if (aiStep) {
              if (!aiStep.subAgents) aiStep.subAgents = [];
              const existingIdx = aiStep.subAgents.findIndex(
                (a) => a.parentRequestId === agent.parentRequestId
              );
              if (existingIdx >= 0) {
                aiStep.subAgents[existingIdx] = { ...agentData };
              } else {
                aiStep.subAgents.push({ ...agentData });
              }
              state.subAgentPipelineMap[agent.parentRequestId] = {
                blockId: block.id,
                stepId: aiStep.stepId,
              };
              attachedToPipeline = true;
              break;
            }
          }

          if (attachedToPipeline) return;

          // Fallback: create standalone sub_agent_activity block
          const blockId = `sub-agent-${agent.parentRequestId}`;
          const existingBlock = timeline.find((b) => b.id === blockId);
          if (existingBlock && existingBlock.type === "sub_agent_activity") {
            existingBlock.data = { ...agentData };
          } else if (!existingBlock) {
            const currentAgents = state.activeSubAgents[sessionId];
            const anyRunning = currentAgents.some(
              (a) => a.status === "running" && a.parentRequestId !== agent.parentRequestId
            );
            if (!anyRunning) {
              state.subAgentBatchCounter[sessionId] = (state.subAgentBatchCounter[sessionId] ?? 0) + 1;
            }
            const batchId = `batch-${state.subAgentBatchCounter[sessionId] ?? 1}`;
            timeline.push({
              id: blockId,
              type: "sub_agent_activity" as const,
              timestamp: now,
              data: { ...agentData },
              batchId,
            });
          }
        }),

      addSubAgentToolCall: (sessionId, parentRequestId, toolCall) =>
        set((state) => {
          const agents = state.activeSubAgents[sessionId];
          if (!agents) return;

          const agent = agents.find((a) => a.parentRequestId === parentRequestId);
          if (agent) {
            agent.toolCalls.push({
              ...toolCall,
              status: "running",
              startedAt: new Date().toISOString(),
            });
            agent.entries.push({ kind: "tool_call", toolCallId: toolCall.id });
          }
          const timeline = state.timelines[sessionId];
          if (timeline && agent) {
            syncSubAgentToTimeline(state, timeline, parentRequestId, agent);
          }
        }),

      completeSubAgentToolCall: (sessionId, parentRequestId, toolId, success, result) =>
        set((state) => {
          const agents = state.activeSubAgents[sessionId];
          if (!agents) return;

          const agent = agents.find((a) => a.parentRequestId === parentRequestId);
          if (agent) {
            const tool = agent.toolCalls.find((t) => t.id === toolId);
            if (tool) {
              tool.status = success ? "completed" : "error";
              tool.result = result;
              tool.completedAt = new Date().toISOString();
            }
          }
          const timeline = state.timelines[sessionId];
          if (timeline && agent) {
            syncSubAgentToTimeline(state, timeline, parentRequestId, agent);
          }
        }),

      completeSubAgent: (sessionId, parentRequestId, result) =>
        set((state) => {
          const agents = state.activeSubAgents[sessionId];
          if (!agents) return;

          const agent = agents.find((a) => a.parentRequestId === parentRequestId);
          if (agent) {
            agent.status = "completed";
            agent.response = result.response;
            agent.durationMs = result.durationMs;
            agent.completedAt = new Date().toISOString();
          }
          const timeline = state.timelines[sessionId];
          if (timeline && agent) {
            syncSubAgentToTimeline(state, timeline, parentRequestId, agent);
          }
        }),

      failSubAgent: (sessionId, parentRequestId, error) =>
        set((state) => {
          const agents = state.activeSubAgents[sessionId];
          if (!agents) return;

          const agent = agents.find((a) => a.parentRequestId === parentRequestId);
          if (agent) {
            agent.status = "error";
            agent.error = error;
            agent.completedAt = new Date().toISOString();
          }
          const timeline = state.timelines[sessionId];
          if (timeline && agent) {
            syncSubAgentToTimeline(state, timeline, parentRequestId, agent);
          }
        }),

      updateSubAgentStreamingText: (sessionId, parentRequestId, text) =>
        set((state) => {
          const agents = state.activeSubAgents[sessionId];
          if (!agents) return;
          const agent = agents.find((a) => a.parentRequestId === parentRequestId);
          if (agent) {
            agent.streamingText = text;
            const lastEntry = agent.entries[agent.entries.length - 1];
            if (lastEntry && lastEntry.kind === "text") {
              lastEntry.text = text;
            } else {
              agent.entries.push({ kind: "text", text });
            }
          }
          const timeline = state.timelines[sessionId];
          if (timeline && agent) {
            syncSubAgentToTimeline(state, timeline, parentRequestId, agent);
          }
        }),

      clearActiveSubAgents: (sessionId) =>
        set((state) => {
          state.activeSubAgents[sessionId] = [];
        }),

      // AI tool execution timeline actions
      addToolExecutionBlock: (sessionId, execution) =>
        set((state) => {
          if (!state.timelines[sessionId]) {
            state.timelines[sessionId] = [];
          }
          // Tag with current in-progress plan step (if any)
          let planStepIndex: number | undefined;
          const plan = state.sessions[sessionId]?.plan;
          if (plan) {
            const idx = plan.steps.findIndex((s) => s.status === "in_progress");
            if (idx >= 0) planStepIndex = idx;
          }
          state.timelines[sessionId].push({
            id: `tool-exec-${execution.requestId}`,
            type: "ai_tool_execution",
            timestamp: new Date().toISOString(),
            data: {
              requestId: execution.requestId,
              toolName: execution.toolName,
              args: execution.args,
              status: "running",
              startedAt: new Date().toISOString(),
              autoApproved: execution.autoApproved,
              riskLevel: execution.riskLevel,
              source: execution.source,
              planStepIndex,
            },
          });
        }),

      completeToolExecutionBlock: (sessionId, requestId, success, result) =>
        set((state) => {
          const timeline = state.timelines[sessionId];
          if (!timeline) return;
          const block = timeline.find(
            (b) => b.type === "ai_tool_execution" && b.data.requestId === requestId
          );
          if (block && block.type === "ai_tool_execution") {
            block.data.status = success ? "completed" : "error";
            block.data.result = result;
            block.data.completedAt = new Date().toISOString();
            const start = new Date(block.data.startedAt).getTime();
            block.data.durationMs = Date.now() - start;
          }
        }),

      appendToolExecutionOutput: (sessionId, requestId, chunk) =>
        set((state) => {
          const timeline = state.timelines[sessionId];
          if (!timeline) return;
          const block = timeline.find(
            (b) => b.type === "ai_tool_execution" && b.data.requestId === requestId
          );
          if (block && block.type === "ai_tool_execution") {
            block.data.streamingOutput = (block.data.streamingOutput || "") + chunk;
          }
        }),

      // AI config actions
      setAiConfig: (config) =>
        set((state) => {
          state.aiConfig = { ...state.aiConfig, ...config };
        }),

      // Per-session AI config actions
      setSessionAiConfig: (sessionId, config) =>
        set((state) => {
          if (state.sessions[sessionId]) {
            const currentConfig = state.sessions[sessionId].aiConfig || {
              provider: "",
              model: "",
              status: "disconnected" as AiStatus,
            };
            state.sessions[sessionId].aiConfig = { ...currentConfig, ...config };
          }
        }),

      getSessionAiConfig: (sessionId) => {
        const session = get().sessions[sessionId];
        return session?.aiConfig;
      },

      // Plan actions
      setPlan: (sessionId, plan) =>
        set((state) => {
          if (state.sessions[sessionId]) {
            state.sessions[sessionId].plan = plan;
          }
        }),

      syncPlanToPipeline: (sessionId, plan) =>
        set((state) => {
          if (!state.timelines[sessionId]) state.timelines[sessionId] = [];
          const timeline = state.timelines[sessionId];

          const STATUS_MAP: Record<StepStatus, PipelineStepStatus> = {
            pending: "pending",
            in_progress: "running",
            completed: "success",
          };

          const blockId = `plan-pipeline-${sessionId}`;
          const now = new Date().toISOString();

          const steps: PipelineStepExecution[] = plan.steps.map((s, i) => ({
            stepId: `plan-step-${i}`,
            name: s.step,
            command: "",
            status: STATUS_MAP[s.status] ?? "pending",
            startedAt: s.status !== "pending" ? now : undefined,
          }));

          const anyRunning = plan.summary.in_progress > 0;
          const allDone = plan.summary.total > 0 && plan.summary.completed === plan.summary.total;
          const pipelineStatus = allDone ? "completed" : anyRunning ? "running" : "pending";

          const existing = timeline.find((b) => b.id === blockId);
          if (existing && existing.type === "pipeline_progress") {
            // Preserve per-step sub-agents from previous syncs
            for (const newStep of steps) {
              const prev = existing.data.steps.find((s) => s.stepId === newStep.stepId);
              if (prev?.subAgents) newStep.subAgents = prev.subAgents;
            }
            existing.data.steps = steps;
            existing.data.status = pipelineStatus;
            if (allDone) existing.data.finishedAt = now;
          } else if (!existing) {
            const execution: PipelineExecution = {
              pipelineId: `plan-v${plan.version}`,
              pipelineName: plan.explanation ?? "Task Plan",
              target: "",
              steps,
              status: pipelineStatus,
              startedAt: now,
            };
            timeline.push({
              id: blockId,
              type: "pipeline_progress",
              timestamp: now,
              data: execution,
            });
          }
        }),

      // Detail view actions
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

      // Pane actions for multi-pane support
      splitPane: (tabId, paneId, direction, newPaneId, newSessionId) =>
        set((state) => {
          const layout = state.tabLayouts[tabId];
          if (!layout) return;

          // Prevent splitting for non-terminal tabs (e.g., settings)
          const rootSession = state.sessions[tabId];
          const tabType = rootSession?.tabType ?? "terminal";
          if (tabType !== "terminal") {
            logger.warn(`[store] splitPane: Cannot split ${tabType} tabs`);
            return;
          }

          // Check pane limit (max 4 panes per tab)
          const currentCount = countLeafPanes(layout.root);
          if (currentCount >= 4) {
            logger.warn("[store] splitPane: Maximum pane limit (4) reached");
            return;
          }

          // Split the pane
          state.tabLayouts[tabId].root = splitPaneNode(
            layout.root,
            paneId,
            direction,
            newPaneId,
            newSessionId
          );

          // Focus the new pane (but keep activeSessionId pointing to the tab's root)
          // Note: activeSessionId identifies the TAB, not the focused pane within it.
          // The focused pane's session can be retrieved via useFocusedSessionId().
          state.tabLayouts[tabId].focusedPaneId = newPaneId;
        }),

      closePane: (tabId, paneId) => {
        // Get the session ID before state update (to dispose terminal outside Immer)
        const currentState = get();
        const layout = currentState.tabLayouts[tabId];
        if (!layout) return;

        const paneNode = findPaneById(layout.root, paneId);
        if (!paneNode || paneNode.type !== "leaf") return;

        const sessionIdToRemove = paneNode.sessionId;

        // Dispose terminal instance (outside state update to avoid side effects in Immer)
        TerminalInstanceManager.dispose(sessionIdToRemove);

        // Clean up AI event sequence tracking to prevent memory leak
        import("@/hooks/useAiEvents").then(({ resetSessionSequence }) => {
          resetSessionSequence(sessionIdToRemove);
        });

        set((state) => {
          const layout = state.tabLayouts[tabId];
          if (!layout) return;

          // Remove pane from tree
          const newRoot = removePaneNode(layout.root, paneId);

          if (newRoot === null) {
            // Last pane in tab - remove the entire tab
            // Note: Session cleanup should be handled by the caller
            delete state.tabLayouts[tabId];
            return;
          }

          // Update tree
          state.tabLayouts[tabId].root = newRoot;

          // Update focus to sibling or first available pane
          // Note: activeSessionId stays as the tab's root session ID
          if (layout.focusedPaneId === paneId) {
            const newFocusId = getFirstLeafPane(newRoot);
            state.tabLayouts[tabId].focusedPaneId = newFocusId;
          }

          // Clean up the closed session's state
          delete state.sessions[sessionIdToRemove];
          delete state.timelines[sessionIdToRemove];
          delete state.pendingCommand[sessionIdToRemove];
          delete state.lastSentCommand[sessionIdToRemove];
          delete state.agentStreamingBuffer[sessionIdToRemove];
          delete state.agentStreaming[sessionIdToRemove];
          delete state.streamingBlocks[sessionIdToRemove];
          delete state.streamingTextOffset[sessionIdToRemove];
          delete state.agentInitialized[sessionIdToRemove];
          delete state.isAgentThinking[sessionIdToRemove];
          delete state.isAgentResponding[sessionIdToRemove];
          delete state.pendingToolApproval[sessionIdToRemove];
          delete state.pendingAskHuman[sessionIdToRemove];
          delete state.processedToolRequests[sessionIdToRemove];
          delete state.activeToolCalls[sessionIdToRemove];
          delete state.thinkingContent[sessionIdToRemove];
          delete state.isThinkingExpanded[sessionIdToRemove];
          delete state.gitStatus[sessionIdToRemove];
          delete state.gitStatusLoading[sessionIdToRemove];
          delete state.tabHasNewActivity[sessionIdToRemove];
          delete state.gitCommitMessage[sessionIdToRemove];
          delete state.contextMetrics[sessionIdToRemove];
        });
      },

      focusPane: (tabId, paneId) =>
        set((state) => {
          const layout = state.tabLayouts[tabId];
          if (!layout) return;

          const paneNode = findPaneById(layout.root, paneId);
          if (!paneNode || paneNode.type !== "leaf") return;

          // Only update focusedPaneId - activeSessionId stays as the tab's root session ID
          // The focused pane's session can be retrieved via useFocusedSessionId()
          state.tabLayouts[tabId].focusedPaneId = paneId;
        }),

      resizePane: (tabId, splitPaneId, ratio) =>
        set((state) => {
          const layout = state.tabLayouts[tabId];
          if (!layout) return;

          state.tabLayouts[tabId].root = updatePaneRatio(layout.root, splitPaneId, ratio);
        }),

      navigatePane: (tabId, direction) =>
        set((state) => {
          const layout = state.tabLayouts[tabId];
          if (!layout) return;

          const neighborId = getPaneNeighbor(layout.root, layout.focusedPaneId, direction);
          if (!neighborId) return;

          // Only update focusedPaneId - activeSessionId stays as the tab's root session ID
          state.tabLayouts[tabId].focusedPaneId = neighborId;
        }),

      startPaneMove: (tabId, paneId, sessionId) =>
        set((state) => {
          state.paneMoveState = { tabId, sourcePaneId: paneId, sourceSessionId: sessionId };
        }),

      cancelPaneMove: () =>
        set((state) => {
          state.paneMoveState = null;
        }),

      completePaneMove: (targetPaneId, direction) =>
        set((state) => {
          const moveState = state.paneMoveState;
          if (!moveState) return;

          const { tabId, sourcePaneId, sourceSessionId } = moveState;
          const layout = state.tabLayouts[tabId];
          if (!layout) return;

          // Can't move to self
          if (sourcePaneId === targetPaneId) {
            state.paneMoveState = null;
            return;
          }

          // Remove source pane from tree
          const treeAfterRemove = removePaneNode(layout.root, sourcePaneId);
          if (!treeAfterRemove) {
            state.paneMoveState = null;
            return;
          }

          // Insert at target position
          const sourceLeaf = {
            type: "leaf" as const,
            id: sourcePaneId,
            sessionId: sourceSessionId,
          };
          const newRoot = insertPaneAtPosition(
            treeAfterRemove,
            targetPaneId,
            direction,
            sourceLeaf
          );

          state.tabLayouts[tabId].root = newRoot;
          state.tabLayouts[tabId].focusedPaneId = sourcePaneId;
          state.paneMoveState = null;
        }),

      movePaneToNewTab: (tabId, paneId) =>
        set((state) => {
          const layout = state.tabLayouts[tabId];
          if (!layout) return;

          // Must have more than one pane
          if (countLeafPanes(layout.root) <= 1) return;

          const paneNode = findPaneById(layout.root, paneId);
          if (!paneNode || paneNode.type !== "leaf") return;

          const sessionId = paneNode.sessionId;

          // Remove pane from source tab's tree
          const newRoot = removePaneNode(layout.root, paneId);
          if (!newRoot) return;

          state.tabLayouts[tabId].root = newRoot;

          // Update focus if the removed pane was focused
          if (layout.focusedPaneId === paneId) {
            state.tabLayouts[tabId].focusedPaneId = getFirstLeafPane(newRoot);
          }

          // Create a new tab layout for the extracted session
          // The session already exists in state.sessions, so we just need a layout
          state.tabLayouts[sessionId] = {
            root: { type: "leaf", id: sessionId, sessionId },
            focusedPaneId: sessionId,
          };
          state.tabHasNewActivity[sessionId] = false;
          state.tabOrder.push(sessionId);
          state.activeSessionId = sessionId;
          // Track in activation history for MRU tab switching
          const histIdx = state.tabActivationHistory.indexOf(sessionId);
          if (histIdx !== -1) {
            state.tabActivationHistory.splice(histIdx, 1);
          }
          state.tabActivationHistory.push(sessionId);
        }),

      openSettingsTab: () =>
        set((state) => {
          // Check if a settings tab already exists
          const existingSettingsTab = Object.values(state.sessions).find(
            (session) => session.tabType === "settings"
          );

          if (existingSettingsTab) {
            // Focus the existing settings tab
            state.activeSessionId = existingSettingsTab.id;
            state.tabHasNewActivity[existingSettingsTab.id] = false;
            // Track in activation history for MRU tab switching
            const histIdx = state.tabActivationHistory.indexOf(existingSettingsTab.id);
            if (histIdx !== -1) {
              state.tabActivationHistory.splice(histIdx, 1);
            }
            state.tabActivationHistory.push(existingSettingsTab.id);
            return;
          }

          // Create a new settings tab with a unique ID
          const settingsId = `settings-${Date.now()}`;

          // Create minimal session for settings tab
          state.sessions[settingsId] = {
            id: settingsId,
            tabType: "settings",
            name: "Settings",
            workingDirectory: "",
            createdAt: new Date().toISOString(),
            mode: "terminal", // Not used for settings, but required by Session interface
          };

          // Set as active tab
          state.activeSessionId = settingsId;

          // Create tab layout (single pane, no splitting for settings)
          state.tabLayouts[settingsId] = {
            root: { type: "leaf", id: settingsId, sessionId: settingsId },
            focusedPaneId: settingsId,
          };
          state.tabHasNewActivity[settingsId] = false;
          state.tabOrder.push(settingsId);
          // Track in activation history for MRU tab switching
          state.tabActivationHistory.push(settingsId);
        }),

      openHomeTab: () =>
        set((state) => {
          // Check if a home tab already exists
          const existingHomeTab = Object.values(state.sessions).find(
            (session) => session.tabType === "home"
          );

          if (existingHomeTab) {
            // Focus the existing home tab
            state.activeSessionId = existingHomeTab.id;
            state.tabHasNewActivity[existingHomeTab.id] = false;
            // Track in activation history for MRU tab switching
            const histIdx = state.tabActivationHistory.indexOf(existingHomeTab.id);
            if (histIdx !== -1) {
              state.tabActivationHistory.splice(histIdx, 1);
            }
            state.tabActivationHistory.push(existingHomeTab.id);
            return;
          }

          // Create a new home tab with a unique ID
          const homeId = `home-${Date.now()}`;

          // Create minimal session for home tab
          state.sessions[homeId] = {
            id: homeId,
            tabType: "home",
            name: "Home",
            workingDirectory: "",
            createdAt: new Date().toISOString(),
            mode: "terminal", // Not used for home, but required by Session interface
          };

          // Set as active tab
          state.activeSessionId = homeId;
          state.homeTabId = homeId;

          // Create tab layout (single pane, no splitting for home)
          state.tabLayouts[homeId] = {
            root: { type: "leaf", id: homeId, sessionId: homeId },
            focusedPaneId: homeId,
          };
          state.tabHasNewActivity[homeId] = false;
          // Home tab is always at index 0
          state.tabOrder.unshift(homeId);
          // Track in activation history for MRU tab switching
          state.tabActivationHistory.push(homeId);
        }),

      openBrowserTab: (url?: string) =>
        set((state) => {
          const existingBrowserTab = Object.values(state.sessions).find(
            (session) => session.tabType === "browser"
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

          state.tabLayouts[browserId] = {
            root: { type: "leaf", id: browserId, sessionId: browserId },
            focusedPaneId: browserId,
          };
          state.tabHasNewActivity[browserId] = false;
          state.tabOrder.push(browserId);
          state.tabActivationHistory.push(browserId);
        }),

      openSecurityTab: () =>
        set((state) => {
          const existingTab = Object.values(state.sessions).find(
            (session) => session.tabType === "security"
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

          state.tabLayouts[securityId] = {
            root: { type: "leaf", id: securityId, sessionId: securityId },
            focusedPaneId: securityId,
          };
          state.tabHasNewActivity[securityId] = false;
          state.tabOrder.push(securityId);
          state.tabActivationHistory.push(securityId);
        }),

      getTabSessionIds: (tabId) => {
        const layout = get().tabLayouts[tabId];
        if (!layout) return [];
        return getAllLeafPanes(layout.root).map((pane) => pane.sessionId);
      },

      closeTab: (tabId) => {
        // Get session IDs before state update (for cleanup outside Immer)
        const currentState = get();
        const layout = currentState.tabLayouts[tabId];
        const sessionIdsToClean: string[] = [];

        if (!layout) {
          // No layout - backward compatibility
          sessionIdsToClean.push(tabId);
        } else {
          // Get all pane sessions in this tab
          const panes = getAllLeafPanes(layout.root);
          for (const pane of panes) {
            sessionIdsToClean.push(pane.sessionId);
          }
        }

        // Clean up outside Immer (terminal instances and AI event sequence tracking)
        for (const sessionId of sessionIdsToClean) {
          TerminalInstanceManager.dispose(sessionId);
        }

        // Clean up AI event sequence tracking to prevent memory leak
        import("@/hooks/useAiEvents").then(({ resetSessionSequence }) => {
          for (const sessionId of sessionIdsToClean) {
            resetSessionSequence(sessionId);
          }
        });

        set((state) => {
          const layout = state.tabLayouts[tabId];
          if (!layout) {
            // No layout - just remove the session directly (backward compatibility)
            delete state.sessions[tabId];
            delete state.timelines[tabId];
            delete state.pendingCommand[tabId];
            delete state.lastSentCommand[tabId];
            delete state.agentStreamingBuffer[tabId];
            delete state.agentStreaming[tabId];
            delete state.streamingBlocks[tabId];
            delete state.streamingTextOffset[tabId];
            delete state.agentInitialized[tabId];
            delete state.isAgentThinking[tabId];
            delete state.isAgentResponding[tabId];
            delete state.pendingToolApproval[tabId];
            delete state.pendingAskHuman[tabId];
            delete state.processedToolRequests[tabId];
            delete state.activeToolCalls[tabId];
            delete state.thinkingContent[tabId];
            delete state.isThinkingExpanded[tabId];
            delete state.contextMetrics[tabId];
            delete state.tabHasNewActivity[tabId];

            // Remove from activation history
            state.tabActivationHistory = state.tabActivationHistory.filter((id) => id !== tabId);
            if (state.activeSessionId === tabId) {
              // Switch to the most recently active tab
              state.activeSessionId =
                state.tabActivationHistory[state.tabActivationHistory.length - 1] ?? null;
            }
            return;
          }

          // Get all pane sessions in this tab
          const panes = getAllLeafPanes(layout.root);

          // Remove state for each pane session
          for (const pane of panes) {
            const sessionId = pane.sessionId;
            delete state.sessions[sessionId];
            delete state.timelines[sessionId];
            delete state.pendingCommand[sessionId];
            delete state.lastSentCommand[sessionId];
            delete state.agentStreamingBuffer[sessionId];
            delete state.agentStreaming[sessionId];
            delete state.streamingBlocks[sessionId];
            delete state.streamingTextOffset[sessionId];
            delete state.agentInitialized[sessionId];
            delete state.isAgentThinking[sessionId];
            delete state.isAgentResponding[sessionId];
            delete state.pendingToolApproval[sessionId];
            delete state.pendingAskHuman[sessionId];
            delete state.processedToolRequests[sessionId];
            delete state.activeToolCalls[sessionId];
            delete state.thinkingContent[sessionId];
            delete state.gitStatus[sessionId];
            delete state.gitStatusLoading[sessionId];
            delete state.gitCommitMessage[sessionId];
            delete state.isThinkingExpanded[sessionId];
            delete state.contextMetrics[sessionId];
            delete state.tabHasNewActivity[sessionId];
          }

          // Remove the tab layout
          delete state.tabLayouts[tabId];
          delete state.tabHasNewActivity[tabId];
          const tabOrderIdx = state.tabOrder.indexOf(tabId);
          if (tabOrderIdx !== -1) {
            state.tabOrder.splice(tabOrderIdx, 1);
          }

          // Remove from activation history
          state.tabActivationHistory = state.tabActivationHistory.filter((id) => id !== tabId);
          // Update active session if needed
          if (state.activeSessionId === tabId) {
            // Switch to the most recently active tab
            state.activeSessionId =
              state.tabActivationHistory[state.tabActivationHistory.length - 1] ?? null;
          }
        });
      },

      markTabNewActivityBySession: (sessionId) =>
        set((state) => {
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
        set((state) => {
          logger.info("[store] moveTabToPane: start", {
            sourceTabId,
            destTabId,
            location,
          });
          const sourceLayout = state.tabLayouts[sourceTabId];
          const destLayout = state.tabLayouts[destTabId];
          if (!sourceLayout || !destLayout) {
            logger.warn("[store] moveTabToPane: missing layout", {
              hasSourceLayout: !!sourceLayout,
              hasDestLayout: !!destLayout,
            });
            return;
          }
          // Can only convert terminal tabs
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
          // Check pane limit on destination
          const destPaneCount = countLeafPanes(destLayout.root);
          const sourcePaneCount = countLeafPanes(sourceLayout.root);
          if (destPaneCount + sourcePaneCount > 4) {
            logger.warn("[store] moveTabToPane: pane limit exceeded", {
              destPaneCount,
              sourcePaneCount,
            });
            return;
          }

          // Determine split direction
          const direction: SplitDirection =
            location === "left" || location === "right" ? "vertical" : "horizontal";

          const newPaneId = crypto.randomUUID();

          // Wrap: the existing dest root + a new leaf with the source session
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

          // Remove the source tab layout (but keep the session alive)
          delete state.tabLayouts[sourceTabId];

          // Remove from tabOrder
          const tabOrderIdx = state.tabOrder.indexOf(sourceTabId);
          if (tabOrderIdx !== -1) {
            state.tabOrder.splice(tabOrderIdx, 1);
          }

          // Clean up tab activity for source tab
          delete state.tabHasNewActivity[sourceTabId];

          // If the source was active, switch to dest tab
          if (state.activeSessionId === sourceTabId) {
            state.activeSessionId = destTabId;
            // Track in activation history for MRU tab switching
            const histIdx = state.tabActivationHistory.indexOf(destTabId);
            if (histIdx !== -1) {
              state.tabActivationHistory.splice(histIdx, 1);
            }
            state.tabActivationHistory.push(destTabId);
          }

          // Focus the new pane
          state.tabLayouts[destTabId].focusedPaneId = newPaneId;
          logger.info("[store] moveTabToPane: completed", {
            sourceTabId,
            destTabId,
            newPaneId,
            direction,
          });
        }),

      setCurrentProject: (name, path) =>
        set((state) => {
          state.currentProjectName = name;
          state.currentProjectPath = path ?? null;
        }),
    })),
    { name: "golish" }
  )
);

// Stable empty arrays to avoid re-render loops (declared once to prevent recreation)
// Frozen to ensure immutability and prevent accidental mutations
const EMPTY_TIMELINE = Object.freeze([]) as unknown as UnifiedBlock[];
const EMPTY_TOOL_CALLS = Object.freeze([]) as unknown as ActiveToolCall[];
const EMPTY_STREAMING_BLOCKS = Object.freeze([]) as unknown as StreamingBlock[];

// Import derived selectors for Single Source of Truth pattern
import { memoizedSelectAgentMessages, memoizedSelectCommandBlocks } from "@/lib/timeline/selectors";

// Selectors
export const useActiveSession = () =>
  useStore((state) => {
    const id = state.activeSessionId;
    return id ? state.sessions[id] : null;
  });

/**
 * Get command blocks for a session.
 * Derives from the unified timeline (Single Source of Truth).
 */
export const useSessionBlocks = (sessionId: string) =>
  useStore((state) => memoizedSelectCommandBlocks(sessionId, state.timelines[sessionId]));

export const useTerminalClearRequest = (sessionId: string) =>
  useStore((state) => state.terminalClearRequest[sessionId] ?? 0);

export const usePendingCommand = (sessionId: string) =>
  useStore((state) => state.pendingCommand[sessionId]);

export const useSessionMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.mode ?? "terminal");

/**
 * Get agent messages for a session.
 * Derives from the unified timeline (Single Source of Truth).
 */
export const useAgentMessages = (sessionId: string) =>
  useStore((state) => memoizedSelectAgentMessages(sessionId, state.timelines[sessionId]));

export const useAgentStreaming = (sessionId: string) =>
  useStore((state) => state.agentStreaming[sessionId] ?? "");

export const useAgentInitialized = (sessionId: string) =>
  useStore((state) => state.agentInitialized[sessionId] ?? false);

export const usePendingToolApproval = (sessionId: string) =>
  useStore((state) => state.pendingToolApproval[sessionId] ?? null);

export const usePendingAskHuman = (sessionId: string) =>
  useStore((state) => state.pendingAskHuman[sessionId] ?? null);

// Timeline selectors
export const useSessionTimeline = (sessionId: string) =>
  useStore((state) => state.timelines[sessionId] ?? EMPTY_TIMELINE);

export const useInputMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.inputMode ?? "terminal");

export const useAgentMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.agentMode ?? "default");

export const useRenderMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.renderMode ?? "timeline");

export const useGitBranch = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.gitBranch ?? null);

export const useGitStatus = (sessionId: string) =>
  useStore((state) => state.gitStatus[sessionId] ?? null);
export const useGitStatusLoading = (sessionId: string) =>
  useStore((state) => state.gitStatusLoading[sessionId] ?? false);
export const useGitCommitMessage = (sessionId: string) =>
  useStore((state) => state.gitCommitMessage[sessionId] ?? "");

// Active tool calls selector
export const useActiveToolCalls = (sessionId: string) =>
  useStore((state) => state.activeToolCalls[sessionId] ?? EMPTY_TOOL_CALLS);

// Streaming blocks selector
export const useStreamingBlocks = (sessionId: string) =>
  useStore((state) => state.streamingBlocks[sessionId] ?? EMPTY_STREAMING_BLOCKS);

// Streaming text length selector (for auto-scroll triggers)
export const useStreamingTextLength = (sessionId: string) =>
  useStore((state) => state.agentStreaming[sessionId]?.length ?? 0);

// AI config selector (global - for backwards compatibility)
export const useAiConfig = () => useStore((state) => state.aiConfig);

// Per-session AI config selector
export const useSessionAiConfig = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.aiConfig);

// Agent thinking selector
export const useIsAgentThinking = (sessionId: string) =>
  useStore((state) => state.isAgentThinking[sessionId] ?? false);

// Agent responding selector (true when agent is actively responding, from started to completed)
export const useIsAgentResponding = (sessionId: string) =>
  useStore((state) => state.isAgentResponding[sessionId] ?? false);

// Extended thinking content selectors
export const useThinkingContent = (sessionId: string) =>
  useStore((state) => state.thinkingContent[sessionId] ?? "");

export const useIsThinkingExpanded = (sessionId: string) =>
  useStore((state) => state.isThinkingExpanded[sessionId] ?? true);

// Context metrics selector (uses slice selector)
export const useContextMetrics = (sessionId: string) =>
  useStore((state) => selectContextMetrics(state, sessionId));

// Conversation selectors
export const useActiveConversation = () => useStore((state) => selectActiveConversation(state));

export const useAllConversations = () => useStore((state) => selectAllConversations(state));

export const useActiveConversationTerminals = () =>
  useStore((state) => selectActiveConversationTerminals(state));

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
 * A session may be the tab's root session, or a pane session within the tab.
 * Returns the tab ID (root session ID) that contains this session.
 * Returns null if the session is not found in any tab.
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

// Helper function to clear conversation (both frontend and backend)
// This should be called instead of clearTimeline when you want to reset AI context
export async function clearConversation(sessionId: string): Promise<void> {
  // Clear frontend state
  useStore.getState().clearTimeline(sessionId);

  // Clear backend conversation history (try session-specific first, fall back to global)
  try {
    const { clearAiConversationSession, clearAiConversation } = await import("@/lib/ai");
    // Try session-specific clear first
    try {
      await clearAiConversationSession(sessionId);
    } catch {
      // Fall back to global clear (legacy)
      await clearAiConversation();
    }
  } catch (error) {
    logger.warn("Failed to clear backend conversation history:", error);
  }
}

// Helper function to restore a previous session (both frontend and backend)
export async function restoreSession(sessionId: string, identifier: string): Promise<void> {
  const aiModule = await import("@/lib/ai");
  const { loadAiSession, restoreAiSession, initAiSession, buildProviderConfig } = aiModule;
  const { getSettings } = await import("@/lib/settings");

  // First, load the session data from disk (doesn't require AI bridge)
  const session = await loadAiSession(identifier);
  if (!session) {
    throw new Error(`Session '${identifier}' not found`);
  }

  // Get current settings and use the user's current default provider
  // (not the session's original provider - conversation history is provider-agnostic)
  const settings = await getSettings();
  const workspace = session.workspace_path;

  logger.info(
    `Restoring session (original: ${session.provider}/${session.model}, ` +
      `using current: ${settings.ai.default_provider}/${settings.ai.default_model})`
  );

  // Build config using the user's current default provider
  const config = await buildProviderConfig(settings, workspace);

  // Initialize the AI bridge BEFORE restoring messages
  await initAiSession(sessionId, config);

  // Update the store's AI config for this session with the current provider
  useStore.getState().setSessionAiConfig(sessionId, {
    provider: settings.ai.default_provider,
    model: settings.ai.default_model,
    status: "ready",
  });

  // Now restore the backend conversation history (bridge is initialized)
  await restoreAiSession(sessionId, identifier);

  // Convert session messages to AgentMessages for the UI
  const agentMessages: AgentMessage[] = session.messages
    .filter((msg) => msg.role === "user" || msg.role === "assistant")
    .map((msg, index) => ({
      id: `restored-${identifier}-${index}`,
      sessionId,
      role: msg.role as "user" | "assistant",
      content: msg.content,
      timestamp: index === 0 ? session.started_at : session.ended_at,
      isStreaming: false,
    }));

  // Clear existing state first
  useStore.getState().clearTimeline(sessionId);

  // Restore the messages to the store (this also populates the timeline)
  useStore.getState().restoreAgentMessages(sessionId, agentMessages);

  // Switch to agent mode since we're restoring an AI conversation
  useStore.getState().setInputMode(sessionId, "agent");
}

// Expose store for testing in development
if (import.meta.env.DEV) {
  (window as unknown as { __QBIT_STORE__: typeof useStore }).__QBIT_STORE__ = useStore;
}
