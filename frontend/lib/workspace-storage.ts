/**
 * Workspace state persistence — per-project.
 *
 * All project-specific data is persisted to <rootPath>/.golish/workspace.json
 * via backend IPC. This includes conversations, terminal scrollback, timeline
 * command blocks, AI model selection, and approval mode.
 *
 * localStorage serves only as a sync backup for beforeunload and is
 * hydrated from workspace.json on load.
 *
 * A lightweight localStorage key tracks the last-opened project
 * so the app can auto-restore on restart.
 */

import { logger } from "@/lib/logger";
import { loadProjectWorkspace, saveProjectWorkspace } from "@/lib/projects";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import type { ChatConversation, ChatMessage } from "@/store/slices/conversation";
import type { CommandBlock, Session, UnifiedBlock } from "@/store";

const LAST_PROJECT_KEY = "golish-last-project";
const SAVE_DEBOUNCE_MS = 1000;

const MAX_SCROLLBACK = 100_000;
const MAX_BLOCK_OUTPUT = 50_000;
const MAX_BLOCKS = 50;

export interface PersistedCommandBlock {
  id: string;
  type: "command";
  timestamp: string;
  data: CommandBlock;
}

export interface PersistedTerminalData {
  workingDirectory: string;
  scrollback: string;
  customName?: string;
  timelineBlocks?: PersistedCommandBlock[];
}

export interface PersistedWorkspaceState {
  version: 1 | 2;
  savedAt: number;
  conversations: PersistedConversation[];
  conversationOrder: string[];
  activeConversationId: string | null;
  terminalTabs?: PersistedTerminalTab[];
  conversationTerminalData?: Record<string, PersistedTerminalData[]>;
  aiModel?: { model: string; provider: string } | null;
  approvalMode?: string;
}

interface PersistedConversation {
  id: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
  aiSessionId: string;
}

export interface PersistedTerminalTab {
  workingDirectory: string;
}

function toPersistedConversation(conv: ChatConversation): PersistedConversation {
  return {
    id: conv.id,
    title: conv.title,
    messages: conv.messages,
    createdAt: conv.createdAt,
    aiSessionId: conv.aiSessionId,
  };
}

export function toChatConversation(p: PersistedConversation): ChatConversation {
  return {
    ...p,
    aiInitialized: false,
    isStreaming: false,
  };
}

/** Get the last-opened project name. */
export function getLastProjectName(): string | null {
  return localStorage.getItem(LAST_PROJECT_KEY);
}

/** Set the last-opened project name. */
export function setLastProjectName(name: string): void {
  localStorage.setItem(LAST_PROJECT_KEY, name);
}

/** Clear the last-opened project. */
export function clearLastProjectName(): void {
  localStorage.removeItem(LAST_PROJECT_KEY);
}

/** Save workspace state for a project (via backend filesystem). */
export async function saveWorkspaceState(
  projectName: string,
  conversations: Record<string, ChatConversation>,
  conversationOrder: string[],
  activeConversationId: string | null,
  terminalTabs?: PersistedTerminalTab[],
  conversationTerminalData?: Record<string, PersistedTerminalData[]>,
  aiModel?: { model: string; provider: string } | null,
  approvalMode?: string,
): Promise<void> {
  try {
    const state: PersistedWorkspaceState = {
      version: 2,
      savedAt: Date.now(),
      conversations: conversationOrder
        .map((id) => conversations[id])
        .filter(Boolean)
        .map(toPersistedConversation),
      conversationOrder,
      activeConversationId,
      terminalTabs,
      conversationTerminalData,
      aiModel,
      approvalMode,
    };

    await saveProjectWorkspace(projectName, JSON.stringify(state));
    logger.debug("[WorkspaceStorage] Saved for project:", projectName, {
      conversations: state.conversations.length,
      terminalTabs: terminalTabs?.length ?? 0,
      terminalDataConvs: conversationTerminalData ? Object.keys(conversationTerminalData).length : 0,
    });
  } catch (e) {
    logger.warn("[WorkspaceStorage] Failed to save:", e);
  }
}

/** Load workspace state for a project. Returns null if none exists. */
export async function loadWorkspaceState(
  projectName: string,
): Promise<PersistedWorkspaceState | null> {
  try {
    const raw = await loadProjectWorkspace(projectName);
    if (!raw) return null;

    const state = JSON.parse(raw) as PersistedWorkspaceState;

    if ((state.version !== 1 && state.version !== 2) || !Array.isArray(state.conversations)) {
      logger.warn("[WorkspaceStorage] Invalid format, discarding");
      return null;
    }

    logger.info("[WorkspaceStorage] Loaded for project:", projectName, {
      conversations: state.conversations.length,
      messages: state.conversations.reduce((sum, c) => sum + c.messages.length, 0),
    });

    return state;
  } catch (e) {
    logger.warn("[WorkspaceStorage] Failed to load:", e);
    return null;
  }
}

/**
 * Creates a debounced auto-saver that subscribes to store changes.
 * Returns an unsubscribe function.
 */
const LOCAL_STORAGE_BACKUP_KEY = "golish-pentest-conversations";

export function createWorkspaceAutoSaver(
  getProjectName: () => string | null,
  subscribe: (listener: () => void) => () => void,
  getState: () => {
    conversations: Record<string, ChatConversation>;
    conversationOrder: string[];
    activeConversationId: string | null;
    terminalTabs: PersistedTerminalTab[];
    conversationTerminals: Record<string, string[]>;
    sessions: Record<string, Session>;
    timelines: Record<string, UnifiedBlock[]>;
  },
): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;

  const buildTerminalData = (): Record<string, PersistedTerminalData[]> => {
    const { conversationOrder, conversations, conversationTerminals, sessions, timelines } = getState();
    const convIds = conversationOrder.filter((id) => conversations[id]?.messages?.length > 0);
    let existingTermData: Record<string, PersistedTerminalData[]> = {};
    try {
      const raw = localStorage.getItem("golish-pentest-conv-terminals");
      if (raw) existingTermData = JSON.parse(raw);
    } catch { /* ignore */ }
    const termData: Record<string, PersistedTerminalData[]> = {};
    for (const cid of convIds) {
      const termIds = conversationTerminals[cid] ?? [];
      if (termIds.length === 0) {
        if (existingTermData[cid]) termData[cid] = existingTermData[cid];
        continue;
      }
      const terminals: PersistedTerminalData[] = [];
      for (let i = 0; i < termIds.length; i++) {
        const tid = termIds[i];
        const sess = sessions[tid];
        if (!sess?.workingDirectory) continue;
        let scrollback = TerminalInstanceManager.serialize(tid);
        if (scrollback.length > MAX_SCROLLBACK) scrollback = scrollback.slice(-MAX_SCROLLBACK);
        if (!scrollback && existingTermData[cid]?.[i]?.scrollback) {
          scrollback = existingTermData[cid][i].scrollback;
        }
        const customName = sess.customName || existingTermData[cid]?.[i]?.customName;
        const blocks: PersistedCommandBlock[] = [];
        const timeline = timelines[tid];
        if (timeline) {
          for (const block of timeline) {
            if (block.type === "command") {
              let output = block.data.output;
              if (output.length > MAX_BLOCK_OUTPUT) output = output.slice(-MAX_BLOCK_OUTPUT);
              blocks.push({ id: block.id, type: "command", timestamp: block.timestamp, data: { ...block.data, output } });
            }
          }
        }
        const entry: PersistedTerminalData = { workingDirectory: sess.workingDirectory, scrollback, customName };
        entry.timelineBlocks = blocks.slice(-MAX_BLOCKS);
        if (!entry.timelineBlocks.length && existingTermData[cid]?.[i]?.timelineBlocks?.length) {
          entry.timelineBlocks = existingTermData[cid][i].timelineBlocks;
        }
        terminals.push(entry);
      }
      if (terminals.length > 0) {
        termData[cid] = terminals;
      } else if (existingTermData[cid]) {
        termData[cid] = existingTermData[cid];
      }
    }
    return termData;
  };

  const save = () => {
    const name = getProjectName();
    if (!name) return;
    const { conversations, conversationOrder, activeConversationId, terminalTabs } = getState();
    const termData = buildTerminalData();

    let aiModel: { model: string; provider: string } | null = null;
    try { aiModel = JSON.parse(localStorage.getItem("golish-pentest-ai-model") || "null"); } catch { /* ignore */ }
    const approvalMode = localStorage.getItem("golish-approval-mode") || undefined;

    saveWorkspaceState(name, conversations, conversationOrder, activeConversationId, terminalTabs, termData, aiModel, approvalMode);
    saveToLocalStorageSync(termData);
  };

  const saveToLocalStorageSync = (prebuiltTermData?: Record<string, PersistedTerminalData[]>) => {
    try {
      const { conversations, conversationOrder } = getState();
      const toSave = conversationOrder
        .map((id) => conversations[id])
        .filter((c) => c && c.messages.length > 0)
        .map((c) => ({
          id: c.id,
          title: c.title,
          messages: c.messages.map((m: ChatMessage) => ({ ...m, isStreaming: false })),
          createdAt: c.createdAt,
          aiSessionId: c.aiSessionId,
          aiInitialized: false,
          isStreaming: false,
        }));
      localStorage.setItem(LOCAL_STORAGE_BACKUP_KEY, JSON.stringify(toSave));
      const termData = prebuiltTermData ?? buildTerminalData();
      localStorage.setItem("golish-pentest-conv-terminals", JSON.stringify(termData));
    } catch { /* ignore */ }
  };

  const debouncedSave = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(save, SAVE_DEBOUNCE_MS);
  };

  const unsubscribe = subscribe(debouncedSave);

  const handleBeforeUnload = () => {
    const termData = buildTerminalData();
    saveToLocalStorageSync(termData);
    save();
  };
  window.addEventListener("beforeunload", handleBeforeUnload);

  return () => {
    unsubscribe();
    window.removeEventListener("beforeunload", handleBeforeUnload);
    if (timer) clearTimeout(timer);
  };
}
