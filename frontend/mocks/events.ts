/**
 * Mock event payload types (mirroring the backend events) and the emit helper
 * functions used to simulate terminal / command / directory / session / AI
 * events in browser mode. All fan out through [`dispatchMockEvent`].
 */

import { dispatchMockEvent } from "./event-bus";

// =============================================================================
// Event Types (matching backend events)
// =============================================================================

export interface TerminalOutputEvent {
  session_id: string;
  data: string;
}

// Command block events are lifecycle events, not full blocks
export interface CommandBlockEvent {
  session_id: string;
  command: string | null;
  exit_code: number | null;
  event_type: "prompt_start" | "prompt_end" | "command_start" | "command_end";
}

export interface DirectoryChangedEvent {
  session_id: string;
  path: string;
}

export interface SessionEndedEvent {
  session_id: string;
}

export type AiEventType =
  | { type: "started"; turn_id: string }
  | { type: "text_delta"; delta: string; accumulated: string }
  | { type: "tool_request"; tool_name: string; args: unknown; request_id: string }
  | {
      type: "tool_auto_approved";
      tool_name: string;
      args: unknown;
      request_id: string;
      reason: string;
    }
  | {
      type: "tool_approval_request";
      tool_name: string;
      args: unknown;
      request_id: string;
      risk_level?: string;
    }
  | {
      type: "tool_result";
      tool_name: string;
      result: unknown;
      success: boolean;
      request_id: string;
    }
  | {
      type: "tool_output_chunk";
      tool_name: string;
      request_id: string;
      chunk: string;
      stream: string;
    }
  | {
      type: "completed";
      response: string;
      tokens_used?: number;
      duration_ms?: number;
      input_tokens?: number;
      output_tokens?: number;
    }
  | { type: "error"; message: string; error_type: string }
  | {
      type: "sub_agent_started";
      agent_id: string;
      agent_name: string;
      task: string;
      depth: number;
      parent_request_id?: string;
    }
  | {
      type: "sub_agent_text_delta";
      agent_id: string;
      delta: string;
      accumulated: string;
      parent_request_id?: string;
    }
  | {
      type: "sub_agent_tool_request";
      agent_id: string;
      tool_name: string;
      args: unknown;
      request_id: string;
      parent_request_id?: string;
    }
  | {
      type: "sub_agent_tool_result";
      agent_id: string;
      tool_name: string;
      result: unknown;
      success: boolean;
      request_id: string;
      parent_request_id?: string;
    }
  | {
      type: "sub_agent_completed";
      agent_id: string;
      response: string;
      duration_ms: number;
      parent_request_id?: string;
    }
  | { type: "sub_agent_error"; agent_id: string; error: string; parent_request_id?: string };

// =============================================================================
// Event Emitter Helpers
// =============================================================================

/**
 * Emit a terminal output event.
 * Use this to simulate terminal output in browser mode.
 */
export async function emitTerminalOutput(sessionId: string, data: string): Promise<void> {
  dispatchMockEvent("terminal_output", { session_id: sessionId, data });
}

/**
 * Emit a command block lifecycle event.
 * Use this to simulate command lifecycle events in browser mode.
 *
 * To simulate a full command execution, call in sequence:
 * 1. emitCommandBlockEvent(sessionId, "prompt_start")
 * 2. emitCommandBlockEvent(sessionId, "command_start", command)
 * 3. emitTerminalOutput(sessionId, output)  // The actual command output
 * 4. emitCommandBlockEvent(sessionId, "command_end", command, exitCode)
 * 5. emitCommandBlockEvent(sessionId, "prompt_end")
 */
export async function emitCommandBlockEvent(
  sessionId: string,
  eventType: CommandBlockEvent["event_type"],
  command: string | null = null,
  exitCode: number | null = null
): Promise<void> {
  dispatchMockEvent("command_block", {
    session_id: sessionId,
    command,
    exit_code: exitCode,
    event_type: eventType,
  });
}

/**
 * Helper to simulate a complete command execution with output.
 * This emits the proper sequence of events that the app expects.
 */
export async function simulateCommand(
  sessionId: string,
  command: string,
  output: string,
  exitCode: number = 0
): Promise<void> {
  // Start command
  await emitCommandBlockEvent(sessionId, "command_start", command);

  // Send output
  await emitTerminalOutput(sessionId, `$ ${command}\r\n`);
  await emitTerminalOutput(sessionId, output);
  if (!output.endsWith("\n")) {
    await emitTerminalOutput(sessionId, "\r\n");
  }

  // End command
  await emitCommandBlockEvent(sessionId, "command_end", command, exitCode);
}

/**
 * @deprecated Use emitCommandBlockEvent() or simulateCommand() instead.
 * This function signature doesn't match the actual event format.
 */
export async function emitCommandBlock(
  sessionId: string,
  command: string,
  output: string,
  exitCode: number | null = 0,
  _workingDirectory: string = "/home/user"
): Promise<void> {
  // Redirect to the proper simulation
  await simulateCommand(sessionId, command, output, exitCode ?? 0);
}

/**
 * Emit a directory changed event.
 * Use this to simulate directory changes in browser mode.
 */
export async function emitDirectoryChanged(sessionId: string, directory: string): Promise<void> {
  dispatchMockEvent("directory_changed", { session_id: sessionId, directory });
}

/**
 * Emit a session ended event.
 * Use this to simulate session termination in browser mode.
 */
export async function emitSessionEnded(sessionId: string): Promise<void> {
  dispatchMockEvent("session_ended", { session_id: sessionId });
}

/**
 * Emit an AI event.
 * Use this to simulate AI streaming responses in browser mode.
 */
export async function emitAiEvent(event: AiEventType): Promise<void> {
  dispatchMockEvent("ai-event", event);
}
