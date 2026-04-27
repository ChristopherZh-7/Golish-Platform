/**
 * DB-backed conversation persistence — replaces workspace.json.
 *
 * Provides load/save functions that read from and write to PostgreSQL
 * via Tauri commands, replacing the file-based workspace-storage.ts.
 */

import { listen } from "@tauri-apps/api/event";
import { logger } from "@/lib/logger";
import {
  convList,
  convLoadMessages,
  convLoadPreferences,
  convLoadTerminalStates,
  convLoadTimeline,
  convSaveBatch,
  type ChatMessageRow,
  type ConvBatchItem,
  type TimelineBlockRow,
} from "@/lib/conversation-db";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import type { ChatConversation, ChatMessage, ChatToolCall } from "@/store/slices/conversation";
import type { Session, UnifiedBlock } from "@/store";

/** Normalize a project path for consistent DB lookups (strip trailing slash, resolve ~). */
function normalizePath(p: string): string {
  let normalized = p;
  // Strip trailing slashes (but keep root /)
  while (normalized.length > 1 && normalized.endsWith("/")) {
    normalized = normalized.slice(0, -1);
  }
  return normalized;
}

const SAVE_DEBOUNCE_MS = 2000;
const MAX_SCROLLBACK = 100_000;
const MAX_BLOCK_OUTPUT = 50_000;
const SAVE_MAX_RETRIES = 2;
const SAVE_RETRY_BASE_MS = 300;

/**
 * Module-level handle to the current auto-saver's immediate-save function.
 * Set by `createDbAutoSaver`; callable from anywhere to bypass the debounce
 * (e.g. after execution-mode or sub-agents toggle changes).
 */
let _flushSaveFn: (() => Promise<void>) | null = null;

/** Trigger an immediate DB save, bypassing the 2 s debounce. No-op if the auto-saver isn't active. */
export function flushDbSave(): Promise<void> {
  return _flushSaveFn ? _flushSaveFn() : Promise.resolve();
}

let _dbLoadOk = false;
export function markDbLoadSucceeded() { _dbLoadOk = true; }
export function isDbLoadOk() { return _dbLoadOk; }

async function withRetry<T>(
  fn: () => Promise<T>,
  retries = SAVE_MAX_RETRIES,
  baseDelay = SAVE_RETRY_BASE_MS,
): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await fn();
    } catch (e) {
      if (attempt >= retries) throw e;
      await new Promise((r) => setTimeout(r, baseDelay * (attempt + 1)));
    }
  }
}

/**
 * Build a lightweight fingerprint that captures actual content changes,
 * not just collection sizes. Uses a fast incremental hash (djb2) so we
 * avoid serializing the entire store on every tick.
 */
function buildChangeFingerprint(state: {
  conversations: Record<string, ChatConversation>;
  conversationOrder: string[];
  activeConversationId: string | null;
  conversationTerminals: Record<string, string[]>;
  sessions: Record<string, Session>;
  timelines: Record<string, UnifiedBlock[]>;
  selectedAiModel: { model: string; provider: string } | null;
  approvalMode: string;
}): string {
  let hash = 5381;
  const feed = (s: string) => {
    for (let i = 0; i < s.length; i++) {
      hash = ((hash << 5) + hash + s.charCodeAt(i)) | 0;
    }
  };

  feed(state.activeConversationId ?? "");
  feed(state.approvalMode);
  feed(state.selectedAiModel?.model ?? "");
  feed(state.selectedAiModel?.provider ?? "");

  for (const id of state.conversationOrder) {
    feed(id);
    const conv = state.conversations[id];
    if (!conv) continue;
    feed(conv.title);
    feed(String(conv.messages.length));
    const msgs = conv.messages;
    const start = Math.max(0, msgs.length - 5);
    for (let i = start; i < msgs.length; i++) {
      const m = msgs[i];
      feed(m.id);
      feed(m.content.slice(0, 200));
      feed(m.content.slice(-200));
      feed(String(m.content.length));
      feed(m.isStreaming ? "s" : "d");
      feed(m.error ?? "");
      feed(String(m.toolCalls?.length ?? 0));
      if (m.toolCalls) {
        for (const tc of m.toolCalls) {
          feed(tc.name);
          feed(tc.result?.slice(0, 100) ?? "");
          feed(tc.success === undefined ? "" : String(tc.success));
        }
      }
    }

    const terms = state.conversationTerminals[id];
    if (terms) {
      for (const t of terms) {
        feed(t);
        const sess = state.sessions[t];
        if (sess) {
          feed(sess.executionMode ?? "");
          feed(sess.useAgents ? "a" : "");
          feed(String(sess.retiredPlans?.length ?? 0));
          feed(sess.planMessageId ?? "");
        }
      }
    }
  }

  for (const [tid, blocks] of Object.entries(state.timelines)) {
    feed(tid);
    feed(String(blocks.length));
    if (blocks.length > 0) {
      const last = blocks[blocks.length - 1];
      feed(last.id);
      feed(last.type);
    }
  }

  return String(hash);
}

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
  executionMode: string | null;
  useAgents: boolean | null;
  retiredPlansJson: unknown | null;
  planMessageId: string | null;
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
    toolCallOffsets: row.toolCallOffsets ?? undefined,
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
    toolCallOffsets: msg.toolCallOffsets ?? null,
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

  // Restore planStepIndex from data where it was embedded during save
  if (
    (row.blockType === "pipeline_progress" || row.blockType === "sub_agent_activity") &&
    data && typeof data === "object"
  ) {
    const d = data as Record<string, unknown>;
    const stored = d.__planStepIndex as number | undefined;
    if (stored != null) {
      delete d.__planStepIndex;
      const block = { ...base, planStepIndex: stored } as Record<string, unknown>;
      if (row.blockType === "sub_agent_activity" && row.batchId) {
        block.batchId = row.batchId;
      }
      return block as UnifiedBlock;
    }
  }

  if (row.blockType === "sub_agent_activity" && row.batchId) {
    return { ...base, batchId: row.batchId } as UnifiedBlock;
  }
  return base as UnifiedBlock;
}

/**
 * Load all workspace state from the database for a given project.
 * Returns null if no conversations exist (fresh project).
 *
 * Uses parallel fetching to reduce latency and avoid N+1 waterfalls.
 * Individual conversation load failures are tolerated — only that
 * conversation is skipped rather than failing the entire load.
 */
export async function loadFromDb(
  projectPath: string,
): Promise<LoadedWorkspaceState | null> {
  const normalized = normalizePath(projectPath);
  try {
    console.log("[ConvDbSync] Loading from DB for project:", normalized);
    const [convRows, prefs] = await Promise.all([
      convList(normalized),
      convLoadPreferences(normalized),
    ]);
    console.log("[ConvDbSync] Found conversations:", convRows.length);
    if (convRows.length === 0) return null;

    const conversationOrder = convRows
      .sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt - b.createdAt)
      .map((r) => r.id);

    // Fetch messages and terminal states for all conversations in parallel
    const loadResults = await Promise.allSettled(
      convRows.map(async (convRow) => {
        const [msgRows, termStates] = await Promise.all([
          convLoadMessages(convRow.id),
          convLoadTerminalStates(convRow.id),
        ]);

        const messages = msgRows
          .sort((a, b) => a.sortOrder - b.sortOrder)
          .map(dbMsgToChatMessage);

        const conv: ChatConversation = {
          id: convRow.id,
          title: convRow.title,
          messages,
          createdAt: convRow.createdAt,
          aiSessionId: convRow.aiSessionId,
          aiInitialized: false,
          isStreaming: false,
        };

        let terminals: LoadedTerminalData[] = [];
        if (termStates.length > 0) {
          const termResults = await Promise.allSettled(
            termStates.map(async (ts) => {
              const blockRows = await convLoadTimeline(ts.sessionId);
              const blocks = blockRows
                .sort((a, b) => a.sortOrder - b.sortOrder)
                .map(dbBlockToUnifiedBlock);
              return {
                sessionId: ts.sessionId,
                workingDirectory: ts.workingDirectory,
                scrollback: ts.scrollback,
                customName: ts.customName ?? null,
                timelineBlocks: blocks,
                planJson: ts.planJson ?? null,
                executionMode: ts.executionMode ?? null,
                useAgents: ts.useAgents ?? null,
                retiredPlansJson: ts.retiredPlansJson ?? null,
                planMessageId: ts.planMessageId ?? null,
              } satisfies LoadedTerminalData;
            }),
          );
          terminals = termResults.flatMap((result) =>
            result.status === "fulfilled" ? [result.value as LoadedTerminalData] : [],
          );
        }

        return { conv, terminals };
      }),
    );

    const conversations: ChatConversation[] = [];
    const terminalData: Record<string, LoadedTerminalData[]> = {};

    for (const result of loadResults) {
      if (result.status === "rejected") {
        logger.warn("[ConvDbSync] Failed to load a conversation, skipping:", result.reason);
        continue;
      }
      const { conv, terminals } = result.value;
      conversations.push(conv);
      if (terminals.length > 0) {
        terminalData[conv.id] = terminals;
      }
    }

    logger.info("[ConvDbSync] Loaded from DB:", {
      conversations: conversations.length,
      messages: conversations.reduce((sum, c) => sum + c.messages.length, 0),
      terminalDataKeys: Object.keys(terminalData),
      terminalBlockCounts: Object.fromEntries(
        Object.entries(terminalData).map(([k, v]) => [k, v.map((t) => ({ sessionId: t.sessionId, blocks: t.timelineBlocks.length, scrollback: t.scrollback.length }))])
      ),
    });

    return {
      conversations,
      conversationOrder: conversationOrder.filter((id) =>
        conversations.some((c) => c.id === id),
      ),
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

/**
 * Per-conversation fingerprint for incremental save.
 * Only conversations whose fingerprint differs from the last save are written.
 */
function convFingerprint(
  conv: ChatConversation,
  termIds: string[],
  timelines: Record<string, UnifiedBlock[]>,
  sessions: Record<string, Session>,
  sortOrder: number,
): string {
  let h = 5381;
  const feed = (s: string) => {
    for (let i = 0; i < s.length; i++) {
      h = ((h << 5) + h + s.charCodeAt(i)) | 0;
    }
  };
  feed(conv.id);
  feed(conv.title);
  feed(String(sortOrder));
  feed(String(conv.messages.length));
  const msgs = conv.messages;
  const start = Math.max(0, msgs.length - 8);
  for (let i = start; i < msgs.length; i++) {
    const m = msgs[i];
    feed(m.id);
    feed(String(m.content.length));
    feed(m.content.slice(-300));
    feed(m.isStreaming ? "s" : "d");
    feed(m.error ?? "");
    feed(String(m.toolCalls?.length ?? 0));
  }
  for (const tid of termIds) {
    feed(tid);
    const blocks = timelines[tid];
    feed(String(blocks?.length ?? 0));
    if (blocks && blocks.length > 0) {
      const last = blocks[blocks.length - 1];
      feed(last.id);
      feed(last.type);
    }
    const sess = sessions[tid];
    if (sess) {
      feed(sess.executionMode ?? "");
      feed(sess.useAgents ? "a" : "");
      feed(String(sess.retiredPlans?.length ?? 0));
      feed(sess.planMessageId ?? "");
    }
  }
  return String(h);
}

function buildTimelineDbBlocks(
  timeline: UnifiedBlock[],
  sessionId: string,
  conversationId: string,
): TimelineBlockRow[] {
  return timeline.map((block, idx) => {
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

    // Embed block-level planStepIndex into data for types that store it outside data
    const anyBlock = block as { planStepIndex?: number };
    if (
      (block.type === "pipeline_progress" || block.type === "sub_agent_activity") &&
      anyBlock.planStepIndex != null
    ) {
      data = { ...data, __planStepIndex: anyBlock.planStepIndex };
    }

    return {
      id: block.id,
      sessionId,
      conversationId,
      blockType: block.type,
      data: data as unknown,
      batchId: (block as { batchId?: string }).batchId ?? null,
      sortOrder: idx,
      timestamp: block.timestamp ?? null,
    };
  });
}

/** Tracks per-conversation fingerprints so we only write what changed. */
const _convSaveFingerprints = new Map<string, string>();

/** Clear fingerprint cache (call on project switch to avoid stale comparisons). */
export function clearSaveFingerprints(): void {
  _convSaveFingerprints.clear();
  _dbLoadOk = false;
}

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
  const normalized = normalizePath(projectPath);
  const convIds = conversationOrder.filter((id) => conversations[id]);

  const batchItems: ConvBatchItem[] = [];

  for (let i = 0; i < convIds.length; i++) {
    const conv = conversations[convIds[i]];
    if (!conv) continue;

    const termIds = conversationTerminals[conv.id] ?? [];
    const fp = convFingerprint(conv, termIds, timelines, sessions, i);
    if (_convSaveFingerprints.get(conv.id) === fp) continue;

    console.log("[ConvDbSync] Saving conv:", conv.id, {
      title: conv.title,
      messages: conv.messages.length,
      termIds,
      timelineBlocks: termIds.map((t) => [t, timelines[t]?.length ?? 0]),
    });

    const dbMessages = conv.messages
      .filter((m) => m.content || m.toolCalls?.length)
      .map((m, idx) => chatMessageToDbRow(m, conv.id, idx));

    const terminalStates: ConvBatchItem["terminalStates"] = [];
    const batchTimelines: ConvBatchItem["timelines"] = [];

    for (const tid of termIds) {
      const sess = sessions[tid];
      if (!sess?.workingDirectory) continue;

      const dbId = sess.logicalTerminalId ?? tid;

      let scrollback = TerminalInstanceManager.serialize(tid);
      if (scrollback.length > MAX_SCROLLBACK) {
        scrollback = scrollback.slice(-MAX_SCROLLBACK);
      }

      terminalStates.push({
        sessionId: dbId,
        conversationId: conv.id,
        workingDirectory: sess.workingDirectory,
        scrollback,
        customName: sess.customName ?? null,
        planJson: sess.plan ?? null,
        executionMode: sess.executionMode ?? null,
        useAgents: sess.useAgents ?? null,
        retiredPlansJson: sess.retiredPlans?.length ? sess.retiredPlans : null,
        planMessageId: sess.planMessageId ?? null,
      });

      const timeline = timelines[tid];
      if (timeline && timeline.length > 0) {
        batchTimelines.push({
          sessionId: dbId,
          conversationId: conv.id,
          blocks: buildTimelineDbBlocks(timeline, dbId, conv.id),
        });
      }
    }

    batchItems.push({
      conversation: {
        id: conv.id,
        title: conv.title,
        aiSessionId: conv.aiSessionId,
        projectPath: normalized,
        sortOrder: i,
        createdAt: conv.createdAt,
      },
      messages: dbMessages,
      terminalStates,
      timelines: batchTimelines,
    });
  }

  if (batchItems.length === 0) {
    return;
  }

  await withRetry(() =>
    convSaveBatch({
      projectPath: normalized,
      survivingIds: convIds,
      items: batchItems,
      preferences: {
        activeConversationId,
        aiModel,
        approvalMode: approvalMode || null,
        approvalPatterns: null,
      },
    }),
  );

  for (const item of batchItems) {
    const convId = item.conversation.id;
    const conv = conversations[convId];
    if (!conv) continue;
    const termIds = conversationTerminals[convId] ?? [];
    const idx = convIds.indexOf(convId);
    _convSaveFingerprints.set(convId, convFingerprint(conv, termIds, timelines, sessions, idx));
  }
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
    terminalRestoreInProgress: boolean;
    pendingTerminalRestoreData: unknown | null;
  },
): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let saving = false;
  let savePromise: Promise<void> | null = null;
  let lastSnapshot: string | null = null;

  const save = async () => {
    const projectPath = getProjectPath();
    if (!projectPath) return;
    if (saving) return;
    if (!_dbLoadOk) return;

    const state = getState();
    if (state.terminalRestoreInProgress || state.pendingTerminalRestoreData) return;

    const snapshot = buildChangeFingerprint(state);
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
      savePromise = null;
    }
  };

  const immediateFlush = async () => {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    lastSnapshot = null;
    if (savePromise) return savePromise;
    savePromise = save();
    return savePromise;
  };

  _flushSaveFn = immediateFlush;

  const debouncedSave = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void save(), SAVE_DEBOUNCE_MS);
  };

  console.log("[ConvDbSync] Auto-saver initialized, projectPath:", getProjectPath());
  const unsubscribe = subscribe(debouncedSave);

  const flushNow = (event?: BeforeUnloadEvent) => {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    lastSnapshot = null;
    savePromise = save();
    if (event) {
      event.returnValue = "";
    }
  };

  // Rust-side emits "flush-state" before the window is destroyed (300ms grace period).
  let unlistenFlush: (() => void) | null = null;
  listen("flush-state", () => flushNow()).then((fn) => { unlistenFlush = fn; }).catch(() => {});

  window.addEventListener("beforeunload", flushNow as EventListener);

  return () => {
    _flushSaveFn = null;
    unsubscribe();
    window.removeEventListener("beforeunload", flushNow as EventListener);
    if (unlistenFlush) unlistenFlush();
    if (timer) clearTimeout(timer);
  };
}
