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

import {
  dispatchMockEvent,
  mockEventListeners,
  mockRegisterListener,
  mockUnregisterListener,
} from "./mocks/event-bus";
import {
  type AiEventType,
  emitAiEvent,
  emitCommandBlockEvent,
  emitTerminalOutput,
  simulateCommand,
} from "./mocks/events";
import {
  type MockCodebase,
  mockApprovalPatterns,
  mockProjectSettings,
  mockPrompts,
  mockSessions,
  mockSkills,
  mockSubAgents,
  mockTools,
  mockWorkflows,
} from "./mocks/fixtures";
import {
  mockCommandBlock,
  mockFullPlanExecution,
  mockPipelineProgressBlock,
  mockPlanPipeline,
  mockRunCommandApproval,
  mockShowAllBlocks,
  mockSubAgentBlocks,
  mockToolExecutionBlocks,
  simulatePipelineFanOut,
} from "./mocks/showcase";
import {
  simulateAiResponse,
  simulateAiResponseWithSubAgent,
  simulateJsHarvest,
} from "./mocks/simulations";

// =============================================================================
// Browser Mode Flag
// =============================================================================

// Re-export isMockBrowserMode from the isolated module for backwards compatibility.
// New code should import directly from "@/lib/isMockBrowser" to avoid pulling
// in this entire 1800+ line file into the bundle.
export { isMockBrowserMode } from "./lib/isMockBrowser";

// Event system + emit helpers moved to ./mocks/event-bus and ./mocks/events.
// Re-export here to preserve the historical "@/mocks" public surface.
export {
  mockRegisterListener,
  mockUnregisterListener,
  emitAiEvent,
  emitCommandBlockEvent,
  emitTerminalOutput,
  simulateCommand,
};
export type { AiEventType };
export type {
  CommandBlockEvent,
  DirectoryChangedEvent,
  SessionEndedEvent,
  TerminalOutputEvent,
} from "./mocks/events";
export { emitCommandBlock, emitDirectoryChanged, emitSessionEnded } from "./mocks/events";
export { simulateAiResponse, simulateAiResponseWithSubAgent, simulateJsHarvest };
export { simulateSubAgent } from "./mocks/simulations";
export {
  mockCommandBlock,
  mockFullPlanExecution,
  mockPipelineProgressBlock,
  mockPlanPipeline,
  mockRunCommandApproval,
  mockShowAllBlocks,
  mockSubAgentBlocks,
  mockToolExecutionBlocks,
  simulatePipelineFanOut,
};

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

// Mock HITL config
let mockHitlConfig = {
  always_allow: ["read_file"],
  always_require_approval: ["run_command"],
  pattern_learning_enabled: true,
  min_approvals: 3,
  approval_threshold: 0.8,
};

// Mock indexer state
let mockIndexerInitialized = false;
let mockIndexerWorkspace: string | null = null;
let mockIndexedFileCount = 0;

// Mock codebases state
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
    deepseek: {
      api_key: null,
      base_url: null,
      show_in_selector: true,
    },
    xiaomi: {
      api_key: null,
      region: null,
      default_protocol: null,
      openai_base_url: null,
      anthropic_base_url: null,
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
    | "nvidia"
    | "deepseek"
    | "xiaomi",
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

/**
 * All-in-one showcase: triggers EVERY visual component inside AIChatPanel.
 * Call from console: __demoAllChatStyles()
 *
 * Components triggered:
 *  1. MessageBlock (user + assistant with markdown + thinking)
 *  2. TaskPlanCard (active + retired iterations)
 *  3. ToolCallSummary / ToolCallCard (completed tools in message)
 *  5. SubAgentInlineCard (sub_agent_* tool in message)
 *  6. WorkflowProgress
 *  7. CompactionNotice
 *  8. AskHumanInline
 *  9. CollapsibleToolCall (pending approval)
 * 10. PlanUpdatedNotice
 */
export async function demoAllChatStyles(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

  const convId = state.activeConversationId;
  const conv = convId ? state.conversations[convId] : null;
  const aiSessionId = conv?.aiSessionId;
  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];

  if (!aiSessionId || !sessionId) {
    console.error("[demoAllChatStyles] No active conversation. Send a message first.");
    return;
  }

  const emit = (event: Record<string, unknown>) =>
    dispatchMockEvent("ai-event", { ...event, session_id: aiSessionId });
  const reqId = () => `mock-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

  console.log("[demoAllChatStyles] Starting comprehensive style showcase...");

  // === 1. AI turn with thinking + plan v1 + tools ===
  const turnId1 = `turn-${Date.now()}`;
  await emit({ type: "started", turn_id: turnId1 });
  await delay(200);

  // Thinking/reasoning
  await emit({
    type: "reasoning",
    content:
      "Let me analyze the codebase structure first. I need to understand the authentication patterns before making changes. The current implementation uses session-based auth, but the user wants JWT...",
  });
  await delay(200);

  // Text
  const text1 = "I'll create a plan and start working on it.\n\n";
  await emit({ type: "text_delta", delta: text1, accumulated: text1 });
  await delay(200);

  // Plan v1
  const planReq1 = reqId();
  await emit({
    type: "tool_request",
    tool_name: "update_plan",
    args: { explanation: "JWT migration" },
    request_id: planReq1,
  });
  await emit({
    type: "tool_result",
    tool_name: "update_plan",
    result: "ok",
    success: true,
    request_id: planReq1,
  });
  await emit({
    type: "plan_updated",
    version: 1,
    explanation: "JWT migration plan",
    steps: [
      { id: "step-auth-analyze", step: "Analyze current auth patterns", status: "completed" },
      { id: "step-jwt-middleware", step: "Create JWT middleware", status: "completed" },
      { id: "step-api-routes", step: "Update API routes", status: "completed" },
    ],
    summary: { total: 3, completed: 3, in_progress: 0, pending: 0 },
  });
  await delay(300);

  // Tool calls (read_file + run_command)
  const toolReq1 = reqId();
  await emit({
    type: "tool_request",
    tool_name: "read_file",
    args: { path: "src/middleware/session.ts" },
    request_id: toolReq1,
  });
  await delay(100);
  await emit({
    type: "tool_result",
    tool_name: "read_file",
    result: "export function sessionAuth() { ... }",
    success: true,
    request_id: toolReq1,
  });

  const toolReq2 = reqId();
  await emit({
    type: "tool_request",
    tool_name: "run_command",
    args: { command: "npm test -- auth" },
    request_id: toolReq2,
  });
  await delay(100);
  await emit({
    type: "tool_result",
    tool_name: "run_command",
    result: { stdout: "12 tests passed", exit_code: 0 },
    success: true,
    request_id: toolReq2,
  });

  // More text
  const text2 = "\n\nAll auth tests pass. Now updating the plan with additional steps.\n\n";
  await emit({ type: "text_delta", delta: text2, accumulated: text1 + text2 });
  await delay(200);

  // Plan v2 (triggers v1 retirement)
  const planReq2 = reqId();
  await emit({
    type: "tool_request",
    tool_name: "update_plan",
    args: { explanation: "Extended plan" },
    request_id: planReq2,
  });
  await emit({
    type: "tool_result",
    tool_name: "update_plan",
    result: "ok",
    success: true,
    request_id: planReq2,
  });
  await emit({
    type: "plan_updated",
    version: 2,
    explanation: "Extended JWT plan",
    steps: [
      { id: "step-auth-analyze", step: "Analyze current auth patterns", status: "completed" },
      { id: "step-jwt-middleware", step: "Create JWT middleware", status: "completed" },
      { id: "step-api-routes", step: "Update API routes", status: "in_progress" },
      { id: "step-refresh-token", step: "Add refresh token logic", status: "pending" },
      { id: "step-security-audit", step: "Security audit", status: "pending" },
    ],
    summary: { total: 5, completed: 2, in_progress: 1, pending: 2 },
  });
  await delay(200);

  // Sub-agent tool call (shows SubAgentInlineCard in message)
  const subReq = reqId();
  await emit({
    type: "tool_request",
    tool_name: "sub_agent_researcher",
    args: { task: "Search for JWT best practices" },
    request_id: subReq,
  });

  await emit({
    type: "sub_agent_started",
    agent_id: "researcher-demo",
    agent_name: "Researcher",
    task: "Search for JWT best practices",
    depth: 1,
    parent_request_id: subReq,
  });
  await delay(300);
  await emit({
    type: "sub_agent_completed",
    agent_id: "researcher-demo",
    response: "Found 3 relevant patterns for JWT refresh.",
    duration_ms: 2400,
    parent_request_id: subReq,
  });
  await emit({
    type: "tool_result",
    tool_name: "sub_agent_researcher",
    result: "Found patterns",
    success: true,
    request_id: subReq,
  });

  // Complete turn 1
  await emit({
    type: "completed",
    response: `${text1 + text2}Research complete. Starting implementation phase.`,
    input_tokens: 4200,
    output_tokens: 1800,
    duration_ms: 12000,
  });
  await delay(500);

  // === 2. Workflow ===
  const wfId = `wf-${Date.now()}`;
  await emit({
    type: "workflow_started",
    workflow_id: wfId,
    workflow_name: "JWT Migration Pipeline",
  });
  await delay(300);
  await emit({
    type: "workflow_step_started",
    workflow_id: wfId,
    step_name: "Generate middleware",
    step_index: 0,
    total_steps: 3,
  });
  await delay(200);
  await emit({
    type: "workflow_step_completed",
    workflow_id: wfId,
    step_name: "Generate middleware",
    output: "Created auth.ts",
    duration_ms: 1200,
  });
  await emit({
    type: "workflow_step_started",
    workflow_id: wfId,
    step_name: "Run tests",
    step_index: 1,
    total_steps: 3,
  });
  await delay(200);

  // === 3. Compaction notice ===
  await emit({ type: "compaction_started", tokens_before: 128000, messages_before: 42 });
  await delay(500);
  await emit({
    type: "compaction_completed",
    tokens_before: 128000,
    messages_before: 42,
    messages_after: 8,
    summary_length: 2400,
  });

  // === 4. Ask Human ===
  await delay(300);
  await emit({
    type: "ask_human_request",
    request_id: `ask-${Date.now()}`,
    question:
      "The target https://api.example.com is not registered. Do you want to add it before scanning?",
    input_type: "confirmation",
    options: [],
    context: "Required before running vulnerability scan on unregistered targets.",
  });

  // === 5. Second turn with pending tool approval ===
  await delay(500);
  const turnId2 = `turn2-${Date.now()}`;
  await emit({ type: "started", turn_id: turnId2 });
  await delay(200);

  const approvalReq = reqId();
  await emit({
    type: "tool_approval_request",
    request_id: approvalReq,
    tool_name: "run_command",
    args: { command: "rm -rf /tmp/old-auth-cache && npm run build" },
    risk_level: "high",
    stats: null,
    can_learn: true,
    suggestion: null,
  });

  console.log(
    "[demoAllChatStyles] Done! Components shown:\n" +
      "  1. MessageBlock (user+assistant)\n" +
      "  2. ThinkingBlock (reasoning)\n" +
      "  3. TaskPlanCard (active v2 + retired v1)\n" +
      "  4. ToolCallCard (read_file, run_command)\n" +
      "  5. SubAgentInlineCard (sub_agent_researcher)\n" +
      "  6. WorkflowProgress (running)\n" +
      "  7. CompactionNotice\n" +
      "  8. AskHumanInline (confirmation)\n" +
      "  9. CollapsibleToolCall (pending approval)\n"
  );
}

/**
 * Showcase SubAgent styling: nested depth, interrupted state, various statuses.
 * Call from console: __demoSubAgentStyles()
 */
export async function demoSubAgentStyleShowcase(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];
  if (!sessionId) {
    console.error("[demoSubAgentStyles] No active session");
    return;
  }
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

  const _now = new Date().toISOString();
  void _now;

  useStore.getState().setAgentResponding(sessionId, true);
  useStore
    .getState()
    .updateAgentStreaming(sessionId, "Coordinating multiple agents with nesting...\n\n");

  // Agent 1: Researcher (depth 1, completed)
  useStore.getState().startSubAgent(sessionId, {
    agentId: "researcher-001",
    agentName: "Researcher",
    parentRequestId: "demo-style-researcher",
    task: "Analyze authentication patterns across the codebase",
    depth: 1,
  });
  await delay(300);
  useStore.getState().addSubAgentToolCall(sessionId, "demo-style-researcher", {
    id: "t-r-1",
    name: "semantic_search",
    args: { query: "JWT auth middleware" },
  });
  await delay(500);
  useStore
    .getState()
    .completeSubAgentToolCall(
      sessionId,
      "demo-style-researcher",
      "t-r-1",
      true,
      "Found 5 relevant files"
    );
  useStore.getState().completeSubAgent(sessionId, "demo-style-researcher", {
    response: "Found JWT patterns in 5 files. Recommending middleware refactor.",
    durationMs: 3200,
  });

  // Agent 2: Coder (depth 1, running with nested child)
  useStore.getState().startSubAgent(sessionId, {
    agentId: "coder-001",
    agentName: "Coder",
    parentRequestId: "demo-style-coder",
    task: "Implement JWT middleware with refresh token logic",
    depth: 1,
  });
  await delay(300);
  useStore.getState().addSubAgentToolCall(sessionId, "demo-style-coder", {
    id: "t-c-1",
    name: "write_file",
    args: { path: "src/middleware/auth.ts", content: "..." },
  });

  // Agent 2a: Sub-coder (depth 2, completed — nested under Coder)
  useStore.getState().startSubAgent(sessionId, {
    agentId: "coder-sub-001",
    agentName: "Coder",
    parentRequestId: "demo-style-coder-sub",
    task: "Generate unit tests for auth middleware",
    depth: 2,
  });
  await delay(400);
  useStore.getState().addSubAgentToolCall(sessionId, "demo-style-coder-sub", {
    id: "t-cs-1",
    name: "write_file",
    args: { path: "src/middleware/__tests__/auth.test.ts" },
  });
  useStore
    .getState()
    .completeSubAgentToolCall(
      sessionId,
      "demo-style-coder-sub",
      "t-cs-1",
      true,
      "12 tests created"
    );
  useStore.getState().completeSubAgent(sessionId, "demo-style-coder-sub", {
    response: "Created 12 unit tests. All passing.",
    durationMs: 2100,
  });

  // Agent 2b: Deep nested (depth 3, running)
  await delay(200);
  useStore.getState().startSubAgent(sessionId, {
    agentId: "explorer-deep-001",
    agentName: "Explorer",
    parentRequestId: "demo-style-explorer-deep",
    task: "Scan integration test coverage for auth module",
    depth: 3,
  });
  useStore.getState().addSubAgentToolCall(sessionId, "demo-style-explorer-deep", {
    id: "t-ed-1",
    name: "list_files",
    args: { pattern: "**/*.integration.test.ts" },
  });

  // Agent 3: Reviewer (depth 1, interrupted)
  useStore.getState().startSubAgent(sessionId, {
    agentId: "reviewer-001",
    agentName: "Reviewer",
    parentRequestId: "demo-style-reviewer",
    task: "Security review of auth implementation",
    depth: 1,
  });
  await delay(300);
  useStore.getState().addSubAgentToolCall(sessionId, "demo-style-reviewer", {
    id: "t-rv-1",
    name: "read_file",
    args: { path: "src/middleware/auth.ts" },
  });
  // Mark as interrupted via direct state mutation
  useStore.setState((s) => {
    const agents = s.activeSubAgents[sessionId];
    if (agents) {
      const reviewer = agents.find((a) => a.parentRequestId === "demo-style-reviewer");
      if (reviewer) {
        reviewer.status = "interrupted";
        reviewer.toolCalls.forEach((tc) => {
          if (tc.status === "running") tc.status = "error";
        });
      }
    }
    const timeline = s.timelines[sessionId];
    if (timeline) {
      const block = timeline.find(
        (b) => b.type === "sub_agent_activity" && b.data.parentRequestId === "demo-style-reviewer"
      );
      if (block && block.type === "sub_agent_activity") {
        (block.data as { status: string }).status = "interrupted";
      }
    }
  });

  // Agent 4: Pentester (depth 1, error)
  useStore.getState().startSubAgent(sessionId, {
    agentId: "pentester-001",
    agentName: "Pentester",
    parentRequestId: "demo-style-pentester",
    task: "Scan for OWASP vulnerabilities in auth endpoints",
    depth: 1,
  });
  await delay(200);
  useStore.getState().addSubAgentToolCall(sessionId, "demo-style-pentester", {
    id: "t-p-1",
    name: "run_pty_cmd",
    args: { command: "nuclei -t cves/ -u https://api.example.com/auth" },
  });
  useStore
    .getState()
    .failSubAgent(
      sessionId,
      "demo-style-pentester",
      "ETIMEDOUT: Target unreachable after 5 retries"
    );

  console.log(
    "[demoSubAgentStyles] Injected 6 agents: completed(d1), running(d1)+completed(d2)+running(d3), interrupted(d1), error(d1)"
  );
}

/**
 * Showcase TaskPlan styling with retired plan iterations.
 * Call from console: __demoTaskPlanStyles()
 */
export async function demoTaskPlanStyleShowcase(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];
  if (!sessionId) {
    console.error("[demoTaskPlanStyles] No active session");
    return;
  }
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

  useStore.getState().setExecutionMode(sessionId, "task");

  // Version 1 plan (will become retired)
  useStore.getState().setPlan(
    sessionId,
    {
      version: 1,
      explanation: "Initial reconnaissance plan",
      summary: { total: 3, completed: 3, in_progress: 0, pending: 0 },
      steps: [
        { id: "step-dns", step: "DNS lookup and subdomain enumeration", status: "completed" },
        { id: "step-http", step: "HTTP probing on discovered hosts", status: "completed" },
        { id: "step-tech", step: "Technology fingerprinting", status: "completed" },
      ],
      updated_at: new Date(Date.now() - 480000).toISOString(),
    },
    "msg-plan-v1"
  );
  await delay(600);

  // Version 2 (triggers retire of v1, then this becomes retired)
  useStore.getState().setPlan(
    sessionId,
    {
      version: 2,
      explanation: "Extended plan: adding port scan and JS harvest",
      summary: { total: 5, completed: 2, in_progress: 0, pending: 3 },
      steps: [
        { id: "step-dns", step: "DNS lookup and subdomain enumeration", status: "completed" },
        { id: "step-http", step: "HTTP probing on discovered hosts", status: "completed" },
        { id: "step-port", step: "Port scan top-1000 ports", status: "cancelled" },
        { id: "step-js", step: "JavaScript file harvest", status: "pending" },
        { id: "step-api", step: "API endpoint extraction", status: "pending" },
      ],
      updated_at: new Date(Date.now() - 180000).toISOString(),
    },
    "msg-plan-v2"
  );
  await delay(600);

  // Version 3 (current active) — triggers retire of v2
  useStore.getState().setPlan(
    sessionId,
    {
      version: 3,
      explanation: "Final plan: focused JS analysis + vulnerability scan",
      summary: { total: 4, completed: 1, in_progress: 1, pending: 2 },
      steps: [
        { id: "step-harvest", step: "Harvest all JS assets from target", status: "completed" },
        { id: "step-analyze", step: "Analyze JS for API keys and secrets", status: "in_progress" },
        { id: "step-map", step: "Map API endpoints from JS routes", status: "pending" },
        { id: "step-vuln", step: "Automated vulnerability scan", status: "pending" },
      ],
      updated_at: new Date().toISOString(),
    },
    "msg-plan-v3"
  );

  console.log("[demoTaskPlanStyles] Injected 3 plan versions (2 retired + 1 active)");
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
    (
      window as unknown as {
        __mockShowAllBlocks?: typeof mockShowAllBlocks;
        __mockCommandBlock?: typeof mockCommandBlock;
        __mockPipelineBlock?: typeof mockPipelineProgressBlock;
        __mockSubAgentBlocks?: typeof mockSubAgentBlocks;
        __mockToolExecutionBlocks?: typeof mockToolExecutionBlocks;
        __mockPlanPipeline?: typeof mockPlanPipeline;
      }
    ).__mockShowAllBlocks = mockShowAllBlocks;
    (window as unknown as { __mockCommandBlock?: typeof mockCommandBlock }).__mockCommandBlock =
      mockCommandBlock;
    (
      window as unknown as { __mockPipelineBlock?: typeof mockPipelineProgressBlock }
    ).__mockPipelineBlock = mockPipelineProgressBlock;
    (window as unknown as { __mockPlanPipeline?: typeof mockPlanPipeline }).__mockPlanPipeline =
      mockPlanPipeline;
    (
      window as unknown as { __mockSubAgentBlocks?: typeof mockSubAgentBlocks }
    ).__mockSubAgentBlocks = mockSubAgentBlocks;
    (
      window as unknown as { __mockToolExecutionBlocks?: typeof mockToolExecutionBlocks }
    ).__mockToolExecutionBlocks = mockToolExecutionBlocks;
    (window as unknown as { __mockFullPlan?: typeof mockFullPlanExecution }).__mockFullPlan =
      mockFullPlanExecution;
    (window as unknown as { __mockRunCommand?: typeof mockRunCommandApproval }).__mockRunCommand =
      mockRunCommandApproval;
    (
      window as unknown as { __demoSubAgentStyles?: typeof demoSubAgentStyleShowcase }
    ).__demoSubAgentStyles = demoSubAgentStyleShowcase;
    (
      window as unknown as { __demoTaskPlanStyles?: typeof demoTaskPlanStyleShowcase }
    ).__demoTaskPlanStyles = demoTaskPlanStyleShowcase;
    (window as unknown as { __demoAllChatStyles?: typeof demoAllChatStyles }).__demoAllChatStyles =
      demoAllChatStyles;

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
        if (
          promptLower.includes("js") ||
          promptLower.includes("javascript") ||
          promptLower.includes("analyze")
        ) {
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
      case "wiki_list":
        return [];
      case "pipeline_list":
        return [
          {
            id: "recon-basic",
            name: "Basic Reconnaissance",
            description: "DNS, subdomains, HTTP probe, ports, tech detection, JS harvest",
            steps: [
              {
                id: "dns_lookup",
                command_template: "dig +short {target}",
                tool_name: "dig",
                args: [],
              },
              {
                id: "subdomain_enum",
                command_template: "subfinder -d {target} -silent",
                tool_name: "subfinder",
                args: [],
              },
              {
                id: "http_probe",
                command_template: "echo {target} | httpx -silent",
                tool_name: "httpx",
                args: [],
              },
              {
                id: "port_scan",
                command_template: "nmap -sV -T4 {target}",
                tool_name: "nmap",
                args: [],
              },
              {
                id: "tech_detect",
                command_template: "whatweb {target}",
                tool_name: "whatweb",
                args: [],
              },
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
      const getConv = () => (convId ? useStore.getState().conversations[convId] : null);

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
          {
            id: "researcher",
            name: "Researcher",
            task: "Searching codebase for authentication patterns and JWT usage",
          },
          {
            id: "coder",
            name: "Coder",
            task: "Implementing JWT token validation middleware with refresh logic",
          },
          {
            id: "reviewer",
            name: "Reviewer",
            task: "Reviewing code changes for security vulnerabilities",
          },
        ],
        [
          {
            id: "analyst",
            name: "Analyst",
            task: "Analyzing API endpoint performance bottlenecks",
          },
          {
            id: "coder",
            name: "Coder",
            task: "Optimizing database queries and adding connection pooling",
          },
        ],
        [
          {
            id: "researcher",
            name: "Researcher",
            task: "Investigating memory leak in worker threads",
          },
          { id: "coder", name: "Coder", task: "Fixing resource cleanup in async handlers" },
          {
            id: "explorer",
            name: "Explorer",
            task: "Scanning for similar patterns in related modules",
          },
          { id: "reviewer", name: "Reviewer", task: "Verifying fix doesn't introduce regressions" },
        ],
      ];
      const agents = taskSets[(runId - 1) % taskSets.length];

      const tasks = [
        "Help me add JWT authentication to the API endpoints",
        "Optimize the database layer for better performance",
        "Fix the memory leak in the worker pool",
      ];
      console.log(
        `[Demo] Run #${runId} - Starting ${agents.length} sub-agents for session:`,
        sessionId
      );

      addUserMsg(tasks[(runId - 1) % tasks.length]);

      setTimeout(() => {
        useStore
          .getState()
          .updateAgentStreaming(
            sessionId,
            "I'll help you implement JWT authentication. Let me coordinate multiple agents to handle this efficiently.\n\n"
          );
        useStore.getState().setAgentResponding(sessionId, true);
      }, 500);

      agents.forEach((a, i) => {
        setTimeout(
          () => {
            useStore.getState().startSubAgent(sessionId, {
              agentId: a.id,
              agentName: a.name,
              parentRequestId: `demo-req-${runId}-${a.id}`,
              task: a.task,
              depth: 1,
            });
            console.log(`[Demo] Started: ${a.name}`);
          },
          1000 + i * 1200
        );

        setTimeout(
          () => {
            useStore.getState().addSubAgentToolCall(sessionId, `demo-req-${runId}-${a.id}`, {
              id: `tool-${runId}-${a.id}-1`,
              name:
                a.id === "researcher"
                  ? "semantic_search"
                  : a.id === "coder"
                    ? "write_file"
                    : "read_file",
              args:
                a.id === "researcher"
                  ? { query: "JWT authentication middleware patterns" }
                  : a.id === "coder"
                    ? { path: "src/middleware/auth.ts", content: "..." }
                    : { path: "src/middleware/auth.ts" },
            });
          },
          1000 + i * 1200 + 1500
        );

        setTimeout(
          () => {
            useStore.getState().addSubAgentToolCall(sessionId, `demo-req-${runId}-${a.id}`, {
              id: `tool-${runId}-${a.id}-2`,
              name:
                a.id === "researcher"
                  ? "read_file"
                  : a.id === "coder"
                    ? "run_command"
                    : "semantic_search",
              args:
                a.id === "researcher"
                  ? { path: "src/config/auth.ts" }
                  : a.id === "coder"
                    ? { command: "npm test -- auth" }
                    : { query: "common JWT security pitfalls" },
            });
          },
          1000 + i * 1200 + 2500
        );

        setTimeout(
          () => {
            useStore
              .getState()
              .completeSubAgentToolCall(
                sessionId,
                `demo-req-${runId}-${a.id}`,
                `tool-${runId}-${a.id}-1`,
                true,
                "Success"
              );
            useStore
              .getState()
              .completeSubAgentToolCall(
                sessionId,
                `demo-req-${runId}-${a.id}`,
                `tool-${runId}-${a.id}-2`,
                true,
                "Success"
              );
          },
          1000 + i * 1200 + 3500
        );

        setTimeout(
          () => {
            useStore.getState().completeSubAgent(sessionId, `demo-req-${runId}-${a.id}`, {
              response:
                a.id === "researcher"
                  ? "Found 5 files with authentication patterns. Current implementation uses session-based auth in src/middleware/session.ts."
                  : a.id === "coder"
                    ? "Created JWT middleware in src/middleware/auth.ts with access/refresh token support. All 12 tests passing."
                    : "Code review passed. No security vulnerabilities found. Recommended adding rate limiting to token refresh endpoint.",
              durationMs: 4000 + i * 800,
            });
            console.log(`[Demo] Completed: ${a.name}`);
          },
          1000 + i * 1200 + 5000
        );
      });

      setTimeout(() => {
        useStore
          .getState()
          .updateAgentStreaming(
            sessionId,
            "\n\nAll agents have completed their tasks. Here's a summary:\n\n- **Researcher**: Analyzed existing auth patterns across 5 files\n- **Coder**: Implemented JWT middleware with access/refresh tokens (12 tests passing)\n- **Reviewer**: Security review passed, suggested rate limiting for token refresh\n\nThe JWT authentication is now integrated into your API endpoints."
          );
        useStore.getState().setAgentResponding(sessionId, false);
      }, 9000);
    });
  };
}
