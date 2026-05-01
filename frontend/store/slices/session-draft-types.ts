/**
 * Typed state subset accessed by session sub-modules during Immer mutations.
 *
 * Session lifecycle actions (addSession / removeSession) must bootstrap or
 * tear down per-session state across many slices.  Previously this was done
 * via `state: any` — this interface captures every cross-slice field that
 * session sub-modules actually touch, restoring compile-time safety without
 * changing the slice composition architecture.
 */

import type { TabLayout } from "@/lib/pane-utils";
import type {
  ActiveSubAgent,
  ActiveToolCall,
  ActiveWorkflow,
  AskHumanRequest,
  PendingCommand,
  Session,
  StreamingBlock,
  ToolCall,
  UnifiedBlock,
} from "../store-types";
import type { ContextMetrics } from "./context";

export interface SessionStoreDraft {
  // ── Session slice fields ───────────────────────────────────────────
  sessions: Record<string, Session>;
  activeSessionId: string | null;
  homeTabId: string | null;
  timelines: Record<string, UnifiedBlock[]>;
  streamingBlocks: Record<string, StreamingBlock[]>;
  streamingTextOffset: Record<string, number>;
  streamingBlockRevision: Record<string, number>;
  pendingCommand: Record<string, PendingCommand | null>;
  lastSentCommand: Record<string, string | null>;
  pipelineCommandSource: Record<string, boolean>;
  terminalClearRequest: Record<string, number>;
  tabOrder: string[];
  tabActivationHistory: string[];
  tabHasNewActivity: Record<string, boolean>;

  // ── AI slice fields ────────────────────────────────────────────────
  agentStreamingBuffer: Record<string, string[]>;
  agentStreaming: Record<string, string>;
  agentInitialized: Record<string, boolean>;
  isAgentThinking: Record<string, boolean>;
  isAgentResponding: Record<string, boolean>;
  activeToolCalls: Record<string, ActiveToolCall[]>;
  thinkingContent: Record<string, string>;
  isThinkingExpanded: Record<string, boolean>;

  // ── HITL slice fields ──────────────────────────────────────────────
  pendingToolApproval: Record<string, ToolCall | null>;
  pendingAskHuman: Record<string, AskHumanRequest | null>;

  // ── Workflow slice fields ──────────────────────────────────────────
  activeWorkflows: Record<string, ActiveWorkflow | null>;
  workflowHistory: Record<string, ActiveWorkflow[]>;
  activeSubAgents: Record<string, ActiveSubAgent[]>;

  // ── Context slice fields ───────────────────────────────────────────
  contextMetrics: Record<string, ContextMetrics>;
  compactionCount: Record<string, number>;
  isCompacting: Record<string, boolean>;
  isSessionDead: Record<string, boolean>;
  compactionError: Record<string, string | null>;

  // ── Git slice fields ───────────────────────────────────────────────
  gitStatus: Record<string, unknown>;
  gitStatusLoading: Record<string, boolean>;
  gitCommitMessage: Record<string, string>;

  // ── Pane slice fields ──────────────────────────────────────────────
  tabLayouts: Record<string, TabLayout>;

  // ── Conversation slice fields ──────────────────────────────────────
  conversationTerminals?: Record<string, string[]>;
  activeConversationId: string | null;
}
