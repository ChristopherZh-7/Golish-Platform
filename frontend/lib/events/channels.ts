/**
 * Centralized event channel definitions.
 *
 * All Tauri event channel names live here. Both the backend (Rust) and
 * frontend (TypeScript) MUST agree on these strings. Changing a channel
 * name here requires a matching change in the Rust `emit()` call.
 */

export const EventChannels = {
  AI_EVENT: "ai-event",
  TERMINAL_OUTPUT: "terminal_output",
  COMMAND_BLOCK: "command_block",
  DIRECTORY_CHANGED: "directory_changed",
  VIRTUAL_ENV_CHANGED: "virtual_env_changed",
  SESSION_ENDED: "session_ended",
  ALTERNATE_SCREEN: "alternate_screen",
  SIDECAR_EVENT: "sidecar-event",
  FILE_CHANGED: "file-changed",
  MCP_EVENT: "mcp-event",
  PIPELINE_EVENT: "pipeline-event",
  TAB_SPLIT_EVENT: "tab-split-event",
} as const;

export type EventChannel = (typeof EventChannels)[keyof typeof EventChannels];
