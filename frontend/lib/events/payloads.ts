/**
 * Centralized event payload type definitions.
 *
 * These types describe the shape of data received from each event channel.
 * They are the single source of truth for the frontend and must match
 * the Rust structs serialized via `serde`.
 */

import type { AiEvent } from "@/lib/ai/types";

export interface TerminalOutputPayload {
  session_id: string;
  data: string;
}

export interface CommandBlockPayload {
  session_id: string;
  command: string | null;
  exit_code: number | null;
  event_type: "prompt_start" | "prompt_end" | "command_start" | "command_end";
}

export interface DirectoryChangedPayload {
  session_id: string;
  path: string;
}

export interface VirtualEnvChangedPayload {
  session_id: string;
  name: string | null;
}

export interface SessionEndedPayload {
  sessionId: string;
}

export interface AlternateScreenPayload {
  session_id: string;
  enabled: boolean;
}

/**
 * Backend emits this when the PTY has been idle long enough for the
 * `stdin_wait_detector` heuristic (300 ms, see
 * `STDIN_WAIT_IDLE_THRESHOLD_MS`) and the trailing output matches a
 * known prompt pattern (Y/N, password, "Continue?", PowerShell choice,
 * etc.). The frontend uses this to switch `UnifiedInput` into
 * interactive mode so the user's keystrokes go to the running command's
 * stdin instead of starting a new command.
 *
 * `detector` mirrors `StdinWaitKind::as_event_str` in the Rust crate
 * — see `frontend/store/types/session.ts::StdinWaitDetector` for the
 * matching frontend enum.
 */
export interface StdinWaitPayload {
  session_id: string;
  detector: string;
}

// ---------------------------------------------------------------------------
// GridTerminal (Phase B · `docs/design/2026-05-15-grid-terminal-phase-b.md`).
//
// Backend ships a `terminal_grid_update` event whenever the virtual
// terminal grid changes (16 fps coalesce window, alt-screen sessions
// only). Field shapes mirror the Rust structs in
// `backend/crates/golish-pty/src/grid/{cell,snapshot}.rs`; keep them in
// sync.
// ---------------------------------------------------------------------------

/** Cell colour. Mirrors the Rust `Color` enum (struct-variant tag). */
export type GridColor =
  | { kind: "default" }
  | { kind: "indexed"; value: number }
  | { kind: "rgb"; value: number };

/** Cursor presentation style. Mirrors the Rust `CursorStyle` enum. */
export type GridCursorStyle = "block" | "underline" | "bar";

/** Cursor position + style for the current grid frame. */
export interface GridCursorPayload {
  x: number;
  y: number;
  visible: boolean;
  style: GridCursorStyle;
}

/** Single cell on the grid. `ch === "\0"` marks the spacer slot of a
 *  CJK wide character — frontend skips those when rendering. `attrs`
 *  is a bitmask (BOLD=1, ITALIC=2, UNDERLINE=4, INVERSE=8,
 *  STRIKETHROUGH=16, DIM=32, HIDDEN=64, BLINK=128). */
export interface GridCellPayload {
  ch: string;
  fg: GridColor;
  bg: GridColor;
  attrs: number;
}

/** Cells of a single row, indexed by `y` (viewport top = 0). */
export interface GridRowPayload {
  y: number;
  cells: GridCellPayload[];
}

/**
 * Grid frame delivered to the frontend. `full = true` means
 * `dirty_rows` covers the whole viewport (used on first subscribe,
 * after resize, or after a missed `rev`). `full = false` means the
 * frontend should merge `dirty_rows` onto its cached grid. `rev` is a
 * monotonic counter — non-contiguous deliveries trigger a
 * `pty_request_grid_snapshot` call to recover.
 */
export interface TerminalGridUpdatePayload {
  session_id: string;
  rev: number;
  cols: number;
  rows: number;
  full: boolean;
  dirty_rows: GridRowPayload[];
  cursor: GridCursorPayload;
  alt_screen: boolean;
  /** Whether the terminal is in DEC `APP_CURSOR` mode (mode 1). The
   *  frontend keymap uses this to pick between `ESC [ X` (normal) and
   *  `ESC O X` (application) arrow-key encodings. */
  app_cursor_mode: boolean;
}

/**
 * File watcher event emitted when a watched workspace file changes on disk.
 * Mirrors Rust `golish::commands::fs::file_watcher::FileChangedEvent`
 * (with `#[serde(rename_all = "camelCase")]`, so `modified_at` ↔ `modifiedAt`).
 */
export interface FileChangedPayload {
  path: string;
  modifiedAt: string | null;
}

export interface McpEventPayload {
  type: "initializing" | "ready" | "error";
  message: string;
  serverCount?: number;
  toolCount?: number;
}

export type SidecarEventPayload =
  | { event_type: "session_started"; session_id: string }
  | { event_type: "session_ended"; session_id: string }
  | { event_type: "patch_created"; session_id: string; patch_id: number; subject: string }
  | { event_type: "patch_applied"; session_id: string; patch_id: number; commit_sha: string }
  | { event_type: "patch_discarded"; session_id: string; patch_id: number }
  | {
      event_type: "patch_message_updated";
      session_id: string;
      patch_id: number;
      new_subject: string;
    }
  | { event_type: "artifact_created"; session_id: string; filename: string; target: string }
  | { event_type: "artifact_applied"; session_id: string; filename: string; target: string }
  | { event_type: "artifact_discarded"; session_id: string; filename: string }
  | { event_type: "state_updated"; session_id: string; backend: string };

/**
 * Descriptor of a single pipeline step, included in the initial
 * `status === "started"` pipeline event.
 * Mirrors Rust `golish_pipeline::engine::types::PipelineStepInfo`.
 */
export interface PipelineStepInfo {
  id: string;
  tool_name: string;
  command_template: string;
}

/**
 * Parser / output-store statistics attached to a pipeline step result.
 * Mirrors Rust `golish_pipeline::parser::StoreStats`.
 */
export interface PipelineStoreStats {
  parsed_count: number;
  stored_count: number;
  new_count: number;
  skipped_count: number;
  errors: string[];
}

/**
 * Pipeline lifecycle event emitted on the `pipeline-event` channel.
 * Mirrors Rust `golish_pipeline::engine::types::PipelineEvent`; optional
 * Rust fields (`#[serde(skip_serializing_if = "Option::is_none")]`) are
 * marked optional here to match the wire format.
 */
export interface PipelineEventPayload {
  pipeline_id: string;
  run_id: string;
  step_id: string;
  step_index: number;
  total_steps: number;
  status: string;
  tool_name: string;
  message?: string;
  store_stats?: PipelineStoreStats;
  pipeline_name?: string;
  target?: string;
  all_steps?: PipelineStepInfo[];
  output?: string;
  duration_ms?: number;
  exit_code?: number | null;
}

/**
 * Emitted when a detached window (floating tab) is closed by the user.
 * Source: `frontend/components/DetachedView/DetachedView.tsx` calls
 * `emit("detached-window-closed", { session_id })`.
 */
export interface DetachedWindowClosedPayload {
  session_id: string;
}

/**
 * Map from channel name to its payload type.
 * Used by the typed listener to provide compile-time safety.
 */
export interface EventPayloadMap {
  "ai-event": AiEvent;
  terminal_output: TerminalOutputPayload;
  command_block: CommandBlockPayload;
  directory_changed: DirectoryChangedPayload;
  virtual_env_changed: VirtualEnvChangedPayload;
  session_ended: SessionEndedPayload;
  alternate_screen: AlternateScreenPayload;
  stdin_wait: StdinWaitPayload;
  terminal_grid_update: TerminalGridUpdatePayload;
  "sidecar-event": SidecarEventPayload;
  "file-changed": FileChangedPayload;
  "mcp-event": McpEventPayload;
  "pipeline-event": PipelineEventPayload;
  "detached-window-closed": DetachedWindowClosedPayload;
}

// ---------------------------------------------------------------------------
// Runtime type guards (added in QW6 · 2026-05).
//
// Only guards required (not nullable / always emitted) shape are checked.
// Optional fields are intentionally left unverified so backend can add new
// optional fields without breaking the guard.
//
// The matching `onEvent(channel, handler, guard?)` listener treats a failed
// guard as a structured warning (`logger.warn`) and silently drops the
// payload — handlers never see malformed input.
// ---------------------------------------------------------------------------

/** Type guard for `ai-event` channel payloads (`AiEvent` envelope + tagged enum). */
export function isAiEvent(v: unknown): v is AiEvent {
  if (typeof v !== "object" || v === null) return false;
  const o = v as Record<string, unknown>;
  // Envelope: session_id is always present (added by Tauri event system).
  // Tagged enum: `type` is the discriminant emitted by ts-rs.
  return typeof o.session_id === "string" && typeof o.type === "string";
}

/** Type guard for `pipeline-event` channel payloads. */
export function isPipelineEventPayload(v: unknown): v is PipelineEventPayload {
  if (typeof v !== "object" || v === null) return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o.pipeline_id === "string" &&
    typeof o.run_id === "string" &&
    typeof o.step_id === "string" &&
    typeof o.step_index === "number" &&
    typeof o.total_steps === "number" &&
    typeof o.status === "string" &&
    typeof o.tool_name === "string"
  );
}

const SIDECAR_EVENT_TYPES: ReadonlySet<string> = new Set([
  "session_started",
  "session_ended",
  "patch_created",
  "patch_applied",
  "patch_discarded",
  "patch_message_updated",
  "artifact_created",
  "artifact_applied",
  "artifact_discarded",
  "state_updated",
]);

/** Type guard for `sidecar-event` channel payloads (discriminated union). */
export function isSidecarEventPayload(v: unknown): v is SidecarEventPayload {
  if (typeof v !== "object" || v === null) return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o.event_type === "string" &&
    SIDECAR_EVENT_TYPES.has(o.event_type) &&
    typeof o.session_id === "string"
  );
}
