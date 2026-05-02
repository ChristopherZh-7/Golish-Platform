/**
 * Detached window management IPC wrappers.
 *
 * Use via:
 *   import { api } from "@/lib/api";
 *   await api.window.createDetached({ sessionId, tabType, title, x, y, width, height });
 *   await api.window.closeDetached(sessionId);
 *
 * Backed by `create_detached_window` / `close_detached_window` Tauri
 * commands (see `commands_facade::pentest`).
 */

import { invoke } from "./client";

export interface CreateDetachedWindowParams {
  sessionId: string;
  tabType: string;
  title: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export async function createDetached(params: CreateDetachedWindowParams): Promise<void> {
  await invoke("create_detached_window", params as unknown as Record<string, unknown>);
}

export async function closeDetached(sessionId: string): Promise<void> {
  await invoke("close_detached_window", { sessionId });
}
