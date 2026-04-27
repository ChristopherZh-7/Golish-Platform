import { invoke } from "@tauri-apps/api/core";

export interface PtySession {
  id: string;
  working_directory: string;
  rows: number;
  cols: number;
}

export async function ptyCreate(
  workingDirectory?: string,
  rows?: number,
  cols?: number
): Promise<PtySession> {
  return invoke("pty_create", {
    workingDirectory,
    rows: rows ?? 24,
    cols: cols ?? 80,
  });
}

export async function ptyWrite(sessionId: string, data: string): Promise<void> {
  return invoke("pty_write", { sessionId, data });
}

export async function ptyResize(sessionId: string, rows: number, cols: number): Promise<void> {
  return invoke("pty_resize", { sessionId, rows, cols });
}

export async function ptyDestroy(sessionId: string): Promise<void> {
  return invoke("pty_destroy", { sessionId });
}

export async function ptyGetSession(sessionId: string): Promise<PtySession> {
  return invoke("pty_get_session", { sessionId });
}

export async function ptyGetForegroundProcess(sessionId: string): Promise<string | null> {
  return invoke("pty_get_foreground_process", { sessionId });
}

export async function setActiveTerminalSession(sessionId: string): Promise<void> {
  return invoke("set_active_terminal_session", { sessionId });
}
