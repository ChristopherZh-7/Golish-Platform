/**
 * Workspace state persistence — per-project.
 *
 * Primary persistence is to PostgreSQL via createDbAutoSaver.
 * This module provides legacy workspace.json support (load/save via backend IPC)
 * and a lightweight localStorage key for last-opened project tracking.
 */

import { logger } from "@/lib/logger";
import { loadProjectWorkspace, saveProjectWorkspace } from "@/lib/projects";
import type { ChatConversation, ChatMessage } from "@/store/slices/conversation";
import type { CommandBlock } from "@/store";

const LAST_PROJECT_KEY = "golish-last-project";

export interface PersistedCommandBlock {
  id: string;
  type: "command";
  timestamp: string;
  data: CommandBlock & { source?: "manual" | "pipeline" };
}

export interface PersistedPipelineBlock {
  id: string;
  type: "pipeline_progress";
  timestamp: string;
  data: import("@/store").PipelineExecution;
}

export interface PersistedToolExecBlock {
  id: string;
  type: "ai_tool_execution";
  timestamp: string;
  data: import("@/store").AiToolExecution;
}

export interface PersistedSubAgentBlock {
  id: string;
  type: "sub_agent_activity";
  timestamp: string;
  data: import("@/store").ActiveSubAgent;
  batchId?: string;
}

export type PersistedTimelineBlock =
  | PersistedCommandBlock
  | PersistedPipelineBlock
  | PersistedToolExecBlock
  | PersistedSubAgentBlock;

export interface PersistedTerminalData {
  workingDirectory: string;
  scrollback: string;
  customName?: string;
  planJson?: unknown;
  timelineBlocks?: PersistedTimelineBlock[];
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
  try { localStorage.setItem(LAST_PROJECT_KEY, name); } catch { /* ignore */ }
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

// DB auto-saver (createDbAutoSaver) handles all persistence to PostgreSQL.
// The legacy createWorkspaceAutoSaver that wrote to localStorage has been removed.
