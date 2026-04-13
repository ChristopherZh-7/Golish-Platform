/**
 * DB-backed conversation persistence — replaces workspace.json.
 *
 * Provides load/save functions that read from and write to PostgreSQL
 * via Tauri commands, replacing the file-based workspace-storage.ts.
 */

import { logger } from "@/lib/logger";
import {
  convDelete,
  convList,
  convLoadMessages,
  convLoadPreferences,
  convLoadTerminalStates,
  convLoadTimeline,
  convSave,
  convSaveMessages,
  convSavePreferences,
  convSaveTerminalState,
  convSaveTimeline,
  type ChatMessageRow,
  type TimelineBlockRow,
} from "@/lib/conversation-db";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import type { ChatConversation, ChatMessage, ChatToolCall } from "@/store/slices/conversation";
import type { Session, UnifiedBlock } from "@/store";

const SAVE_DEBOUNCE_MS = 1500;
const MAX_SCROLLBACK = 100_000;
const MAX_BLOCK_OUTPUT = 50_000;

// ─── Load from DB ────────────────────────────────────────────────────────────

export interface LoadedWorkspaceState {
  conversations: ChatConversation[];
  conversationOrder: string[];
  activeConversationId: string | null;
  terminalData: Record<string, LoadedTerminalData[]>;
  aiModel: { model: string; provider: string } | null;
  approvalMode: string | null;
}

export interface LoadedTerminalData {
  sessionId: string;
  workingDirectory: string;
  scrollback: string;
  customName: string | null;
  timelineBlocks: UnifiedBlock[];
  planJson: unknown | null;
}

function dbMsgToChatMessage(row: ChatMessageRow): ChatMessage {
  return {
    id: row.id,
    role: row.role as "user" | "assistant",
    content: row.content,
    timestamp: row.createdAt,
    thinking: row.thinking ?? undefined,
    error: row.error ?? undefined,
    toolCalls: row.toolCalls
      ? (row.toolCalls as ChatToolCall[])
      : undefined,
    toolCallsContentOffset: row.toolCallsContentOffset ?? undefined,
  };
}

function chatMessageToDbRow(
  msg: ChatMessage,
  conversationId: string,
  sortOrder: number,
): ChatMessageRow {
  return {
    id: msg.id,
    conversationId,
    role: msg.role,
    content: msg.content,
    thinking: msg.thinking ?? null,
    error: msg.error ?? null,
    toolCalls: msg.toolCalls ? (msg.toolCalls as unknown as unknown[]) : null,
    toolCallsContentOffset: msg.toolCallsContentOffset ?? null,
    sortOrder,
    createdAt: msg.timestamp,
  };
}

function dbBlockToUnifiedBlock(row: TimelineBlockRow): UnifiedBlock {
  let data = row.data;
  if (row.blockType === "command" && data && typeof data === "object") {
    data = { ...data, isCollapsed: false };
  }

  const base = {
    id: row.id,
    type: row.blockType as UnifiedBlock["type"],
    timestamp: row.timestamp ?? new Date().toISOString(),
    data,
  };

  if (row.blockType === "sub_agent_activity" && row.batchId) {
    return { ...base, batchId: row.batchId } as UnifiedBlock;
  }
  return base as UnifiedBlock;
}

/**
 * Load all workspace state from the database for a given project.
 * Returns null if no conversations exist (fresh project).
 */
export async function loadFromDb(
  projectPath: string,
): Promise<LoadedWorkspaceState | null> {
  try {
    console.log("[ConvDbSync] Loading from DB for project:", projectPath);
    const convRows = await convList(projectPath);
    console.log("[ConvDbSync] Found conversations:", convRows.length);
    if (convRows.length === 0) return null;

    const prefs = await convLoadPreferences(projectPath);

    const conversations: ChatConversation[] = [];
    const terminalData: Record<string, LoadedTerminalData[]> = {};
    const conversationOrder = convRows
      .sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt - b.createdAt)
      .map((r) => r.id);

    for (const convRow of convRows) {
      const msgRows = await convLoadMessages(convRow.id);
      const messages = msgRows
        .sort((a, b) => a.sortOrder - b.sortOrder)
        .map(dbMsgToChatMessage);

      conversations.push({
        id: convRow.id,
        title: convRow.title,
        messages,
        createdAt: convRow.createdAt,
        aiSessionId: convRow.aiSessionId,
        aiInitialized: false,
        isStreaming: false,
      });

      const termStates = await convLoadTerminalStates(convRow.id);
      if (termStates.length > 0) {
        const loaded: LoadedTerminalData[] = [];
        for (const ts of termStates) {
          const blockRows = await convLoadTimeline(ts.sessionId);
          const blocks = blockRows
            .sort((a, b) => a.sortOrder - b.sortOrder)
            .map(dbBlockToUnifiedBlock);

          loaded.push({
            sessionId: ts.sessionId,
            workingDirectory: ts.workingDirectory,
            scrollback: ts.scrollback,
            customName: ts.customName ?? null,
            timelineBlocks: blocks,
            planJson: ts.planJson ?? null,
          });
        }
        terminalData[convRow.id] = loaded;
      }
    }

    logger.info("[ConvDbSync] Loaded from DB:", {
      conversations: conversations.length,
      messages: conversations.reduce((sum, c) => sum + c.messages.length, 0),
    });

    return {
      conversations,
      conversationOrder,
      activeConversationId: prefs?.activeConversationId ?? null,
      terminalData,
      aiModel: prefs?.aiModel as LoadedWorkspaceState["aiModel"],
      approvalMode: prefs?.approvalMode ?? null,
    };
  } catch (e) {
    logger.warn("[ConvDbSync] Failed to load from DB:", e);
    return null;
  }
}

// ─── Save to DB ──────────────────────────────────────────────────────────────

async function saveConversationsToDb(
  conversations: Record<string, ChatConversation>,
  conversationOrder: string[],
  conversationTerminals: Record<string, string[]>,
  sessions: Record<string, Session>,
  timelines: Record<string, UnifiedBlock[]>,
  projectPath: string,
  activeConversationId: string | null,
  aiModel: { model: string; provider: string } | null,
  approvalMode: string,
): Promise<void> {
  const convIds = conversationOrder.filter((id) => conversations[id]);

  // Save conversations + messages
  for (let i = 0; i < convIds.length; i++) {
    const conv = conversations[convIds[i]];
    if (!conv) continue;

    await convSave({
      id: conv.id,
      title: conv.title,
      aiSessionId: conv.aiSessionId,
      projectPath,
      sortOrder: i,
      createdAt: conv.createdAt,
    });

    const dbMessages = conv.messages
      .filter((m) => !m.isStreaming)
      .map((m, idx) => chatMessageToDbRow(m, conv.id, idx));

    if (dbMessages.length > 0) {
      await convSaveMessages(conv.id, dbMessages);
    }

    // Save terminal state + timeline for this conversation's terminals
    const termIds = conversationTerminals[conv.id] ?? [];
    for (const tid of termIds) {
      const sess = sessions[tid];
      if (!sess?.workingDirectory) continue;

      let scrollback = TerminalInstanceManager.serialize(tid);
      if (scrollback.length > MAX_SCROLLBACK) {
        scrollback = scrollback.slice(-MAX_SCROLLBACK);
      }

      await convSaveTerminalState({
        sessionId: tid,
        conversationId: conv.id,
        workingDirectory: sess.workingDirectory,
        scrollback,
        customName: sess.customName ?? null,
        planJson: sess.plan ?? null,
      });

      const timeline = timelines[tid];
      if (timeline && timeline.length > 0) {
        const dbBlocks: TimelineBlockRow[] = timeline.map((block, idx) => {
          let data = block.data as Record<string, unknown>;
          if (block.type === "command") {
            const d = data as Record<string, unknown>;
            let output = (d.output as string) || "";
            if (output.length > MAX_BLOCK_OUTPUT) {
              output = output.slice(-MAX_BLOCK_OUTPUT);
            }
            data = { ...d, output };
          } else if (block.type === "ai_tool_execution") {
            const d = data as Record<string, unknown>;
            let streamingOutput = (d.streamingOutput as string) || "";
            if (streamingOutput.length > MAX_BLOCK_OUTPUT) {
              streamingOutput = streamingOutput.slice(-MAX_BLOCK_OUTPUT);
            }
            data = { ...d, streamingOutput };
          } else if (block.type === "sub_agent_activity") {
            const d = data as Record<string, unknown>;
            let streamingText = (d.streamingText as string) || "";
            if (streamingText.length > MAX_BLOCK_OUTPUT) {
              streamingText = streamingText.slice(-MAX_BLOCK_OUTPUT);
            }
            data = { ...d, streamingText };
          }

          return {
            id: block.id,
            sessionId: tid,
            conversationId: conv.id,
            blockType: block.type,
            data: data as unknown,
            batchId: (block as { batchId?: string }).batchId ?? null,
            sortOrder: idx,
            timestamp: block.timestamp ?? null,
          };
        });

        await convSaveTimeline(tid, conv.id, dbBlocks);
      }
    }
  }

  // Delete conversations that no longer exist
  try {
    const existingRows = await convList(projectPath);
    const currentIds = new Set(convIds);
    for (const row of existingRows) {
      if (!currentIds.has(row.id)) {
        await convDelete(row.id);
      }
    }
  } catch { /* best effort */ }

  await convSavePreferences(projectPath, {
    activeConversationId,
    aiModel,
    approvalMode: approvalMode || null,
    approvalPatterns: null,
  });
}

/**
 * Creates a debounced auto-saver that subscribes to store changes
 * and persists to PostgreSQL instead of workspace.json.
 * Returns an unsubscribe function.
 */
export function createDbAutoSaver(
  getProjectPath: () => string | null,
  subscribe: (listener: () => void) => () => void,
  getState: () => {
    conversations: Record<string, ChatConversation>;
    conversationOrder: string[];
    activeConversationId: string | null;
    conversationTerminals: Record<string, string[]>;
    sessions: Record<string, Session>;
    timelines: Record<string, UnifiedBlock[]>;
    selectedAiModel: { model: string; provider: string } | null;
    approvalMode: string;
  },
): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let saving = false;
  let lastSnapshot: string | null = null;

  const save = async () => {
    const projectPath = getProjectPath();
    if (!projectPath) return;
    if (saving) return;

    const state = getState();
    const snapshot = JSON.stringify([
      state.conversationOrder,
      Object.keys(state.conversations).length,
      Object.values(state.conversationTerminals),
      Object.keys(state.timelines).map((k) => [k, state.timelines[k]?.length ?? 0]),
      state.activeConversationId,
      state.selectedAiModel,
      state.approvalMode,
    ]);
    if (snapshot === lastSnapshot) return;

    saving = true;
    try {
      lastSnapshot = snapshot;
      await saveConversationsToDb(
        state.conversations,
        state.conversationOrder,
        state.conversationTerminals,
        state.sessions,
        state.timelines,
        projectPath,
        state.activeConversationId,
        state.selectedAiModel,
        state.approvalMode,
      );
    } catch (e) {
      console.error("[ConvDbSync] Save FAILED:", e);
      lastSnapshot = null;
    } finally {
      saving = false;
    }
  };

  const debouncedSave = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void save(), SAVE_DEBOUNCE_MS);
  };

  console.log("[ConvDbSync] Auto-saver initialized, projectPath:", getProjectPath());
  const unsubscribe = subscribe(debouncedSave);

  const handleBeforeUnload = () => {
    void save();
  };
  window.addEventListener("beforeunload", handleBeforeUnload);

  return () => {
    unsubscribe();
    window.removeEventListener("beforeunload", handleBeforeUnload);
    if (timer) clearTimeout(timer);
  };
}
