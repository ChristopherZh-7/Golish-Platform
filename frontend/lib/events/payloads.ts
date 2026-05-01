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

export interface FileChangedPayload {
  path: string;
  kind: "create" | "modify" | "remove";
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
  "sidecar-event": SidecarEventPayload;
  "file-changed": FileChangedPayload;
  "mcp-event": McpEventPayload;
}
