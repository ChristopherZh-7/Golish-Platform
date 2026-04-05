/**
 * Workspace state persistence — per-project.
 *
 * Each project stores its conversation state in
 * ~/.golish/projects/<slug>/workspace.json via backend IPC.
 *
 * A lightweight localStorage key tracks the last-opened project
 * so the app can auto-restore on restart.
 */

import { logger } from "@/lib/logger";
import { loadProjectWorkspace, saveProjectWorkspace } from "@/lib/projects";
import type { ChatConversation, ChatMessage } from "@/store/slices/conversation";

const LAST_PROJECT_KEY = "golish-last-project";
const SAVE_DEBOUNCE_MS = 1000;

/** Serializable workspace state — only what needs to survive a restart. */
export interface PersistedWorkspaceState {
  version: 1;
  savedAt: number;
  conversations: PersistedConversation[];
  conversationOrder: string[];
  activeConversationId: string | null;
}

interface PersistedConversation {
  id: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
  aiSessionId: string;
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
): Promise<void> {
  try {
    const state: PersistedWorkspaceState = {
      version: 1,
      savedAt: Date.now(),
      conversations: conversationOrder
        .map((id) => conversations[id])
        .filter(Boolean)
        .map(toPersistedConversation),
      conversationOrder,
      activeConversationId,
    };

    await saveProjectWorkspace(projectName, JSON.stringify(state));
    logger.debug("[WorkspaceStorage] Saved for project:", projectName, {
      conversations: state.conversations.length,
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

    if (state.version !== 1 || !Array.isArray(state.conversations)) {
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
export function createWorkspaceAutoSaver(
  getProjectName: () => string | null,
  subscribe: (listener: () => void) => () => void,
  getState: () => {
    conversations: Record<string, ChatConversation>;
    conversationOrder: string[];
    activeConversationId: string | null;
  },
): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;

  const save = () => {
    const name = getProjectName();
    if (!name) return;
    const { conversations, conversationOrder, activeConversationId } = getState();
    saveWorkspaceState(name, conversations, conversationOrder, activeConversationId);
  };

  const debouncedSave = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(save, SAVE_DEBOUNCE_MS);
  };

  const unsubscribe = subscribe(debouncedSave);

  const handleBeforeUnload = () => save();
  window.addEventListener("beforeunload", handleBeforeUnload);

  return () => {
    unsubscribe();
    window.removeEventListener("beforeunload", handleBeforeUnload);
    if (timer) clearTimeout(timer);
  };
}
