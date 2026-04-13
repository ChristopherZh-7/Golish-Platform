/**
 * Tauri IPC Mock Adapter
 *
 * This module provides mock implementations for all Tauri IPC commands and events,
 * enabling browser-only development without the Rust backend.
 *
 * Usage: This file is automatically loaded in browser environments
 * (when window.__TAURI_INTERNALS__ is undefined).
 *
 * Events can be emitted using the exported helper functions:
 * - emitTerminalOutput(sessionId, data)
 * - emitCommandBlock(block)
 * - emitDirectoryChanged(sessionId, directory)
 * - emitSessionEnded(sessionId)
 * - emitAiEvent(event)
 */

import * as tauriEvent from "@tauri-apps/api/event";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";

// =============================================================================
// Browser Mode Flag
// =============================================================================

// Re-export isMockBrowserMode from the isolated module for backwards compatibility.
// New code should import directly from "@/lib/isMockBrowser" to avoid pulling
// in this entire 1800+ line file into the bundle.
export { isMockBrowserMode } from "./lib/isMockBrowser";

// =============================================================================
// Event System (custom implementation for browser mode)
// =============================================================================

// Auto-incrementing handler ID
let nextHandlerId = 1;

// Map of event name -> array of { handlerId, callback }
const mockEventListeners: Map<
  string,
  Array<{ handlerId: number; callback: (event: { event: string; payload: unknown }) => void }>
> = new Map();

// Map of handler ID -> { event, callback } (for unlisten)
const handlerToEvent: Map<number, string> = new Map();

/**
 * Register an event listener with its callback
 */
export function mockRegisterListener(
  event: string,
  callback: (event: { event: string; payload: unknown }) => void
): number {
  const handlerId = nextHandlerId++;
  if (!mockEventListeners.has(event)) {
    mockEventListeners.set(event, []);
  }
  mockEventListeners.get(event)?.push({ handlerId, callback });
  handlerToEvent.set(handlerId, event);
  console.log(`[Mock Events] Registered listener for "${event}" (handler: ${handlerId})`);
  return handlerId;
}

/**
 * Unregister an event listener by handler ID
 */
export function mockUnregisterListener(handlerId: number): void {
  const eventName = handlerToEvent.get(handlerId);
  if (!eventName) return;

  handlerToEvent.delete(handlerId);
  const listeners = mockEventListeners.get(eventName);
  if (listeners) {
    const filtered = listeners.filter((l) => l.handlerId !== handlerId);
    mockEventListeners.set(eventName, filtered);
    console.log(`[Mock Events] Unregistered listener for "${eventName}" (handler: ${handlerId})`);
  }
}

/**
 * Dispatch an event to all registered listeners
 */
function dispatchMockEvent(eventName: string, payload: unknown): void {
  const listeners = mockEventListeners.get(eventName);
  if (listeners && listeners.length > 0) {
    console.log(
      `[Mock Events] Dispatching "${eventName}" to ${listeners.length} listener(s)`,
      payload
    );
    for (const { callback } of listeners) {
      try {
        callback({ event: eventName, payload });
      } catch (e) {
        console.error(`[Mock Events] Error in listener for "${eventName}":`, e);
      }
    }
  } else {
    console.log(`[Mock Events] No listeners for "${eventName}"`, payload);
  }
}

// =============================================================================
// Mock Data
// =============================================================================

// Mock PTY sessions
// Keep the first session id stable for MockDevTools presets.
let mockPtySessionCounter = 1;
const mockPtySessions: Record<
  string,
  { id: string; working_directory: string; rows: number; cols: number }
> = {
  "mock-session-001": {
    id: "mock-session-001",
    working_directory: "/home/user",
    rows: 24,
    cols: 80,
  },
};

// Mock AI state
let mockAiInitialized = false;
let mockConversationLength = 0;
let mockSessionPersistenceEnabled = true;

// Mock Git state (used by get_git_branch and git_status)
interface MockGitStatusSummary {
  branch: string | null;
  ahead: number;
  behind: number;
  entries: Array<unknown>;
  insertions: number;
  deletions: number;
}

let mockGitBranch: string | null = "main";
let mockGitStatus: MockGitStatusSummary = {
  branch: mockGitBranch,
  ahead: 0,
  behind: 0,
  entries: [],
  insertions: 0,
  deletions: 0,
};

export function setMockGitState(next: Partial<MockGitStatusSummary>): void {
  if ("branch" in next) {
    mockGitBranch = next.branch ?? null;
  }

  mockGitStatus = {
    ...mockGitStatus,
    ...next,
    branch: mockGitBranch,
  };
}

// Session-specific AI state (for per-tab isolation)
const mockSessionAiState: Map<
  string,
  { initialized: boolean; conversationLength: number; config?: unknown }
> = new Map();

// =============================================================================
// Parameter Validation Helper
// =============================================================================

/**
 * Validates that required parameters are present in the args object.
 * Throws an error (like Tauri would) if a required parameter is missing.
 *
 * @param cmd - The command name (for error messages)
 * @param args - The arguments object passed to the command
 * @param requiredParams - List of required parameter names (in camelCase, as sent from JS)
 */
function validateRequiredParams(cmd: string, args: unknown, requiredParams: string[]): void {
  const argsObj = args as Record<string, unknown> | undefined;

  for (const param of requiredParams) {
    if (!argsObj || !(param in argsObj) || argsObj[param] === undefined) {
      const error = `invalid args \`${param}\` for command \`${cmd}\`: command ${cmd} missing required key ${param}`;
      console.error(`[Mock IPC] ${error}`);
      throw new Error(error);
    }
  }
}

// Mock tool definitions
const mockTools = [
  {
    name: "read_file",
    description: "Read the contents of a file",
    parameters: {
      type: "object",
      properties: {
        path: { type: "string", description: "Path to the file" },
      },
      required: ["path"],
    },
  },
  {
    name: "write_file",
    description: "Write content to a file",
    parameters: {
      type: "object",
      properties: {
        path: { type: "string", description: "Path to the file" },
        content: { type: "string", description: "Content to write" },
      },
      required: ["path", "content"],
    },
  },
  {
    name: "run_command",
    description: "Execute a shell command",
    parameters: {
      type: "object",
      properties: {
        command: { type: "string", description: "Command to execute" },
      },
      required: ["command"],
    },
  },
];

// Mock workflows
const mockWorkflows = [
  { name: "code-review", description: "Review code changes and provide feedback" },
  { name: "test-generation", description: "Generate unit tests for code" },
  { name: "refactor", description: "Suggest code refactoring improvements" },
];

// Mock sub-agents
const mockSubAgents = [
  { id: "explorer", name: "Code Explorer", description: "Explores and understands codebases" },
  { id: "debugger", name: "Debug Assistant", description: "Helps debug issues" },
  { id: "documenter", name: "Documentation Writer", description: "Generates documentation" },
  { id: "js_harvester", name: "JS Harvester", description: "AI-driven comprehensive JavaScript collection from target URLs" },
  { id: "js_analyzer", name: "JS Analyzer", description: "Read-only security analysis of collected JavaScript assets" },
];

// Mock sessions
const mockSessions = [
  {
    identifier: "session-2024-01-15-001",
    path: "/home/user/.golish/sessions/session-2024-01-15-001.json",
    workspace_label: "golish",
    workspace_path: "/home/user/golish",
    model: "claude-opus-4.5",
    provider: "anthropic_vertex",
    started_at: "2024-01-15T10:00:00Z",
    ended_at: "2024-01-15T11:30:00Z",
    total_messages: 24,
    distinct_tools: ["read_file", "write_file", "run_command"],
    first_prompt_preview: "Can you help me refactor the authentication module?",
    first_reply_preview: "I'll help you refactor the authentication module...",
  },
  {
    identifier: "session-2024-01-14-002",
    path: "/home/user/.golish/sessions/session-2024-01-14-002.json",
    workspace_label: "golish",
    workspace_path: "/home/user/golish",
    model: "claude-opus-4.5",
    provider: "anthropic_vertex",
    started_at: "2024-01-14T14:00:00Z",
    ended_at: "2024-01-14T16:45:00Z",
    total_messages: 42,
    distinct_tools: ["read_file", "run_command"],
    first_prompt_preview: "Help me add unit tests for the PTY manager",
    first_reply_preview: "I'll help you add unit tests for the PTY manager...",
  },
];

// Mock approval patterns
const mockApprovalPatterns = [
  {
    tool_name: "read_file",
    total_requests: 50,
    approvals: 50,
    denials: 0,
    always_allow: true,
    last_updated: "2024-01-15T10:00:00Z",
    justifications: [],
  },
  {
    tool_name: "write_file",
    total_requests: 20,
    approvals: 18,
    denials: 2,
    always_allow: false,
    last_updated: "2024-01-15T09:30:00Z",
    justifications: ["Writing config file", "Updating source code"],
  },
  {
    tool_name: "run_command",
    total_requests: 30,
    approvals: 25,
    denials: 5,
    always_allow: false,
    last_updated: "2024-01-15T11:00:00Z",
    justifications: ["Running tests", "Building project"],
  },
];

// Mock HITL config
let mockHitlConfig = {
  always_allow: ["read_file"],
  always_require_approval: ["run_command"],
  pattern_learning_enabled: true,
  min_approvals: 3,
  approval_threshold: 0.8,
};

// Mock prompts
const mockPrompts = [
  { name: "review", path: "/home/user/.golish/prompts/review.md", source: "global" as const },
  { name: "explain", path: "/home/user/.golish/prompts/explain.md", source: "global" as const },
  { name: "project-context", path: ".golish/prompts/project-context.md", source: "local" as const },
];

// Mock skills
const mockSkills = [
  {
    name: "code-review",
    path: "/home/user/.golish/skills/code-review",
    source: "global",
    description: "Review code for quality and best practices",
    license: undefined,
    compatibility: undefined,
    metadata: undefined,
    allowed_tools: ["read_file", "glob", "grep"],
    has_scripts: false,
    has_references: false,
    has_assets: false,
  },
  {
    name: "refactor",
    path: "/home/user/.golish/skills/refactor",
    source: "global",
    description: "Refactor code for improved readability and maintainability",
    license: undefined,
    compatibility: undefined,
    metadata: undefined,
    allowed_tools: ["read_file", "write_file", "glob"],
    has_scripts: false,
    has_references: false,
    has_assets: false,
  },
];

// Mock indexer state
let mockIndexerInitialized = false;
let mockIndexerWorkspace: string | null = null;
let mockIndexedFileCount = 0;

// Mock codebases state
interface MockCodebase {
  path: string;
  file_count: number;
  status: "synced" | "indexing" | "not_indexed" | "error";
  error?: string;
  memory_file?: string;
}

let mockCodebases: MockCodebase[] = [
  {
    path: "/home/user/projects/my-app",
    file_count: 150,
    status: "synced",
    memory_file: "CLAUDE.md",
  },
  {
    path: "/home/user/projects/backend-api",
    file_count: 89,
    status: "synced",
    memory_file: "AGENTS.md",
  },
];

// Mock settings state
// Mock per-project settings (stored in .golish/project.toml)
const mockProjectSettings: {
  provider: string | null;
  model: string | null;
  agent_mode: string | null;
} = {
  provider: null,
  model: null,
  agent_mode: null,
};

let mockSettings = {
  version: 1,
  ai: {
    default_provider: "vertex_ai",
    default_model: "claude-opus-4-5@20251101",
    default_reasoning_effort: undefined,
    sub_agent_models: {},
    vertex_ai: {
      credentials_path: "/mock/path/to/credentials.json",
      project_id: "mock-project-id",
      location: "us-east5",
      show_in_selector: true,
    },
    openrouter: {
      api_key: "mock-openrouter-key",
      show_in_selector: true,
    },
    anthropic: {
      api_key: null,
      show_in_selector: true,
    },
    openai: {
      api_key: "mock-openai-key",
      base_url: null,
      show_in_selector: true,
    },
    ollama: {
      base_url: "http://localhost:11434",
      show_in_selector: true,
    },
    gemini: {
      api_key: null,
      show_in_selector: true,
    },
    groq: {
      api_key: null,
      show_in_selector: true,
    },
    xai: {
      api_key: null,
      show_in_selector: true,
    },
    zai_sdk: {
      api_key: null,
      base_url: null,
      model: null,
      show_in_selector: true,
    },
    nvidia: {
      api_key: null,
      base_url: null,
      show_in_selector: true,
    },
  },
  api_keys: {
    tavily: null,
    github: null,
  },
  ui: {
    theme: "dark",
    show_tips: true,
    hide_banner: false,
  },
  terminal: {
    shell: null,
    font_family: "SF Mono",
    font_size: 14,
    scrollback: 10000,
  },
  agent: {
    session_persistence: true,
    session_retention_days: 30,
    pattern_learning: true,
    min_approvals_for_auto: 3,
    approval_threshold: 0.8,
  },
  mcp_servers: {},
  trust: {
    full_trust: [],
    read_only_trust: [],
    never_trust: [],
  },
  privacy: {
    usage_statistics: false,
    log_prompts: false,
  },
  advanced: {
    enable_experimental: false,
    log_level: "info",
  },
  sidecar: {
    enabled: true,
    synthesis_enabled: true,
    synthesis_backend: "template",
    synthesis_vertex: {
      project_id: null,
      location: null,
      model: "claude-sonnet-4-5-20250514",
      credentials_path: null,
    },
    synthesis_openai: {
      api_key: null,
      model: "gpt-4o-mini",
      base_url: null,
    },
    synthesis_grok: {
      api_key: null,
      model: "grok-2",
    },
    retention_days: 30,
    capture_tool_calls: true,
    capture_reasoning: true,
  },
  network: {
    proxy_url: null,
    no_proxy: null,
  },
};

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
  | { type: "tool_auto_approved"; tool_name: string; args: unknown; request_id: string; reason: string }
  | { type: "tool_approval_request"; tool_name: string; args: unknown; request_id: string; risk_level?: string }
  | {
      type: "tool_result";
      tool_name: string;
      result: unknown;
      success: boolean;
      request_id: string;
    }
  | { type: "tool_output_chunk"; tool_name: string; request_id: string; chunk: string; stream: string }
  | {
      type: "completed";
      response: string;
      tokens_used?: number;
      duration_ms?: number;
      input_tokens?: number;
      output_tokens?: number;
    }
  | { type: "error"; message: string; error_type: string }
  | { type: "sub_agent_started"; agent_id: string; agent_name: string; task: string; depth: number; parent_request_id?: string }
  | { type: "sub_agent_text_delta"; agent_id: string; delta: string; accumulated: string; parent_request_id?: string }
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
  | { type: "sub_agent_completed"; agent_id: string; response: string; duration_ms: number; parent_request_id?: string }
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

/**
 * Simulate a complete AI response with streaming.
 * This emits started -> text_delta(s) -> completed events.
 */
export async function simulateAiResponse(response: string, delayMs: number = 50): Promise<void> {
  const turnId = `mock-turn-${Date.now()}`;

  // Emit started
  await emitAiEvent({ type: "started", turn_id: turnId });

  // Emit text deltas (word by word)
  const words = response.split(" ");
  let accumulated = "";
  for (const word of words) {
    const delta = accumulated ? ` ${word}` : word;
    accumulated += delta;
    await emitAiEvent({ type: "text_delta", delta, accumulated });
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  }

  // Emit completed
  await emitAiEvent({
    type: "completed",
    response: accumulated,
    tokens_used: Math.floor(accumulated.length / 4),
    duration_ms: words.length * delayMs,
  });
}

/**
 * Simulate a sub-agent execution with tool calls.
 * This emits the proper sequence of sub-agent events.
 */
export async function simulateSubAgent(
  agentId: string,
  agentName: string,
  task: string,
  toolCalls: Array<{ name: string; args: unknown; result: unknown }>,
  response: string,
  delayMs: number = 20
): Promise<void> {
  // Emit sub-agent started
  await emitAiEvent({
    type: "sub_agent_started",
    agent_id: agentId,
    agent_name: agentName,
    task,
    depth: 1,
  });
  await new Promise((resolve) => setTimeout(resolve, delayMs));

  // Emit tool calls
  for (const tool of toolCalls) {
    const requestId = `mock-req-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;

    await emitAiEvent({
      type: "sub_agent_tool_request",
      agent_id: agentId,
      tool_name: tool.name,
      args: tool.args,
      request_id: requestId,
    });
    await new Promise((resolve) => setTimeout(resolve, delayMs));

    await emitAiEvent({
      type: "sub_agent_tool_result",
      agent_id: agentId,
      tool_name: tool.name,
      result: tool.result,
      success: true,
      request_id: requestId,
    });
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  }

  // Emit sub-agent completed
  await emitAiEvent({
    type: "sub_agent_completed",
    agent_id: agentId,
    response,
    duration_ms: toolCalls.length * delayMs * 2 + 100,
  });
}

/**
 * Simulate an AI response that spawns a sub-agent.
 * This demonstrates the proper interleaving of sub-agent tool calls in the timeline.
 */
export async function simulateAiResponseWithSubAgent(
  subAgentName: string,
  subAgentTask: string,
  subAgentResponse: string,
  finalResponse: string,
  delayMs: number = 20
): Promise<void> {
  const turnId = `mock-turn-${Date.now()}`;
  const agentId = `mock-agent-${Date.now()}`;
  const subAgentToolRequestId = `mock-sub-req-${Date.now()}`;

  // Emit turn started
  await emitAiEvent({ type: "started", turn_id: turnId });
  await new Promise((resolve) => setTimeout(resolve, delayMs));

  // Emit sub-agent tool call (this creates the tool block in streamingBlocks)
  await emitAiEvent({
    type: "tool_request",
    tool_name: `sub_agent_${subAgentName.toLowerCase().replace(/\s+/g, "_")}`,
    args: { task: subAgentTask },
    request_id: subAgentToolRequestId,
  });
  await new Promise((resolve) => setTimeout(resolve, delayMs));

  // Emit sub-agent started (this populates activeSubAgents)
  await emitAiEvent({
    type: "sub_agent_started",
    agent_id: agentId,
    agent_name: subAgentName,
    task: subAgentTask,
    depth: 1,
  });
  await new Promise((resolve) => setTimeout(resolve, delayMs));

  // Emit some sub-agent tool calls
  const subToolReqId = `mock-sub-tool-${Date.now()}`;
  await emitAiEvent({
    type: "sub_agent_tool_request",
    agent_id: agentId,
    tool_name: "list_files",
    args: { path: "." },
    request_id: subToolReqId,
  });
  await new Promise((resolve) => setTimeout(resolve, delayMs));

  await emitAiEvent({
    type: "sub_agent_tool_result",
    agent_id: agentId,
    tool_name: "list_files",
    result: ["file1.ts", "file2.ts"],
    success: true,
    request_id: subToolReqId,
  });
  await new Promise((resolve) => setTimeout(resolve, delayMs));

  // Emit sub-agent completed
  await emitAiEvent({
    type: "sub_agent_completed",
    agent_id: agentId,
    response: subAgentResponse,
    duration_ms: 5000,
  });
  await new Promise((resolve) => setTimeout(resolve, delayMs));

  // Emit sub-agent tool result (marks the tool call as completed)
  await emitAiEvent({
    type: "tool_result",
    tool_name: `sub_agent_${subAgentName.toLowerCase().replace(/\s+/g, "_")}`,
    result: subAgentResponse,
    success: true,
    request_id: subAgentToolRequestId,
  });
  await new Promise((resolve) => setTimeout(resolve, delayMs));

  // Emit final text response
  const words = finalResponse.split(" ");
  let accumulated = "";
  for (const word of words) {
    const delta = accumulated ? ` ${word}` : word;
    accumulated += delta;
    await emitAiEvent({ type: "text_delta", delta, accumulated });
    await new Promise((resolve) => setTimeout(resolve, delayMs / 2));
  }

  // Emit completed
  await emitAiEvent({
    type: "completed",
    response: accumulated,
    tokens_used: Math.floor(accumulated.length / 4),
    duration_ms: 6000,
    input_tokens: 100,
    output_tokens: 50,
  });
}

/**
 * Simulate the JS Harvester + Analyzer sub-agent flow.
 * Triggered automatically when user sends a message containing "js" or "analyze".
 */
export async function simulateJsHarvest(): Promise<void> {
  const turnId = `mock-turn-${Date.now()}`;
  const harvesterId = `mock-harvester-${Date.now()}`;
  const analyzerId = `mock-analyzer-${Date.now()}`;
  const harvesterReqId = `mock-sub-req-harvest-${Date.now()}`;
  const analyzerReqId = `mock-sub-req-analyze-${Date.now()}`;
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const emitSubAgentTools = async (agentId: string, tools: { name: string; args: Record<string, unknown>; result: string }[]) => {
    for (const tool of tools) {
      const reqId = `mock-req-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      await emitAiEvent({ type: "sub_agent_tool_request", agent_id: agentId, tool_name: tool.name, args: tool.args, request_id: reqId });
      await delay(600);
      await emitAiEvent({ type: "sub_agent_tool_result", agent_id: agentId, tool_name: tool.name, result: tool.result, success: true, request_id: reqId });
      await delay(300);
    }
  };

  await emitAiEvent({ type: "started", turn_id: turnId });
  await delay(200);

  await emitAiEvent({ type: "text_delta", delta: "I'll collect all JavaScript files from example.com and then analyze them for security issues.\n\n", accumulated: "I'll collect all JavaScript files from example.com and then analyze them for security issues.\n\n" });
  await delay(300);

  // === Phase 1: JS Harvester ===
  await emitAiEvent({ type: "tool_request", tool_name: "sub_agent_js_harvester", args: { task: "Collect ALL JS files from https://example.com" }, request_id: harvesterReqId });
  await delay(200);
  await emitAiEvent({ type: "sub_agent_started", agent_id: harvesterId, agent_name: "JS Harvester", task: "Collect ALL JS files from https://example.com", depth: 1, parent_request_id: harvesterReqId });
  await delay(300);
  await emitAiEvent({ type: "sub_agent_text_delta", agent_id: harvesterId, delta: "Probing target for bundler type...", accumulated: "Probing target for bundler type...", parent_request_id: harvesterReqId });
  await delay(200);

  await emitSubAgentTools(harvesterId, [
    { name: "run_pty_cmd", args: { command: "curl -sLk -D- https://example.com" }, result: "HTTP/2 200\nserver: nginx\ncontent-type: text/html\n\n<!DOCTYPE html>...<script type=\"module\" src=\"/assets/index-BjK2xfA.js\">" },
    { name: "run_pty_cmd", args: { command: "curl -sLk -w '%{http_code}' -o /dev/null https://example.com/.vite/manifest.json" }, result: "404" },
    { name: "run_pty_cmd", args: { command: "curl -sLk -w '%{http_code}' -o /dev/null https://example.com/asset-manifest.json" }, result: "404" },
    { name: "run_pty_cmd", args: { command: "curl -sLk https://example.com/assets/index-BjK2xfA.js | head -5" }, result: "import{c as createApp}from\"./chunk-framework-De4f.js\";const routes=[...]" },
    { name: "write_file", args: { path: "/tmp/js_harvest.sh", content: "#!/bin/bash\nBASE=https://example.com/assets ..." }, result: "File written: /tmp/js_harvest.sh" },
    { name: "run_pty_cmd", args: { command: "bash /tmp/js_harvest.sh" }, result: "Downloading index-BjK2xfA.js... OK\nDownloading vendor-Ca3dR2p.js... OK\n... (50 files from main bundle)\nRecursive pass 1: found 5 new refs in system.js\nRecursive pass 2: found 3 new refs in project.js\nRecursive pass 3: 0 new files\nTOTAL: 58 files downloaded (2.8MB)" },
    { name: "run_pty_cmd", args: { command: "for f in .golish/js-assets/example.com/*.js; do curl -sLk -w '%{http_code}' -o \"${f}.map\" \"https://example.com/assets/$(basename $f).map\"; done | grep 200 | wc -l" }, result: "6 source maps found" },
    { name: "write_file", args: { path: ".golish/js-assets/example.com/index.json", content: "{...manifest...}" }, result: "Manifest updated: 58 files, 6 sourcemaps, 2 failed (auth_required)" },
  ]);

  const harvesterResponse = "Collection complete: 58 JS files (2.8MB) + 6 source maps. Strategy: recursive script (no manifest found). 2 files require authentication.";
  await emitAiEvent({ type: "sub_agent_completed", agent_id: harvesterId, response: harvesterResponse, duration_ms: 8500, parent_request_id: harvesterReqId });
  await delay(200);
  await emitAiEvent({ type: "tool_result", tool_name: "sub_agent_js_harvester", result: harvesterResponse, success: true, request_id: harvesterReqId });
  await delay(400);

  // === Phase 2: JS Analyzer ===
  await emitAiEvent({ type: "tool_request", tool_name: "sub_agent_js_analyzer", args: { task: "Analyze collected JS in .golish/js-assets/example.com/ for security issues" }, request_id: analyzerReqId });
  await delay(200);
  await emitAiEvent({ type: "sub_agent_started", agent_id: analyzerId, agent_name: "JS Analyzer", task: "Security analysis of 58 collected JS files", depth: 1, parent_request_id: analyzerReqId });
  await delay(300);
  await emitAiEvent({ type: "sub_agent_text_delta", agent_id: analyzerId, delta: "Scanning for API endpoints and secrets...", accumulated: "Scanning for API endpoints and secrets...", parent_request_id: analyzerReqId });
  await delay(200);

  await emitSubAgentTools(analyzerId, [
    { name: "read_file", args: { path: ".golish/js-assets/example.com/index.json" }, result: '{"bundler":"vite","stats":{"total_files":58,"source_maps":6}}' },
    { name: "grep_file", args: { pattern: "/api/v[0-9]", path: ".golish/js-assets/example.com/" }, result: "5 API endpoints found across 4 files" },
    { name: "grep_file", args: { pattern: "(api_key|secret|token|password|pk_live)", path: ".golish/js-assets/example.com/" }, result: "3 secrets found in vendor-Ca3dR2p.js and index-BjK2xfA.js" },
    { name: "read_file", args: { path: ".golish/js-assets/example.com/Debug-Qr9s.js" }, result: "window.__DEBUG__={env:process.env,dump:()=>...} // no auth check" },
  ]);

  const analyzerResponse = `**API Endpoints**: 5 found (POST /auth/login, GET /users/me, POST /payments/charge, DELETE /admin/users/:id, GET /config)
**Secrets**: STRIPE_PK, API_BASE (internal URL), AWS_REGION
**Hidden Routes**: /debug (NO AUTH — env dump), /admin (DELETE endpoint)
**Vulnerable**: lodash@4.17.15, axios@0.21.0`;

  await emitAiEvent({ type: "sub_agent_completed", agent_id: analyzerId, response: analyzerResponse, duration_ms: 6200, parent_request_id: analyzerReqId });
  await delay(200);
  await emitAiEvent({ type: "tool_result", tool_name: "sub_agent_js_analyzer", result: analyzerResponse, success: true, request_id: analyzerReqId });
  await delay(400);

  // === Final summary in main chat ===
  const finalResponse = "JS 收集与分析完成。\n\n**收集**: 58 个文件 (2.8MB) + 6 个 source maps, Vite bundler\n**发现**:\n1. 5 个 API endpoints (含支付、管理员删除接口)\n2. 3 个硬编码密钥 (Stripe, 内网 API, AWS)\n3. /debug 路由无认证 — 可直接访问环境变量\n4. 2 个已知漏洞依赖\n\n优先处理: /debug 环境变量泄露 + Stripe 密钥硬编码。";
  const words = finalResponse.split(" ");
  let accumulated = "";
  for (const word of words) {
    const delta = accumulated ? ` ${word}` : word;
    accumulated += delta;
    await emitAiEvent({ type: "text_delta", delta, accumulated });
    await delay(30);
  }

  await emitAiEvent({ type: "completed", response: accumulated, tokens_used: 2400, duration_ms: 18000, input_tokens: 4800, output_tokens: 520 });
}

// =============================================================================
// Timeline Block Showcase — one mock per UnifiedBlock type
// Call from console: __mockShowAllBlocks()
// =============================================================================

/**
 * 1/4 — Command Block
 * Injects a static CommandBlock into the timeline.
 */
export async function mockCommandBlock(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];
  if (!sessionId) { console.error("[mockCommandBlock] No active session"); return; }

  useStore.setState((s) => {
    if (!s.timelines[sessionId]) s.timelines[sessionId] = [];
    s.timelines[sessionId].push({
      id: `mock-cmd-${Date.now()}`,
      type: "command",
      timestamp: new Date().toISOString(),
      data: {
        id: `mock-cmd-${Date.now()}`,
        sessionId,
        command: "nmap -sV -sC --top-ports 1000 example.com",
        output: [
          "Starting Nmap 7.94 ( https://nmap.org ) at 2026-04-11 10:00 CST",
          "Nmap scan report for example.com (93.184.216.34)",
          "Host is up (0.12s latency).",
          "Not shown: 997 filtered tcp ports (no-response)",
          "PORT    STATE SERVICE  VERSION",
          "80/tcp  open  http     nginx 1.21.6",
          "443/tcp open  ssl/http nginx 1.21.6",
          "8080/tcp open  http-proxy",
          "",
          "Service detection performed. Please provide correct ports.",
          "Nmap done: 1 IP address (1 host up) scanned in 42.31 seconds",
        ].join("\n"),
        exitCode: 0,
        startTime: new Date(Date.now() - 42310).toISOString(),
        durationMs: 42310,
        workingDirectory: "/home/user/projects",
        isCollapsed: false,
      },
    });
  });
  console.log("[mockCommandBlock] Injected command block");
}

/**
 * 2/4 — Pipeline Progress Block (with nested sub-agents in AI step)
 * Injects a PipelineProgressBlock where the JS Harvest AI step
 * has sub-agents (JS Harvester + JS Analyzer) embedded inside it.
 */
export async function mockPipelineProgressBlock(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];
  if (!sessionId) { console.error("[mockPipelineBlock] No active session"); return; }

  const now = new Date().toISOString();

  useStore.getState().startPipelineExecution(sessionId, {
    pipelineId: "recon-basic-demo",
    pipelineName: "Basic Reconnaissance",
    target: "target.example.com",
    steps: [
      {
        stepId: "dns", name: "DNS Lookup", command: "dig +short target.example.com",
        status: "success", output: "93.184.216.34\n2606:2800:220:1:248:1893:25c8:1946",
        exitCode: 0, startedAt: now, durationMs: 820,
      },
      {
        stepId: "subfinder", name: "Subdomain Enum", command: "subfinder -d target.example.com -silent",
        status: "success",
        output: "api.target.example.com\nstaging.target.example.com\ndev.target.example.com\nadmin.target.example.com",
        exitCode: 0, startedAt: now, durationMs: 5200,
        discoveredTargets: ["api.target.example.com", "staging.target.example.com", "dev.target.example.com", "admin.target.example.com"],
      },
      {
        stepId: "httpx", name: "HTTP Probe", command: "httpx -l subdomains.txt -sc -title -tech-detect -silent",
        status: "success",
        output: "https://api.target.example.com [200] [API Gateway] [Nginx,Express]\nhttps://staging.target.example.com [403] [Forbidden]\nhttps://admin.target.example.com [200] [Admin Panel] [React,Nginx]",
        exitCode: 0, startedAt: now, durationMs: 3100,
        subTargets: [
          { target: "api.target.example.com", status: "success", output: "[200] API Gateway", durationMs: 800 },
          { target: "staging.target.example.com", status: "success", output: "[403] Forbidden", durationMs: 600 },
          { target: "dev.target.example.com", status: "failed", output: "Connection refused", exitCode: 1, durationMs: 3000 },
          { target: "admin.target.example.com", status: "success", output: "[200] Admin Panel", durationMs: 700 },
        ],
      },
      {
        stepId: "nmap", name: "Port Scan", command: "nmap -sV --top-ports 1000 {target}",
        status: "success", exitCode: 0, startedAt: now, durationMs: 12400,
        output: "PORT    STATE SERVICE\n80/tcp  open  http\n443/tcp open  https\n8080/tcp open  http-proxy",
      },
      {
        stepId: "whatweb", name: "Tech Fingerprint", command: "whatweb {target} --color=never",
        status: "success", exitCode: 0, startedAt: now, durationMs: 2100,
        output: "https://target.example.com [200 OK] Nginx[1.21.6], React",
      },
      {
        stepId: "js_harvest", name: "JS Harvest (AI)", command: "AI: js_harvest {target}",
        status: "running", startedAt: now,
        subAgents: [
          {
            agentId: "js_harvester_001",
            agentName: "JS Harvester",
            parentRequestId: `mock-pipeline-harvester-${Date.now()}`,
            task: "Collect ALL JS files from https://admin.target.example.com",
            depth: 1,
            status: "completed",
            toolCalls: [
              { id: "tc1", name: "run_pty_cmd", args: { command: "curl -sL https://admin.target.example.com/" }, status: "completed", result: "<html>...</html>", startedAt: now, completedAt: now },
              { id: "tc2", name: "write_file", args: { path: ".golish/js-assets/manifest.json" }, status: "completed", result: "Written 12KB", startedAt: now, completedAt: now },
              { id: "tc3", name: "run_pty_cmd", args: { command: "bash collect.sh https://admin.target.example.com/assets" }, status: "completed", result: "TOTAL: 42 files collected (1.8MB)", startedAt: now, completedAt: now },
            ],
            entries: [
              { kind: "text", text: "Starting JS collection from target..." },
              { kind: "tool_call", toolCallId: "tc1" },
              { kind: "text", text: "Found Vite manifest, extracting asset list..." },
              { kind: "tool_call", toolCallId: "tc2" },
              { kind: "tool_call", toolCallId: "tc3" },
            ],
            response: "Collection complete: 42 JS files (1.8MB) + 3 source maps. Strategy: manifest-based (Vite detected).",
            startedAt: new Date(Date.now() - 8500).toISOString(),
            completedAt: now,
            durationMs: 8500,
          },
          {
            agentId: "js_analyzer_001",
            agentName: "JS Analyzer",
            parentRequestId: `mock-pipeline-analyzer-${Date.now()}`,
            task: "Security analysis of 42 collected JS files from admin.target.example.com",
            depth: 1,
            status: "running",
            toolCalls: [
              { id: "tc4", name: "list_files", args: { pattern: ".golish/js-assets/**/*.js" }, status: "completed", result: "42 files found", startedAt: now, completedAt: now },
              { id: "tc5", name: "grep_file", args: { pattern: "api[_-]?key|secret|token", path: ".golish/js-assets/" }, status: "running", startedAt: now },
            ],
            entries: [
              { kind: "text", text: "Listing all collected JS files..." },
              { kind: "tool_call", toolCallId: "tc4" },
              { kind: "text", text: "Scanning for API endpoints and hardcoded secrets across 42 files..." },
              { kind: "tool_call", toolCallId: "tc5" },
            ],
            streamingText: "Scanning for API endpoints and hardcoded secrets across 42 files...",
            startedAt: new Date(Date.now() - 3200).toISOString(),
          },
        ],
      },
    ],
    status: "running",
    startedAt: now,
  });
  console.log("[mockPipelineBlock] Injected pipeline block with nested sub-agents in JS Harvest step");
}

/**
 * 3/4 — Sub-Agent Activity Block (group of 2)
 * Injects two SubAgent cards that appear as a grouped SubAgentGroup.
 */
export async function mockSubAgentBlocks(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];
  if (!sessionId) { console.error("[mockSubAgentBlocks] No active session"); return; }

  const now = new Date().toISOString();
  const batchId = `batch-mock-${Date.now()}`;

  useStore.setState((s) => {
    if (!s.timelines[sessionId]) s.timelines[sessionId] = [];

    // Sub-agent 1: JS Harvester (completed)
    s.timelines[sessionId].push({
      id: `mock-sa-harvester-${Date.now()}`,
      type: "sub_agent_activity",
      timestamp: now,
      batchId,
      data: {
        agentId: "js_harvester_001",
        agentName: "JS Harvester",
        parentRequestId: `mock-parent-req-${Date.now()}`,
        task: "Collect ALL JS files from https://admin.target.example.com",
        depth: 1,
        status: "completed",
        toolCalls: [
          { id: "tc1", name: "run_pty_cmd", args: { command: "curl -sL https://admin.target.example.com/" }, status: "completed", result: "<html>...</html>", startedAt: now, completedAt: now },
          { id: "tc2", name: "write_file", args: { path: ".golish/js-assets/manifest.json" }, status: "completed", result: "Written 12KB", startedAt: now, completedAt: now },
          { id: "tc3", name: "run_pty_cmd", args: { command: "bash collect.sh https://admin.target.example.com/assets" }, status: "completed", result: "TOTAL: 42 files collected (1.8MB)", startedAt: now, completedAt: now },
        ],
        entries: [
          { kind: "text", text: "Starting JS collection from target..." },
          { kind: "tool_call", toolCallId: "tc1" },
          { kind: "text", text: "Found Vite manifest, extracting asset list..." },
          { kind: "tool_call", toolCallId: "tc2" },
          { kind: "tool_call", toolCallId: "tc3" },
        ],
        response: "Collection complete: 42 JS files (1.8MB) + 3 source maps. Strategy: manifest-based (Vite detected).",
        startedAt: new Date(Date.now() - 8500).toISOString(),
        completedAt: now,
        durationMs: 8500,
      },
    });

    // Sub-agent 2: JS Analyzer (running)
    s.timelines[sessionId].push({
      id: `mock-sa-analyzer-${Date.now() + 1}`,
      type: "sub_agent_activity",
      timestamp: new Date(Date.now() + 10).toISOString(),
      batchId,
      data: {
        agentId: "js_analyzer_001",
        agentName: "JS Analyzer",
        parentRequestId: `mock-parent-req-${Date.now() + 1}`,
        task: "Security analysis of 42 collected JS files from admin.target.example.com",
        depth: 1,
        status: "running",
        toolCalls: [
          { id: "tc4", name: "list_files", args: { pattern: ".golish/js-assets/**/*.js" }, status: "completed", result: "42 files found", startedAt: now, completedAt: now },
          { id: "tc5", name: "grep_file", args: { pattern: "api[_-]?key|secret|token", path: ".golish/js-assets/" }, status: "running", startedAt: now },
        ],
        entries: [
          { kind: "text", text: "Listing all collected JS files..." },
          { kind: "tool_call", toolCallId: "tc4" },
          { kind: "text", text: "Scanning for API endpoints and hardcoded secrets across 42 files..." },
          { kind: "tool_call", toolCallId: "tc5" },
        ],
        streamingText: "Scanning for API endpoints and hardcoded secrets across 42 files...",
        startedAt: new Date(Date.now() - 3200).toISOString(),
      },
    });
  });
  console.log("[mockSubAgentBlocks] Injected 2 sub-agent blocks (grouped)");
}

/**
 * 4/4 — AI Tool Execution Block
 * Injects multiple ToolExecutionCards with different statuses.
 */
export async function mockToolExecutionBlocks(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];
  if (!sessionId) { console.error("[mockToolExecutionBlocks] No active session"); return; }

  const now = new Date().toISOString();

  useStore.setState((s) => {
    if (!s.timelines[sessionId]) s.timelines[sessionId] = [];

    // Tool 1: run_command (completed)
    s.timelines[sessionId].push({
      id: `mock-tool-cmd-${Date.now()}`,
      type: "ai_tool_execution",
      timestamp: now,
      data: {
        requestId: `mock-tool-cmd-${Date.now()}`,
        toolName: "run_command",
        args: { command: "subfinder -d target.example.com -silent | httpx -sc -title" },
        status: "completed",
        result: "https://api.target.example.com [200] [API Gateway]\nhttps://admin.target.example.com [200] [Admin Panel]\nhttps://staging.target.example.com [403] [Forbidden]",
        startedAt: new Date(Date.now() - 6300).toISOString(),
        completedAt: now,
        durationMs: 6300,
        autoApproved: true,
        riskLevel: "low",
      },
    });

    // Tool 2: read_file (completed)
    s.timelines[sessionId].push({
      id: `mock-tool-read-${Date.now()}`,
      type: "ai_tool_execution",
      timestamp: new Date(Date.now() + 10).toISOString(),
      data: {
        requestId: `mock-tool-read-${Date.now()}`,
        toolName: "read_file",
        args: { file_path: ".golish/js-assets/manifest.json" },
        status: "completed",
        result: '{"entries":{"main.js":"assets/main-a1b2c3.js","vendor.js":"assets/vendor-d4e5f6.js"}}',
        startedAt: new Date(Date.now() - 120).toISOString(),
        completedAt: now,
        durationMs: 120,
        autoApproved: true,
        riskLevel: "safe",
      },
    });

    // Tool 3: edit_file (running)
    s.timelines[sessionId].push({
      id: `mock-tool-edit-${Date.now()}`,
      type: "ai_tool_execution",
      timestamp: new Date(Date.now() + 20).toISOString(),
      data: {
        requestId: `mock-tool-edit-${Date.now()}`,
        toolName: "edit_file",
        args: { file_path: "src/config/targets.json", changes: "Add admin.target.example.com to scope" },
        status: "running",
        startedAt: now,
        riskLevel: "medium",
      },
    });

    // Tool 4: web_search (error)
    s.timelines[sessionId].push({
      id: `mock-tool-search-${Date.now()}`,
      type: "ai_tool_execution",
      timestamp: new Date(Date.now() + 30).toISOString(),
      data: {
        requestId: `mock-tool-search-${Date.now()}`,
        toolName: "web_search",
        args: { query: "target.example.com CVE vulnerabilities 2026" },
        status: "error",
        result: "TAVILY_API_KEY not configured",
        startedAt: new Date(Date.now() - 500).toISOString(),
        completedAt: now,
        durationMs: 500,
        riskLevel: "safe",
      },
    });
  });
  console.log("[mockToolExecutionBlocks] Injected 4 tool execution blocks (completed, read, running, error)");
}

/**
 * 5/5 — Plan → Pipeline Bridge
 * Simulates an AI plan_updated event that creates a pipeline progress block.
 * Call from console: __mockPlanPipeline()
 */
export async function mockPlanPipeline(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];
  if (!sessionId) { console.error("[mockPlanPipeline] No active session"); return; }

  useStore.getState().syncPlanToPipeline(sessionId, {
    version: 1,
    explanation: "Recon Pipeline — target.example.com",
    summary: { total: 6, completed: 3, in_progress: 1, pending: 2 },
    steps: [
      { step: "DNS Lookup — dig +short target.example.com", status: "completed" },
      { step: "Subdomain Enum — subfinder -d target.example.com", status: "completed" },
      { step: "HTTP Probe — httpx -l subdomains.txt -sc -title", status: "completed" },
      { step: "Port Scan — nmap -sV --top-ports 1000", status: "in_progress" },
      { step: "Tech Fingerprint — whatweb", status: "pending" },
      { step: "JS Harvest (AI) — js_harvest {target}", status: "pending" },
    ],
    updated_at: new Date().toISOString(),
  });
  console.log("[mockPlanPipeline] Injected plan→pipeline block into timeline");
}

/**
 * Showcase all timeline block types at once.
 * Call from console: __mockShowAllBlocks()
 */
export async function mockShowAllBlocks(): Promise<void> {
  await mockCommandBlock();
  await mockPlanPipeline();
  await mockToolExecutionBlocks();
  console.log("[mockShowAllBlocks] All block types injected");
}

// =============================================================================
// Full AI Plan Execution Demo (call from browser console)
// =============================================================================

/**
 * Simulate the complete AI-driven plan execution flow with proper session routing.
 * This mocks: started → text → update_plan → tool executions → completed.
 * The right chat TaskPlanCard will update.
 * Call from console: __mockFullPlan()
 */
export async function mockFullPlanExecution(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

  // Resolve the AI session ID from the active conversation
  const convId = state.activeConversationId;
  const conv = convId ? state.conversations[convId] : null;
  const aiSessionId = conv?.aiSessionId;
  const terminalSessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];

  if (!aiSessionId) {
    console.error("[mockFullPlan] No active conversation with AI session. Open a chat first.");
    return;
  }

  console.log("[mockFullPlan] Starting with AI session:", aiSessionId, "terminal:", terminalSessionId);

  // Helper to emit AI event with proper session_id
  const emit = (event: AiEventType) =>
    dispatchMockEvent("ai-event", { ...event, session_id: aiSessionId });

  const turnId = `mock-plan-${Date.now()}`;
  const reqId = (name: string) => `mock-${name}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

  // --- 1. AI turn starts ---
  await emit({ type: "started", turn_id: turnId });
  await delay(300);

  // --- 2. AI sends planning text ---
  const planText = "I'll create a task plan and execute each step.\n\n";
  await emit({ type: "text_delta", delta: planText, accumulated: planText });
  await delay(400);

  // --- 3. AI calls update_plan to create the plan (step 1 in_progress) ---
  const planReqId1 = reqId("update-plan-1");
  await emit({ type: "tool_request", tool_name: "update_plan", args: { explanation: "System reconnaissance", steps: [{ step: "Check system information", status: "in_progress" }, { step: "List files in workspace", status: "pending" }, { step: "Show network configuration", status: "pending" }] }, request_id: planReqId1 });
  await delay(200);

  // plan_updated event (emitted by the backend when update_plan runs)
  dispatchMockEvent("ai-event", { type: "plan_updated", session_id: aiSessionId, version: 1, explanation: "System reconnaissance", steps: [{ step: "Check system information", status: "in_progress" }, { step: "List files in workspace", status: "pending" }, { step: "Show network configuration", status: "pending" }], summary: { total: 3, completed: 0, in_progress: 1, pending: 2 } });
  await delay(100);

  await emit({ type: "tool_result", tool_name: "update_plan", result: "Plan created with 3 steps", success: true, request_id: planReqId1 });
  await delay(300);

  // --- 4. Step 1: run_command uname -a ---
  const cmdReqId1 = reqId("run-cmd-1");
  await emit({ type: "tool_request", tool_name: "run_command", args: { command: "uname -a && sw_vers" }, request_id: cmdReqId1 });
  await delay(500);

  // Streaming output
  dispatchMockEvent("ai-event", { type: "tool_output_chunk", session_id: aiSessionId, request_id: cmdReqId1, tool_name: "run_command", chunk: "Darwin MacBook-Pro.local 24.4.0 Darwin Kernel Version 24.4.0\n", stream: "stdout" });
  await delay(300);
  dispatchMockEvent("ai-event", { type: "tool_output_chunk", session_id: aiSessionId, request_id: cmdReqId1, tool_name: "run_command", chunk: "ProductName:    macOS\nProductVersion: 15.4\nBuildVersion:   24E5238a\n", stream: "stdout" });
  await delay(300);

  await emit({ type: "tool_result", tool_name: "run_command", result: "Darwin MacBook-Pro.local 24.4.0 ...", success: true, request_id: cmdReqId1 });
  await delay(200);

  // --- 5. Mark step 1 complete, step 2 in_progress ---
  const planReqId2 = reqId("update-plan-2");
  await emit({ type: "tool_request", tool_name: "update_plan", args: {}, request_id: planReqId2 });
  await delay(100);

  dispatchMockEvent("ai-event", { type: "plan_updated", session_id: aiSessionId, version: 2, explanation: "System reconnaissance", steps: [{ step: "Check system information", status: "completed" }, { step: "List files in workspace", status: "in_progress" }, { step: "Show network configuration", status: "pending" }], summary: { total: 3, completed: 1, in_progress: 1, pending: 1 } });
  await delay(100);

  await emit({ type: "tool_result", tool_name: "update_plan", result: "Plan updated", success: true, request_id: planReqId2 });
  await delay(300);

  // --- 6. Step 2: list_files ---
  const listReqId = reqId("list-files");
  await emit({ type: "tool_request", tool_name: "list_files", args: { path: "." }, request_id: listReqId });
  await delay(600);

  await emit({ type: "tool_result", tool_name: "list_files", result: "backend/\nfrontend/\npackage.json\nCargo.toml\nREADME.md\njustfile\n... (196 entries)", success: true, request_id: listReqId });
  await delay(200);

  // --- 7. Mark step 2 complete, step 3 in_progress ---
  const planReqId3 = reqId("update-plan-3");
  await emit({ type: "tool_request", tool_name: "update_plan", args: {}, request_id: planReqId3 });
  await delay(100);

  dispatchMockEvent("ai-event", { type: "plan_updated", session_id: aiSessionId, version: 3, explanation: "System reconnaissance", steps: [{ step: "Check system information", status: "completed" }, { step: "List files in workspace", status: "completed" }, { step: "Show network configuration", status: "in_progress" }], summary: { total: 3, completed: 2, in_progress: 1, pending: 0 } });
  await delay(100);

  await emit({ type: "tool_result", tool_name: "update_plan", result: "Plan updated", success: true, request_id: planReqId3 });
  await delay(300);

  // --- 8. Step 3: run_command ifconfig ---
  const cmdReqId2 = reqId("run-cmd-2");
  await emit({ type: "tool_request", tool_name: "run_command", args: { command: "ifconfig en0" }, request_id: cmdReqId2 });
  await delay(500);

  dispatchMockEvent("ai-event", { type: "tool_output_chunk", session_id: aiSessionId, request_id: cmdReqId2, tool_name: "run_command", chunk: "en0: flags=8863<UP,BROADCAST,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\tinet 192.168.0.69 netmask 0xffffff00 broadcast 192.168.0.255\n", stream: "stdout" });
  await delay(300);

  await emit({ type: "tool_result", tool_name: "run_command", result: "en0: flags=8863 ... inet 192.168.0.69", success: true, request_id: cmdReqId2 });
  await delay(200);

  // --- 9. Mark all steps complete ---
  const planReqId4 = reqId("update-plan-4");
  await emit({ type: "tool_request", tool_name: "update_plan", args: {}, request_id: planReqId4 });
  await delay(100);

  dispatchMockEvent("ai-event", { type: "plan_updated", session_id: aiSessionId, version: 4, explanation: "System reconnaissance", steps: [{ step: "Check system information", status: "completed" }, { step: "List files in workspace", status: "completed" }, { step: "Show network configuration", status: "completed" }], summary: { total: 3, completed: 3, in_progress: 0, pending: 0 } });
  await delay(100);

  await emit({ type: "tool_result", tool_name: "update_plan", result: "All steps completed", success: true, request_id: planReqId4 });
  await delay(300);

  // --- 10. AI sends summary text ---
  const summary = "All 3 steps completed:\n\n1. **System Info**: macOS 15.4 (Darwin 24.4.0)\n2. **Files**: 196 entries in workspace (Rust + React project)\n3. **Network**: en0 active at 192.168.0.69";
  const words = summary.split(" ");
  let accumulated = planText;
  for (const word of words) {
    const delta = accumulated.length > planText.length ? ` ${word}` : word;
    accumulated += delta;
    await emit({ type: "text_delta", delta, accumulated });
    await delay(30);
  }
  await delay(200);

  // --- 11. Turn complete ---
  await emit({ type: "completed", response: accumulated, tokens_used: 3200, duration_ms: 12000, input_tokens: 6400, output_tokens: 800 });

  console.log("[mockFullPlan] Complete! Check right chat for plan card, click it to see left pane detail.");
}

// =============================================================================
// AI run_command Approval Demo (call from browser console)
// Call from console: __mockRunCommand()
// =============================================================================

/**
 * Simulate AI executing multiple tool calls (auto-approved) that appear as
 * compact badges in the right chat and as ToolExecutionCards in the center
 * ToolDetailView. Click the badges to navigate.
 * Call from console: __mockRunCommand()
 */
export async function mockRunCommandApproval(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

  const convId = state.activeConversationId;
  const conv = convId ? state.conversations[convId] : null;
  const aiSessionId = conv?.aiSessionId;

  if (!aiSessionId) {
    console.error("[mockRunCommand] No active conversation with AI session. Open a chat first.");
    return;
  }

  console.log("[mockRunCommand] Starting with AI session:", aiSessionId);

  const emit = (event: AiEventType) =>
    dispatchMockEvent("ai-event", { ...event, session_id: aiSessionId });

  const turnId = `mock-runcmd-${Date.now()}`;
  const reqId = (name: string) => `mock-${name}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

  // 1. AI turn starts
  await emit({ type: "started", turn_id: turnId });
  await delay(200);

  // 2. AI types some text
  const text = "Let me check your system configuration.\n\n";
  let accumulated = "";
  for (const word of text.split(" ")) {
    const delta = accumulated ? ` ${word}` : word;
    accumulated += delta;
    await emit({ type: "text_delta", delta, accumulated });
    await delay(20);
  }
  await delay(200);

  // 3. Tool call 1: run_command (auto-approved)
  const cmd1Id = reqId("cmd1");
  await emit({
    type: "tool_auto_approved",
    request_id: cmd1Id,
    tool_name: "run_command",
    args: { command: "uname -a" },
    reason: "read-only command",
  });
  await delay(300);

  dispatchMockEvent("ai-event", {
    type: "tool_output_chunk",
    session_id: aiSessionId,
    request_id: cmd1Id,
    tool_name: "run_command",
    chunk: "Darwin MacBook-Pro.local 24.4.0 Darwin Kernel Version 24.4.0: root:xnu-11417.401.54~1/RELEASE_ARM64_T6031 arm64\n",
    stream: "stdout",
  });
  await delay(300);

  await emit({
    type: "tool_result",
    request_id: cmd1Id,
    tool_name: "run_command",
    result: "Darwin MacBook-Pro.local 24.4.0 Darwin Kernel Version 24.4.0",
    success: true,
  });
  await delay(200);

  // 4. Tool call 2: read_file (auto-approved)
  const readId = reqId("read");
  await emit({
    type: "tool_auto_approved",
    request_id: readId,
    tool_name: "read_file",
    args: { path: "/etc/hostname" },
    reason: "read-only tool",
  });
  await delay(300);
  await emit({
    type: "tool_result",
    request_id: readId,
    tool_name: "read_file",
    result: "MacBook-Pro.local",
    success: true,
  });
  await delay(200);

  // 5. Tool call 3: run_command (auto-approved)
  const cmd2Id = reqId("cmd2");
  await emit({
    type: "tool_auto_approved",
    request_id: cmd2Id,
    tool_name: "run_command",
    args: { command: "df -h | head -5" },
    reason: "read-only command",
  });
  await delay(300);

  dispatchMockEvent("ai-event", {
    type: "tool_output_chunk",
    session_id: aiSessionId,
    request_id: cmd2Id,
    tool_name: "run_command",
    chunk: "Filesystem       Size   Used  Avail Capacity  Mounted on\n/dev/disk3s1s1  460Gi  320Gi  140Gi    70%    /\n",
    stream: "stdout",
  });
  await delay(200);

  await emit({
    type: "tool_result",
    request_id: cmd2Id,
    tool_name: "run_command",
    result: "Filesystem       Size   Used  Avail Capacity  Mounted on\n/dev/disk3s1s1  460Gi  320Gi  140Gi    70%    /",
    success: true,
  });
  await delay(200);

  // 6. AI summary text
  const summary = "\n\nYour system is running macOS on Apple Silicon (arm64). Disk usage is at 70%.";
  for (const word of summary.split(" ")) {
    const delta = accumulated.length > 0 ? ` ${word}` : word;
    accumulated += delta;
    await emit({ type: "text_delta", delta, accumulated });
    await delay(20);
  }

  await emit({
    type: "completed",
    response: accumulated,
    tokens_used: 800,
    duration_ms: 4000,
    input_tokens: 1200,
    output_tokens: 200,
  });

  console.log("[mockRunCommand] Done! You should see tool badges (Shell, Read, Shell) in the right chat. Click them to open ToolDetailView in the center.");
}

// =============================================================================
// Pipeline Fan-Out Demo (call from browser console)
// =============================================================================

/**
 * Simulate a pipeline execution with data-flow and fan-out.
 * Call from console: __mockPipelineFanOut()
 */
export async function simulatePipelineFanOut(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];
  if (!sessionId) { console.error("[mockPipelineFanOut] No active session found. Sessions:", Object.keys(state.sessions)); return; }
  console.log("[mockPipelineFanOut] Using session:", sessionId);

  const now = () => new Date().toISOString();

  const mkStep = (id: string, name: string, cmd: string) => ({
    stepId: id, name, command: cmd, status: "pending" as const,
  });

  const execution = {
    pipelineId: "recon-basic",
    pipelineName: "Basic Reconnaissance",
    target: "example.com",
    steps: [
      mkStep("s1", "DNS Lookup", "dig +short example.com"),
      mkStep("s2", "Subdomain Enumeration", "subfinder -d example.com -silent"),
      mkStep("s3", "HTTP Probe", "httpx -l subdomains.txt -silent"),
      mkStep("s4", "Port Scan", "nmap -sV -T4 {target}"),
      mkStep("s5", "Tech Detection", "whatweb {target}"),
      mkStep("s6", "JS Harvest (AI)", "AI: js_harvest {target}"),
    ],
    status: "running" as const,
    startedAt: now(),
  };

  state.startPipelineExecution(sessionId, execution);
  const tl = useStore.getState().timelines[sessionId] ?? [];
  const blockId = tl[tl.length - 1]?.id ?? "";
  console.log("[mockPipelineFanOut] Block ID:", blockId, "Timeline length:", tl.length, "Last block type:", tl[tl.length - 1]?.type);

  const up = (stepId: string, u: Record<string, unknown>) =>
    useStore.getState().updatePipelineStep(sessionId, blockId, stepId, u);

  // Step 1: DNS
  up("s1", { status: "running", startedAt: now() });
  await delay(600);
  up("s1", { status: "success", finishedAt: now(), durationMs: 580, output: "93.184.216.34\n2606:2800:220:1:248:1893:25c8:1946" });

  // Step 2: Subdomain Enum — discovers 4 subs
  up("s2", { status: "running", startedAt: now() });
  await delay(1200);
  const subs = ["www.example.com", "api.example.com", "cdn.example.com", "admin.example.com"];
  up("s2", {
    status: "success", finishedAt: now(), durationMs: 3200,
    output: subs.join("\n"),
    discoveredTargets: subs,
  });

  // Step 3: HTTP Probe — fans out on subs, discovers live hosts
  up("s3", { status: "running", startedAt: now(), discoveredTargets: subs });
  await delay(800);
  const liveHosts = ["https://www.example.com", "https://api.example.com", "https://admin.example.com"];
  up("s3", {
    status: "success", finishedAt: now(), durationMs: 2100,
    output: liveHosts.join("\n"),
    discoveredTargets: liveHosts,
    subTargets: [
      { target: "www.example.com", status: "success" as const, durationMs: 450 },
      { target: "api.example.com", status: "success" as const, durationMs: 380 },
      { target: "cdn.example.com", status: "failed" as const, durationMs: 1200 },
      { target: "admin.example.com", status: "success" as const, durationMs: 520 },
    ],
  });

  // Step 4: Port Scan — fans out on live hosts
  up("s4", { status: "running", startedAt: now(), discoveredTargets: liveHosts });
  await delay(1500);
  up("s4", {
    status: "success", finishedAt: now(), durationMs: 12400,
    output: "www: 80,443\napi: 80,443,8080\nadmin: 443",
    subTargets: [
      { target: "https://www.example.com", status: "success" as const, durationMs: 4200 },
      { target: "https://api.example.com", status: "success" as const, durationMs: 3800 },
      { target: "https://admin.example.com", status: "success" as const, durationMs: 4400 },
    ],
  });

  // Step 5: Tech Detection
  up("s5", { status: "running", startedAt: now() });
  await delay(900);
  up("s5", {
    status: "success", finishedAt: now(), durationMs: 5600,
    output: "www: Nginx, React, Vite\napi: Nginx, Express, Node.js\nadmin: Nginx, Vue.js",
    subTargets: [
      { target: "https://www.example.com", status: "success" as const, durationMs: 1800 },
      { target: "https://api.example.com", status: "success" as const, durationMs: 1600 },
      { target: "https://admin.example.com", status: "success" as const, durationMs: 2200 },
    ],
  });

  // Step 6: JS Harvest (AI)
  up("s6", { status: "running", startedAt: now() });
  await delay(2000);
  up("s6", {
    status: "success", finishedAt: now(), durationMs: 18500,
    output: "[AI] Collected 58 JS files (2.8MB) + 6 source maps across 3 targets",
    subTargets: [
      { target: "https://www.example.com", status: "success" as const, durationMs: 8500 },
      { target: "https://api.example.com", status: "success" as const, durationMs: 4200 },
      { target: "https://admin.example.com", status: "success" as const, durationMs: 5800 },
    ],
  });

  useStore.getState().completePipelineExecution(sessionId, blockId, "completed");
}

// =============================================================================
// Mock Settings Accessors (for e2e testing)
// =============================================================================

/**
 * Get the current mock settings.
 * Use this in e2e tests to verify settings state.
 */
export function getMockSettings(): typeof mockSettings {
  return structuredClone(mockSettings);
}

/**
 * Update mock settings.
 * Use this in e2e tests to set up specific test scenarios.
 */
export function setMockSettings(settings: Partial<typeof mockSettings>): void {
  mockSettings = { ...mockSettings, ...settings };
}

/**
 * Update a specific provider's visibility in mock settings.
 * This is a convenience function for e2e testing the provider toggle feature.
 */
export function setMockProviderVisibility(
  provider:
    | "vertex_ai"
    | "openrouter"
    | "anthropic"
    | "openai"
    | "ollama"
    | "gemini"
    | "groq"
    | "xai"
    | "zai_sdk"
    | "nvidia",
  visible: boolean
): void {
  mockSettings.ai[provider].show_in_selector = visible;
}

// =============================================================================
// Setup Mock IPC
// =============================================================================

/**
 * Clean up mocks. Call this when unmounting or resetting.
 */
export function cleanupMocks(): void {
  clearMocks();
  console.log("[Mocks] Tauri mocks cleared");
}

export function setupMocks(): void {
  console.log("[Mocks] Setting up Tauri IPC mocks for browser development");

  // Set the browser mode flag BEFORE mockWindows creates __TAURI_INTERNALS__
  // This allows components to check isMockBrowserMode() after mocks are set up
  window.__MOCK_BROWSER_MODE__ = true;

  try {
    // Setup mock window context (required for Tauri internals)
    mockWindows("main");

    // Patch the Tauri event module's listen function to use our mock event system
    // ES module exports are read-only, so we use Object.defineProperty to override
    const originalListen = tauriEvent.listen;

    // Create our mock listen function
    const mockListen = async <T>(
      eventName: string,
      callback: (event: { event: string; payload: T }) => void
    ): Promise<() => void> => {
      console.log(`[Mock Events] listen("${eventName}") called`);

      // Register the callback with our mock event system
      const handlerId = mockRegisterListener(
        eventName,
        callback as (event: { event: string; payload: unknown }) => void
      );

      // Return an unlisten function
      return () => {
        mockUnregisterListener(handlerId);
      };
    };

    // Try to override the listen export using Object.defineProperty
    // Note: This usually fails because ES modules have read-only exports,
    // but we try anyway in case the bundler makes it writable
    try {
      Object.defineProperty(tauriEvent, "listen", {
        value: mockListen,
        writable: true,
        configurable: true,
      });
    } catch {
      // Expected to fail - we use the global fallback instead
      // Hooks check for window.__MOCK_LISTEN__ when in browser mode
    }

    // Store mock listen function globally as a fallback
    // Hooks can check for this when the module patch doesn't work
    (window as unknown as { __MOCK_LISTEN__?: typeof mockListen }).__MOCK_LISTEN__ = mockListen;

    // Expose mock event listeners for debugging in e2e tests
    (
      window as unknown as { __MOCK_EVENT_LISTENERS__?: typeof mockEventListeners }
    ).__MOCK_EVENT_LISTENERS__ = mockEventListeners;

    // Store reference to original for cleanup
    (
      window as unknown as { __MOCK_ORIGINAL_LISTEN__?: typeof originalListen }
    ).__MOCK_ORIGINAL_LISTEN__ = originalListen;

    // Expose mock event emitters globally for e2e testing
    (
      window as unknown as {
        __MOCK_EMIT_AI_EVENT__?: typeof emitAiEvent;
        __MOCK_SIMULATE_AI_RESPONSE_WITH_SUB_AGENT__?: typeof simulateAiResponseWithSubAgent;
        __MOCK_SIMULATE_AI_RESPONSE__?: typeof simulateAiResponse;
      }
    ).__MOCK_EMIT_AI_EVENT__ = emitAiEvent;
    (
      window as unknown as {
        __MOCK_SIMULATE_AI_RESPONSE_WITH_SUB_AGENT__?: typeof simulateAiResponseWithSubAgent;
      }
    ).__MOCK_SIMULATE_AI_RESPONSE_WITH_SUB_AGENT__ = simulateAiResponseWithSubAgent;
    (
      window as unknown as {
        __MOCK_SIMULATE_AI_RESPONSE__?: typeof simulateAiResponse;
      }
    ).__MOCK_SIMULATE_AI_RESPONSE__ = simulateAiResponse;
    (
      window as unknown as {
        __mockJsHarvest?: typeof simulateJsHarvest;
      }
    ).__mockJsHarvest = simulateJsHarvest;
    (
      window as unknown as {
        __mockPipelineFanOut?: typeof simulatePipelineFanOut;
      }
    ).__mockPipelineFanOut = simulatePipelineFanOut;

    // Expose per-block-type mock functions for visual QA
    (window as unknown as {
      __mockShowAllBlocks?: typeof mockShowAllBlocks;
      __mockCommandBlock?: typeof mockCommandBlock;
      __mockPipelineBlock?: typeof mockPipelineProgressBlock;
      __mockSubAgentBlocks?: typeof mockSubAgentBlocks;
      __mockToolExecutionBlocks?: typeof mockToolExecutionBlocks;
      __mockPlanPipeline?: typeof mockPlanPipeline;
    }).__mockShowAllBlocks = mockShowAllBlocks;
    (window as unknown as { __mockCommandBlock?: typeof mockCommandBlock }).__mockCommandBlock = mockCommandBlock;
    (window as unknown as { __mockPipelineBlock?: typeof mockPipelineProgressBlock }).__mockPipelineBlock = mockPipelineProgressBlock;
    (window as unknown as { __mockPlanPipeline?: typeof mockPlanPipeline }).__mockPlanPipeline = mockPlanPipeline;
    (window as unknown as { __mockSubAgentBlocks?: typeof mockSubAgentBlocks }).__mockSubAgentBlocks = mockSubAgentBlocks;
    (window as unknown as { __mockToolExecutionBlocks?: typeof mockToolExecutionBlocks }).__mockToolExecutionBlocks = mockToolExecutionBlocks;
    (window as unknown as { __mockFullPlan?: typeof mockFullPlanExecution }).__mockFullPlan = mockFullPlanExecution;
    (window as unknown as { __mockRunCommand?: typeof mockRunCommandApproval }).__mockRunCommand = mockRunCommandApproval;

    // Expose command simulation functions for e2e testing
    (
      window as unknown as {
        __MOCK_SIMULATE_COMMAND__?: typeof simulateCommand;
        __MOCK_EMIT_COMMAND_BLOCK_EVENT__?: typeof emitCommandBlockEvent;
        __MOCK_EMIT_TERMINAL_OUTPUT__?: typeof emitTerminalOutput;
      }
    ).__MOCK_SIMULATE_COMMAND__ = simulateCommand;
    (
      window as unknown as {
        __MOCK_EMIT_COMMAND_BLOCK_EVENT__?: typeof emitCommandBlockEvent;
      }
    ).__MOCK_EMIT_COMMAND_BLOCK_EVENT__ = emitCommandBlockEvent;

    // Expose git state controls for e2e testing
    (
      window as unknown as {
        __MOCK_SET_GIT_STATE__?: typeof setMockGitState;
      }
    ).__MOCK_SET_GIT_STATE__ = setMockGitState;
    (
      window as unknown as {
        __MOCK_EMIT_TERMINAL_OUTPUT__?: typeof emitTerminalOutput;
      }
    ).__MOCK_EMIT_TERMINAL_OUTPUT__ = emitTerminalOutput;
  } catch (error) {
    console.error("[Mocks] Error during initial setup:", error);
  }

  mockIPC((cmd, args) => {
    console.log(`[Mock IPC] Command: ${cmd}`, args);

    switch (cmd) {
      // =========================================================================
      // PTY Commands
      // =========================================================================
      case "pty_create": {
        const payload = args as { workingDirectory?: string; rows?: number; cols?: number };
        // First create returns the stable id; subsequent creates get incrementing ids.
        const id =
          mockPtySessionCounter === 1
            ? "mock-session-001"
            : `mock-session-${String(mockPtySessionCounter).padStart(3, "0")}`;

        const session = {
          id,
          working_directory: payload.workingDirectory ?? "/home/user",
          rows: payload.rows ?? 24,
          cols: payload.cols ?? 80,
        };

        mockPtySessions[id] = session;
        mockPtySessionCounter += 1;
        return session;
      }

      case "pty_write":
        // Simulate writing to PTY - in real app this would send data to the terminal
        return undefined;

      case "pty_resize": {
        const resizePayload = args as { sessionId: string; rows: number; cols: number };
        const session = mockPtySessions[resizePayload.sessionId];
        if (session) {
          session.rows = resizePayload.rows;
          session.cols = resizePayload.cols;
        }
        return undefined;
      }

      case "pty_destroy":
        return undefined;

      case "pty_get_session": {
        const getPayload = args as { sessionId: string };
        return mockPtySessions[getPayload.sessionId] ?? null;
      }

      // =========================================================================
      // Shell Integration Commands
      // =========================================================================
      case "shell_integration_status":
        return { type: "Installed", version: "1.0.0" };

      case "shell_integration_install":
        return undefined;

      case "shell_integration_uninstall":
        return undefined;

      case "get_git_branch":
        // Return mock branch name for browser mode
        return mockGitBranch;

      case "git_status":
        // Return mock git status summary for browser mode
        return mockGitStatus;

      // =========================================================================
      // Theme Commands
      // =========================================================================
      case "list_themes":
        // Return empty array - no custom themes in mock mode
        return [];

      case "read_theme":
        return JSON.stringify({
          name: "Mock Theme",
          colors: {
            background: "#1e1e1e",
            foreground: "#d4d4d4",
          },
        });

      // =========================================================================
      // Input Classification (auto mode)
      // =========================================================================
      case "classify_input":
        return { route: "terminal", detected_command: null };

      // =========================================================================
      // Workspace Commands
      // =========================================================================
      case "list_workspace_files":
        // Return mock file list
        return [
          { name: "src/App.tsx", path: "/home/user/src/App.tsx" },
          { name: "src/main.tsx", path: "/home/user/src/main.tsx" },
          { name: "package.json", path: "/home/user/package.json" },
        ];

      case "list_path_completions": {
        // Return mock path completions for tab completion feature
        const pathPayload = args as { sessionId: string; partialPath: string; limit?: number };
        const prefix = pathPayload.partialPath.split("/").pop() ?? "";
        const limit = pathPayload.limit ?? 20;

        // Mock completions - directories and files
        const allCompletions = [
          { name: "src/", insert_text: "src/", entry_type: "directory" as const },
          { name: "node_modules/", insert_text: "node_modules/", entry_type: "directory" as const },
          { name: "public/", insert_text: "public/", entry_type: "directory" as const },
          { name: "dist/", insert_text: "dist/", entry_type: "directory" as const },
          { name: ".git/", insert_text: ".git/", entry_type: "directory" as const },
          { name: "package.json", insert_text: "package.json", entry_type: "file" as const },
          { name: "tsconfig.json", insert_text: "tsconfig.json", entry_type: "file" as const },
          { name: "vite.config.ts", insert_text: "vite.config.ts", entry_type: "file" as const },
          { name: "README.md", insert_text: "README.md", entry_type: "file" as const },
          { name: ".gitignore", insert_text: ".gitignore", entry_type: "file" as const },
        ];

        // Fuzzy match helper: returns [score, matchIndices] or null if no match
        const fuzzyMatch = (
          text: string,
          pattern: string
        ): { score: number; indices: number[] } | null => {
          if (!pattern) return { score: 0, indices: [] };

          const textLower = text.toLowerCase();
          const patternLower = pattern.toLowerCase();
          const indices: number[] = [];
          let patternIdx = 0;

          for (let i = 0; i < text.length && patternIdx < patternLower.length; i++) {
            if (textLower[i] === patternLower[patternIdx]) {
              indices.push(i);
              patternIdx++;
            }
          }

          // All pattern characters must be found
          if (patternIdx !== patternLower.length) return null;

          // Score: prefer consecutive matches and earlier matches
          let score = 100;
          for (let i = 1; i < indices.length; i++) {
            if (indices[i] === indices[i - 1] + 1) {
              score += 10; // Bonus for consecutive
            }
          }
          score -= indices[0] * 2; // Penalty for late first match

          return { score, indices };
        };

        // Filter by fuzzy match and hidden file rules
        const showHidden = prefix.startsWith(".");
        const matched = allCompletions
          .map((c) => {
            const name = c.name.replace(/\/$/, "");
            const isHidden = name.startsWith(".");
            if (isHidden && !showHidden) return null;
            if (!prefix) return isHidden ? null : { ...c, score: 0, match_indices: [] as number[] };

            const result = fuzzyMatch(name, prefix);
            if (!result) return null;
            return { ...c, score: result.score, match_indices: result.indices };
          })
          .filter((c): c is NonNullable<typeof c> => c !== null);

        // Sort: by score descending, then directories first, then alphabetically
        matched.sort((a, b) => {
          // Score descending
          if (b.score !== a.score) return b.score - a.score;
          // Directories first
          const aIsDir = a.entry_type === "directory";
          const bIsDir = b.entry_type === "directory";
          if (aIsDir && !bIsDir) return -1;
          if (!aIsDir && bIsDir) return 1;
          return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
        });

        const totalCount = matched.length;
        const limited = matched.slice(0, limit);

        return {
          completions: limited,
          total_count: totalCount,
        };
      }

      // =========================================================================
      // Sidecar Commands
      // =========================================================================
      case "sidecar_status":
        return {
          active_session: false,
          session_id: null,
          enabled: true,
          sessions_dir: "/home/user/.golish/sessions",
          workspace_path: "/home/user",
        };

      // =========================================================================
      // Prompt Commands
      // =========================================================================
      case "list_prompts":
        return mockPrompts;

      case "read_prompt":
        return "# Mock Prompt\n\nThis is a mock prompt content for browser development.";

      // =========================================================================
      // Skill Commands
      // =========================================================================
      case "list_skills":
        return mockSkills;

      case "read_skill":
        return "---\nname: mock-skill\ndescription: Mock skill\n---\n\n# Mock Skill Instructions";

      case "read_skill_body":
        return "# Mock Skill\n\nMock skill content for browser development.";

      case "list_skill_files":
        return [];

      case "read_skill_file":
        return "# Mock skill file content";

      // =========================================================================
      // AI Agent Commands
      // =========================================================================
      case "init_ai_agent":
      case "init_ai_agent_vertex":
        mockAiInitialized = true;
        mockConversationLength = 0;
        return undefined;

      case "send_ai_prompt":
        // In browser mode, we just return a mock response
        // Real streaming events would come from the backend
        mockConversationLength += 2; // User message + AI response
        return `mock-turn-id-${Date.now()}`;

      case "execute_ai_tool":
        return { success: true, result: "Mock tool execution result" };

      case "get_available_tools":
        return mockTools;

      case "list_workflows":
        return mockWorkflows;

      case "list_sub_agents":
        return mockSubAgents;

      case "shutdown_ai_agent":
        mockAiInitialized = false;
        mockConversationLength = 0;
        return undefined;

      case "is_ai_initialized":
        return mockAiInitialized;

      case "update_ai_workspace":
        return undefined;

      case "clear_ai_conversation":
        mockConversationLength = 0;
        return undefined;

      case "get_ai_conversation_length":
        return mockConversationLength;

      case "get_openrouter_api_key":
        return null; // No API key in mock mode

      case "load_env_file":
        return 0; // No variables loaded in mock mode

      case "get_vertex_ai_config":
        // Return mock credentials so the app can initialize in browser mode
        return {
          credentials_path: "/mock/path/to/credentials.json",
          project_id: "mock-project-id",
          location: "us-east5",
        };

      // =========================================================================
      // Session-Specific AI Commands (Per-Tab Isolation)
      // =========================================================================
      case "init_ai_session": {
        validateRequiredParams(cmd, args, ["sessionId", "config"]);
        const payload = args as { sessionId: string; config: unknown };
        mockSessionAiState.set(payload.sessionId, {
          initialized: true,
          conversationLength: 0,
          config: payload.config,
        });
        return undefined;
      }

      case "shutdown_ai_session": {
        validateRequiredParams(cmd, args, ["sessionId"]);
        const payload = args as { sessionId: string };
        mockSessionAiState.delete(payload.sessionId);
        return undefined;
      }

      case "is_ai_session_initialized": {
        validateRequiredParams(cmd, args, ["sessionId"]);
        const payload = args as { sessionId: string };
        return mockSessionAiState.has(payload.sessionId);
      }

      case "get_session_ai_config": {
        validateRequiredParams(cmd, args, ["sessionId"]);
        const payload = args as { sessionId: string };
        const state = mockSessionAiState.get(payload.sessionId);
        if (!state) return null;
        return {
          provider_name: "mock_provider",
          model_name: "mock-model",
          config: state.config,
        };
      }

      case "send_ai_prompt_session": {
        validateRequiredParams(cmd, args, ["sessionId", "prompt"]);
        const payload = args as { sessionId: string; prompt: string };
        const state = mockSessionAiState.get(payload.sessionId);
        if (state) {
          state.conversationLength += 2;
        }
        const promptLower = (payload.prompt || "").toLowerCase();
        if (promptLower.includes("js") || promptLower.includes("javascript") || promptLower.includes("analyze")) {
          setTimeout(() => simulateJsHarvest(), 300);
        } else {
          setTimeout(() => {
            simulateAiResponse(
              "I can help with that. What would you like me to do? Try asking me to 'analyze JS files' to see the JS Analyzer sub-agent in action.",
              30
            );
          }, 300);
        }
        return `mock-turn-id-${Date.now()}`;
      }

      case "clear_ai_conversation_session": {
        validateRequiredParams(cmd, args, ["sessionId"]);
        const payload = args as { sessionId: string };
        const state = mockSessionAiState.get(payload.sessionId);
        if (state) {
          state.conversationLength = 0;
        }
        return undefined;
      }

      case "get_ai_conversation_length_session": {
        validateRequiredParams(cmd, args, ["sessionId"]);
        const payload = args as { sessionId: string };
        const state = mockSessionAiState.get(payload.sessionId);
        return state?.conversationLength ?? 0;
      }

      // =========================================================================
      // Session Persistence Commands
      // =========================================================================
      case "list_ai_sessions":
        return mockSessions;

      case "find_ai_session": {
        const findPayload = args as { identifier: string };
        return mockSessions.find((s) => s.identifier === findPayload.identifier) ?? null;
      }

      case "load_ai_session": {
        const loadPayload = args as { identifier: string };
        const session = mockSessions.find((s) => s.identifier === loadPayload.identifier);
        if (!session) return null;
        return {
          ...session,
          transcript: ["User: Hello", "Assistant: Hi! How can I help you?"],
          messages: [
            { role: "user", content: "Hello" },
            { role: "assistant", content: "Hi! How can I help you?" },
          ],
        };
      }

      case "export_ai_session_transcript":
        return undefined;

      case "set_ai_session_persistence": {
        const persistPayload = args as { enabled: boolean };
        mockSessionPersistenceEnabled = persistPayload.enabled;
        return undefined;
      }

      case "is_ai_session_persistence_enabled":
        return mockSessionPersistenceEnabled;

      case "finalize_ai_session":
        return "/home/user/.golish/sessions/mock-session.json";

      case "restore_ai_session": {
        const restorePayload = args as { identifier: string };
        const restoredSession = mockSessions.find(
          (s) => s.identifier === restorePayload.identifier
        );
        if (!restoredSession) {
          throw new Error(`Session not found: ${restorePayload.identifier}`);
        }
        mockConversationLength = restoredSession.total_messages;
        return {
          ...restoredSession,
          transcript: ["User: Hello", "Assistant: Hi! How can I help you?"],
          messages: [
            { role: "user", content: "Hello" },
            { role: "assistant", content: "Hi! How can I help you?" },
          ],
        };
      }

      // =========================================================================
      // HITL (Human-in-the-Loop) Commands
      // =========================================================================
      case "get_approval_patterns":
        return mockApprovalPatterns;

      case "get_tool_approval_pattern": {
        const patternPayload = args as { toolName: string };
        return mockApprovalPatterns.find((p) => p.tool_name === patternPayload.toolName) ?? null;
      }

      case "get_hitl_config":
        return mockHitlConfig;

      case "set_hitl_config": {
        const configPayload = args as { config: typeof mockHitlConfig };
        mockHitlConfig = configPayload.config;
        return undefined;
      }

      case "add_tool_always_allow": {
        const addPayload = args as { toolName: string };
        if (!mockHitlConfig.always_allow.includes(addPayload.toolName)) {
          mockHitlConfig.always_allow.push(addPayload.toolName);
        }
        return undefined;
      }

      case "remove_tool_always_allow": {
        const removePayload = args as { toolName: string };
        mockHitlConfig.always_allow = mockHitlConfig.always_allow.filter(
          (t) => t !== removePayload.toolName
        );
        return undefined;
      }

      case "reset_approval_patterns":
        return undefined;

      case "respond_to_tool_approval":
        return undefined;

      // =========================================================================
      // Indexer Commands
      // =========================================================================
      case "init_indexer": {
        const initPayload = args as { workspacePath: string };
        mockIndexerInitialized = true;
        mockIndexerWorkspace = initPayload.workspacePath;
        mockIndexedFileCount = 42; // Mock some indexed files
        return {
          files_indexed: 42,
          success: true,
          message: "Mock indexer initialized successfully",
        };
      }

      case "is_indexer_initialized":
        return mockIndexerInitialized;

      case "get_indexer_workspace":
        return mockIndexerWorkspace;

      case "get_indexed_file_count":
        return mockIndexedFileCount;

      case "index_file":
        mockIndexedFileCount += 1;
        return {
          files_indexed: 1,
          success: true,
          message: "File indexed successfully",
        };

      case "index_directory":
        mockIndexedFileCount += 10;
        return {
          files_indexed: 10,
          success: true,
          message: "Directory indexed successfully",
        };

      case "search_code":
        return [
          {
            file_path: "/home/user/golish/src/lib/ai.ts",
            line_number: 42,
            line_content: "export async function initAiAgent(config: AiConfig): Promise<void> {",
            matches: ["initAiAgent"],
          },
          {
            file_path: "/home/user/golish/src/lib/tauri.ts",
            line_number: 15,
            line_content: "export async function ptyCreate(",
            matches: ["ptyCreate"],
          },
        ];

      case "search_files":
        return [
          "/home/user/golish/src/lib/ai.ts",
          "/home/user/golish/src/lib/tauri.ts",
          "/home/user/golish/src/lib/indexer.ts",
        ];

      case "shutdown_indexer":
        mockIndexerInitialized = false;
        mockIndexerWorkspace = null;
        mockIndexedFileCount = 0;
        return undefined;

      // =========================================================================
      // Codebase Management Commands
      // =========================================================================
      case "list_indexed_codebases":
        return structuredClone(mockCodebases);

      case "add_indexed_codebase": {
        const addPayload = args as { path: string };
        const newCodebase: MockCodebase = {
          path: addPayload.path,
          file_count: Math.floor(Math.random() * 200) + 50,
          status: "synced",
          memory_file: undefined,
        };
        mockCodebases.push(newCodebase);
        return structuredClone(newCodebase);
      }

      case "remove_indexed_codebase": {
        const removePayload = args as { path: string };
        mockCodebases = mockCodebases.filter((cb) => cb.path !== removePayload.path);
        return undefined;
      }

      case "reindex_codebase": {
        const reindexPayload = args as { path: string };
        const codebase = mockCodebases.find((cb) => cb.path === reindexPayload.path);
        if (codebase) {
          codebase.file_count = Math.floor(Math.random() * 200) + 50;
          codebase.status = "synced";
          return structuredClone(codebase);
        }
        throw new Error(`Codebase not found: ${reindexPayload.path}`);
      }

      case "update_codebase_memory_file": {
        const updatePayload = args as { path: string; memoryFile: string | null };
        const codebase = mockCodebases.find((cb) => cb.path === updatePayload.path);
        if (codebase) {
          codebase.memory_file = updatePayload.memoryFile ?? undefined;
        }
        return undefined;
      }

      case "detect_memory_files": {
        // Simulate detecting memory files - randomly return one of the options
        const detectOptions = ["AGENTS.md", "CLAUDE.md", null];
        return detectOptions[Math.floor(Math.random() * detectOptions.length)];
      }

      // =========================================================================
      // Home View Commands
      // =========================================================================
      case "list_projects_for_home":
        // Return mock projects for the home view
        return [
          {
            path: "/home/user/projects/golish",
            name: "golish",
            branches: [
              {
                name: "main",
                path: "/home/user/projects/golish",
                file_count: 0,
                insertions: 0,
                deletions: 0,
                last_activity: "2h ago",
              },
              {
                name: "feature/home-view",
                path: "/home/user/projects/golish-feature-home-view",
                file_count: 3,
                insertions: 42,
                deletions: 12,
                last_activity: "1h ago",
              },
            ],
            warnings: 0,
            last_activity: "1h ago",
          },
        ];

      case "list_recent_directories": {
        // Return mock recent directories
        return [
          {
            path: "/home/user/projects/golish",
            name: "golish",
            branch: "main",
            file_count: 0,
            insertions: 0,
            deletions: 0,
            last_accessed: "2h ago",
          },
          {
            path: "/home/user/projects/other-project",
            name: "other-project",
            branch: "develop",
            file_count: 5,
            insertions: 100,
            deletions: 50,
            last_accessed: "1d ago",
          },
        ];
      }

      case "list_git_branches":
        // Return mock git branches
        return ["main", "develop", "feature/new-feature"];

      case "create_git_worktree":
        return {
          path: "/home/user/projects/golish-new-branch",
          branch: "new-branch",
          init_script_run: false,
          init_script_output: null,
        };

      case "save_project":
        console.log("[Mock IPC] save_project:", args);
        return undefined;

      case "delete_project_config":
        console.log("[Mock IPC] delete_project_config:", args);
        return true;

      case "list_project_configs":
        return [
          { name: "golish", rootPath: "/home/user/projects/golish" },
          { name: "my-pentest", rootPath: "/home/user/projects/my-pentest" },
        ];

      case "get_project_config":
        return { name: "golish", rootPath: "/home/user/projects/golish" };

      case "save_project_workspace":
        console.log("[Mock IPC] save_project_workspace:", args);
        return undefined;

      case "load_project_workspace":
        return null;

      // =========================================================================
      // Settings Commands
      // =========================================================================
      case "get_settings":
        return structuredClone(mockSettings);

      case "update_settings": {
        const updatePayload = args as { settings: typeof mockSettings };
        mockSettings = structuredClone(updatePayload.settings);
        return undefined;
      }

      case "get_setting": {
        const getPayload = args as { key: string };
        const keys = getPayload.key.split(".");
        let value: unknown = mockSettings;
        for (const k of keys) {
          if (value && typeof value === "object" && k in value) {
            value = (value as Record<string, unknown>)[k];
          } else {
            return null;
          }
        }
        return value;
      }

      case "set_setting": {
        const setPayload = args as { key: string; value: unknown };
        const keys = setPayload.key.split(".");
        let target: Record<string, unknown> = mockSettings as unknown as Record<string, unknown>;
        for (let i = 0; i < keys.length - 1; i++) {
          const k = keys[i];
          if (target[k] && typeof target[k] === "object") {
            target = target[k] as Record<string, unknown>;
          } else {
            return undefined;
          }
        }
        target[keys[keys.length - 1]] = setPayload.value;
        return undefined;
      }

      case "reset_settings":
        // Reset to defaults - in mock mode we just return
        return undefined;

      case "reload_settings":
        // Reload from disk - in mock mode we just return
        return undefined;

      case "settings_file_exists":
        return true;

      case "get_settings_path":
        return "/home/user/.golish/settings.toml";

      // =========================================================================
      // Project Settings Commands (per-project .golish/project.toml)
      // =========================================================================
      case "get_project_settings": {
        // Return mock project settings - provider, model, agent_mode
        return mockProjectSettings;
      }

      case "save_project_model": {
        const payload = args as { workspace: string; provider: string; model: string };
        mockProjectSettings.provider = payload.provider;
        mockProjectSettings.model = payload.model;
        console.log(`[Mock IPC] Saved project model: ${payload.provider}/${payload.model}`);
        return undefined;
      }

      case "save_project_agent_mode": {
        const payload = args as { workspace: string; mode: string };
        mockProjectSettings.agent_mode = payload.mode;
        console.log(`[Mock IPC] Saved project agent mode: ${payload.mode}`);
        return undefined;
      }

      // =========================================================================
      // Tauri Plugin Commands (event system)
      // Note: We patch tauriEvent.listen directly, so these handlers are just
      // for compatibility if any code calls invoke() directly
      // =========================================================================
      case "plugin:event|listen": {
        const payload = args as { event: string; handler: number };
        // Return the handler ID - actual registration happens via patched listen()
        return payload.handler;
      }

      case "plugin:event|unlisten": {
        const payload = args as { event: string; eventId: number };
        mockUnregisterListener(payload.eventId);
        return undefined;
      }

      case "plugin:event|emit": {
        // Emit is handled by our emit() calls, just acknowledge it
        return undefined;
      }

      // =========================================================================
      // History Commands
      // =========================================================================
      case "add_command_history":
      case "add_prompt_history":
        return undefined;

      case "load_history":
      case "search_history":
        return [];

      case "clear_history":
        return undefined;

      // =========================================================================
      // Pentest panels (return valid empty structures)
      // =========================================================================
      case "target_list":
        return { targets: [], groups: ["default"] };
      case "target_clear_all":
      case "target_add":
      case "target_remove":
        return undefined;
      case "findings_list":
        return { findings: [] };
      case "findings_add":
      case "findings_delete":
      case "findings_update":
      case "findings_add_evidence":
      case "findings_remove_evidence":
      case "findings_deduplicate":
      case "findings_import_parsed":
        return undefined;
      case "findings_for_host":
        return [];
      case "method_list_templates":
      case "method_list_projects":
        return [];
      case "method_start_project":
      case "method_load_project":
      case "method_delete_project":
      case "method_update_item":
        return undefined;
      case "topo_list":
        return [];
      case "topo_save":
      case "topo_delete":
      case "topo_diff":
        return undefined;
      case "wiki_list":
        return [];
      case "pipeline_list":
        return [
          {
            id: "recon-basic",
            name: "Basic Reconnaissance",
            description: "DNS, subdomains, HTTP probe, ports, tech detection, JS harvest",
            steps: [
              { id: "dns_lookup", command_template: "dig +short {target}", tool_name: "dig", args: [] },
              { id: "subdomain_enum", command_template: "subfinder -d {target} -silent", tool_name: "subfinder", args: [] },
              { id: "http_probe", command_template: "echo {target} | httpx -silent", tool_name: "httpx", args: [] },
              { id: "port_scan", command_template: "nmap -sV -T4 {target}", tool_name: "nmap", args: [] },
              { id: "tech_detect", command_template: "whatweb {target}", tool_name: "whatweb", args: [] },
              { id: "js_harvest", command_template: "", tool_name: "js_harvest", args: [] },
            ],
          },
        ];
      case "pipeline_save":
        return "mock-pipeline-id";
      case "pipeline_delete":
        return undefined;
      case "scan_queue_list":
        return [];
      case "scan_queue_upsert":
        return "mock-scan-queue-id";
      case "scan_queue_save_all":
      case "scan_queue_remove":
      case "scan_queue_clear_completed":
        return undefined;
      case "custom_rules_list":
        return [];
      case "custom_rules_upsert":
      case "custom_rules_save_all":
      case "custom_rules_delete":
        return undefined;
      case "audit_list":
        return [];
      case "audit_clear":
      case "audit_log":
        return undefined;
      case "notes_list":
        return [];
      case "notes_add":
      case "notes_update":
      case "notes_delete":
        return undefined;
      case "vault_list":
        return [];
      case "vault_get_value":
        return "";
      case "pentest_scan_tools":
        return { success: true, tools: [] };
      case "pentest_get_categories":
        return [];
      case "pentest_get_config":
        return {};
      case "pentest_check_env_setup":
        return { ready: false, missing: [] };
      case "pentest_check_runtime":
        return { ready: false, version: null };
      case "pentest_resolve_python_path":
      case "pentest_resolve_java_path":
        return "";
      case "pentest_read_tool_config":
        return "{}";
      case "pentest_list_dep_files":
        return [];
      case "pentest_check_tool_updates":
        return [];
      case "pentest_browser_close":
      case "pentest_install_runtime":
      case "pentest_open_directory":
      case "pentest_save_tool_config":
      case "pentest_uninstall_tool_files":
      case "pentest_git_clone_tool":
      case "pentest_rename_tool_dir":
        return undefined;
      case "zap_api_call":
        return {};
      case "zap_status":
        return { status: "stopped", port: 8090 };
      case "zap_detect_path":
        return null;
      case "check_recon_tools_cmd":
        return { tools: [] };
      case "wordlist_list":
      case "wordlist_preview":
        return [];
      case "wordlist_import":
      case "wordlist_delete":
      case "wordlist_merge":
        return undefined;
      case "wordlist_deduplicate":
        return null;
      case "wordlist_path":
        return "";
      case "intel_get_cached":
      case "intel_fetch":
      case "intel_fetch_page":
      case "intel_search":
      case "intel_search_remote":
        return [];
      case "intel_list_feeds":
        return [];
      case "intel_toggle_feed":
      case "intel_delete_feed":
      case "intel_add_feed":
        return undefined;
      case "write_frontend_log":
        return undefined;
      case "unwatch_all_files":
        return undefined;

      // =========================================================================
      // Default: Unhandled command — return safe fallback
      // =========================================================================
      default:
        if (!cmd.startsWith("plugin:")) {
          console.warn(`[Mock IPC] Unhandled command: ${cmd}`, args);
        }
        if (cmd.endsWith("_list") || cmd.endsWith("_list_feeds")) return [];
        return undefined;
    }
  });

  console.log("[Mocks] Tauri IPC mocks initialized successfully (v2-patched)");

  // Dev-only: expose sub-agent demo on window for testing multi-agent UI
  let demoRunCount = 0;
  (window as unknown as Record<string, unknown>).demoSubAgents = () => {
    import("@/store/index").then(({ useStore }) => {
      const store = useStore.getState();
      const sessionId = store.activeSessionId;
      if (!sessionId) {
        console.warn("[Demo] No active session");
        return;
      }
      demoRunCount++;
      const runId = demoRunCount;

      const convId = store.activeConversationId;
      const getConv = () => convId ? useStore.getState().conversations[convId] : null;

      const addUserMsg = (text: string) => {
        const conv = getConv();
        if (!conv || !convId) return;
        const msg = {
          id: `demo-user-${Date.now()}`,
          sessionId,
          role: "user" as const,
          content: text,
          timestamp: Date.now(),
        };
        useStore.setState((s) => {
          const c = s.conversations[convId];
          if (c) c.messages.push(msg);
        });
      };

      // addAssistantMsg removed — sub-agent demo uses direct store manipulation

      const taskSets = [
        [
          { id: "researcher", name: "Researcher", task: "Searching codebase for authentication patterns and JWT usage" },
          { id: "coder", name: "Coder", task: "Implementing JWT token validation middleware with refresh logic" },
          { id: "reviewer", name: "Reviewer", task: "Reviewing code changes for security vulnerabilities" },
        ],
        [
          { id: "analyst", name: "Analyst", task: "Analyzing API endpoint performance bottlenecks" },
          { id: "coder", name: "Coder", task: "Optimizing database queries and adding connection pooling" },
        ],
        [
          { id: "researcher", name: "Researcher", task: "Investigating memory leak in worker threads" },
          { id: "coder", name: "Coder", task: "Fixing resource cleanup in async handlers" },
          { id: "explorer", name: "Explorer", task: "Scanning for similar patterns in related modules" },
          { id: "reviewer", name: "Reviewer", task: "Verifying fix doesn't introduce regressions" },
        ],
      ];
      const agents = taskSets[(runId - 1) % taskSets.length];

      const tasks = ["Help me add JWT authentication to the API endpoints", "Optimize the database layer for better performance", "Fix the memory leak in the worker pool"];
      console.log(`[Demo] Run #${runId} - Starting ${agents.length} sub-agents for session:`, sessionId);

      addUserMsg(tasks[(runId - 1) % tasks.length]);

      setTimeout(() => {
        useStore.getState().updateAgentStreaming(sessionId, "I'll help you implement JWT authentication. Let me coordinate multiple agents to handle this efficiently.\n\n");
        useStore.getState().setAgentResponding(sessionId, true);
      }, 500);

      agents.forEach((a, i) => {
        setTimeout(() => {
          useStore.getState().startSubAgent(sessionId, {
            agentId: a.id,
            agentName: a.name,
            parentRequestId: `demo-req-${runId}-${a.id}`,
            task: a.task,
            depth: 1,
          });
          console.log(`[Demo] Started: ${a.name}`);
        }, 1000 + i * 1200);

        setTimeout(() => {
          useStore.getState().addSubAgentToolCall(sessionId, `demo-req-${runId}-${a.id}`, {
            id: `tool-${runId}-${a.id}-1`,
            name: a.id === "researcher" ? "semantic_search" : a.id === "coder" ? "write_file" : "read_file",
            args: a.id === "researcher"
              ? { query: "JWT authentication middleware patterns" }
              : a.id === "coder"
                ? { path: "src/middleware/auth.ts", content: "..." }
                : { path: "src/middleware/auth.ts" },
          });
        }, 1000 + i * 1200 + 1500);

        setTimeout(() => {
          useStore.getState().addSubAgentToolCall(sessionId, `demo-req-${runId}-${a.id}`, {
            id: `tool-${runId}-${a.id}-2`,
            name: a.id === "researcher" ? "read_file" : a.id === "coder" ? "run_command" : "semantic_search",
            args: a.id === "researcher"
              ? { path: "src/config/auth.ts" }
              : a.id === "coder"
                ? { command: "npm test -- auth" }
                : { query: "common JWT security pitfalls" },
          });
        }, 1000 + i * 1200 + 2500);

        setTimeout(() => {
          useStore.getState().completeSubAgentToolCall(sessionId, `demo-req-${runId}-${a.id}`, `tool-${runId}-${a.id}-1`, true, "Success");
          useStore.getState().completeSubAgentToolCall(sessionId, `demo-req-${runId}-${a.id}`, `tool-${runId}-${a.id}-2`, true, "Success");
        }, 1000 + i * 1200 + 3500);

        setTimeout(() => {
          useStore.getState().completeSubAgent(sessionId, `demo-req-${runId}-${a.id}`, {
            response: a.id === "researcher"
              ? "Found 5 files with authentication patterns. Current implementation uses session-based auth in src/middleware/session.ts."
              : a.id === "coder"
                ? "Created JWT middleware in src/middleware/auth.ts with access/refresh token support. All 12 tests passing."
                : "Code review passed. No security vulnerabilities found. Recommended adding rate limiting to token refresh endpoint.",
            durationMs: 4000 + i * 800,
          });
          console.log(`[Demo] Completed: ${a.name}`);
        }, 1000 + i * 1200 + 5000);
      });

      setTimeout(() => {
        useStore.getState().updateAgentStreaming(sessionId, "\n\nAll agents have completed their tasks. Here's a summary:\n\n- **Researcher**: Analyzed existing auth patterns across 5 files\n- **Coder**: Implemented JWT middleware with access/refresh tokens (12 tests passing)\n- **Reviewer**: Security review passed, suggested rate limiting for token refresh\n\nThe JWT authentication is now integrated into your API endpoints.");
        useStore.getState().setAgentResponding(sessionId, false);
      }, 9000);
    });
  };
}
