import type { TerminalGridUpdatePayload } from "@/lib/events/payloads";
import { invoke } from "./client";

export interface PtySession {
  id: string;
  working_directory: string;
  rows: number;
  cols: number;
}

// Default PTY size used when a caller doesn't pass rows/cols. 80×24 is the
// classic vt100 minimum, but it makes Windows PowerShell's `Format-Table`
// (used by `dir`/`Get-ChildItem`) collapse the `Mode` column into the
// preceding `Directory:` header line until ResizeObserver fires the first
// real resize. 120×30 matches the modern Windows Terminal / Console host
// defaults and is wide enough for `Format-Table` to render the full table.
const DEFAULT_PTY_ROWS = 30;
const DEFAULT_PTY_COLS = 120;

export async function ptyCreate(
  workingDirectory?: string,
  rows?: number,
  cols?: number
): Promise<PtySession> {
  return invoke<PtySession>("pty_create", {
    workingDirectory,
    rows: rows ?? DEFAULT_PTY_ROWS,
    cols: cols ?? DEFAULT_PTY_COLS,
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
  return invoke<PtySession>("pty_get_session", { sessionId });
}

export async function ptyGetForegroundProcess(sessionId: string): Promise<string | null> {
  return invoke<string | null>("pty_get_foreground_process", { sessionId });
}

export async function setActiveTerminalSession(sessionId: string): Promise<void> {
  return invoke("set_active_terminal_session", { sessionId });
}

/**
 * Phase B · GridTerminal: ask the backend for a full snapshot of the
 * virtual terminal grid. Used on first subscribe and as a recovery
 * mechanism when the frontend notices a missed `rev` in the
 * `terminal_grid_update` stream. Returns `null` when the session is not
 * currently on alt-screen (i.e. no grid is allocated).
 */
export async function ptyRequestGridSnapshot(
  sessionId: string
): Promise<TerminalGridUpdatePayload | null> {
  return invoke<TerminalGridUpdatePayload | null>("pty_request_grid_snapshot", { sessionId });
}

/**
 * Phase B · GridTerminal: resize the virtual terminal grid for the
 * given session. Targets the GridTerminal layer specifically — the
 * underlying PTY keeps its own dimensions, set via [`ptyResize`].
 * No-ops when the session has no grid allocated.
 */
export async function ptyResizeGrid(sessionId: string, cols: number, rows: number): Promise<void> {
  return invoke("pty_resize_grid", { sessionId, cols, rows });
}
