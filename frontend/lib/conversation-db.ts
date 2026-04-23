/**
 * Typed wrappers for conversation persistence Tauri commands.
 * These replace workspace.json read/write with PostgreSQL-backed storage.
 */

import { invoke } from "@tauri-apps/api/core";

// ─── Types ───────────────────────────────────────────────────────────────────

export interface ConversationRow {
  id: string;
  title: string;
  aiSessionId: string;
  projectPath: string | null;
  sortOrder: number;
  createdAt: number;
}

export interface ChatMessageRow {
  id: string;
  conversationId: string;
  role: "user" | "assistant";
  content: string;
  thinking: string | null;
  error: string | null;
  toolCalls: unknown[] | null;
  toolCallsContentOffset: number | null;
  toolCallOffsets: number[] | null;
  sortOrder: number;
  createdAt: number;
}

export interface TimelineBlockRow {
  id: string;
  sessionId: string;
  conversationId: string | null;
  blockType: string;
  data: unknown;
  batchId: string | null;
  sortOrder: number;
  timestamp?: string | null;
}

export interface TerminalStateRow {
  sessionId: string;
  conversationId: string | null;
  workingDirectory: string;
  scrollback: string;
  customName: string | null;
  planJson: unknown | null;
  executionMode: string | null;
  useAgents: boolean | null;
  retiredPlansJson: unknown | null;
  planMessageId: string | null;
}

export interface WorkspacePreferences {
  activeConversationId: string | null;
  aiModel: { model: string; provider: string } | null;
  approvalMode: string | null;
  approvalPatterns: unknown | null;
}

// ─── Conversations ───────────────────────────────────────────────────────────

export async function convSave(conversation: ConversationRow): Promise<void> {
  await invoke("conv_save", { conversation });
}

export async function convDelete(conversationId: string): Promise<void> {
  await invoke("conv_delete", { conversationId });
}

export async function convList(
  projectPath?: string | null
): Promise<ConversationRow[]> {
  return invoke<ConversationRow[]>("conv_list", {
    projectPath: projectPath ?? null,
  });
}

// ─── Chat Messages ───────────────────────────────────────────────────────────

export async function convSaveMessages(
  conversationId: string,
  messages: ChatMessageRow[]
): Promise<void> {
  await invoke("conv_save_messages", { conversationId, messages });
}

export async function convLoadMessages(
  conversationId: string
): Promise<ChatMessageRow[]> {
  return invoke<ChatMessageRow[]>("conv_load_messages", { conversationId });
}

// ─── Timeline Blocks ─────────────────────────────────────────────────────────

export async function convSaveTimeline(
  sessionId: string,
  conversationId: string | null,
  blocks: TimelineBlockRow[]
): Promise<void> {
  await invoke("conv_save_timeline", { sessionId, conversationId, blocks });
}

export async function convLoadTimeline(
  sessionId: string
): Promise<TimelineBlockRow[]> {
  return invoke<TimelineBlockRow[]>("conv_load_timeline", { sessionId });
}

// ─── Terminal State ──────────────────────────────────────────────────────────

export async function convSaveTerminalState(
  terminal: TerminalStateRow
): Promise<void> {
  await invoke("conv_save_terminal_state", { terminal });
}

export async function convLoadTerminalStates(
  conversationId: string
): Promise<TerminalStateRow[]> {
  return invoke<TerminalStateRow[]>("conv_load_terminal_states", {
    conversationId,
  });
}

// ─── Batch Save ─────────────────────────────────────────────────────────────

export interface BatchTimelineEntry {
  sessionId: string;
  conversationId: string | null;
  blocks: TimelineBlockRow[];
}

export interface ConvBatchItem {
  conversation: ConversationRow;
  messages: ChatMessageRow[];
  terminalStates: TerminalStateRow[];
  timelines: BatchTimelineEntry[];
}

export interface ConvBatchSavePayload {
  projectPath: string;
  survivingIds: string[];
  items: ConvBatchItem[];
  preferences: WorkspacePreferences;
}

export async function convSaveBatch(
  payload: ConvBatchSavePayload
): Promise<void> {
  await invoke("conv_save_batch", { payload });
}

// ─── Workspace Preferences ──────────────────────────────────────────────────

export async function convSavePreferences(
  projectPath: string,
  prefs: WorkspacePreferences
): Promise<void> {
  await invoke("conv_save_preferences", { projectPath, prefs });
}

export async function convLoadPreferences(
  projectPath: string
): Promise<WorkspacePreferences | null> {
  return invoke<WorkspacePreferences | null>("conv_load_preferences", {
    projectPath,
  });
}
