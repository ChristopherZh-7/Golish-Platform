/**
 * Timeline block showcase + full-flow walkthroughs. These inject blocks
 * directly into the store (`useStore`) and/or drive `ai-event` sequences via
 * [`dispatchMockEvent`] for visual QA. Exposed on `window.__mock*` by
 * `setupMocks` so they can be triggered from the browser console / e2e.
 */

import { dispatchMockEvent } from "./event-bus";
import type { AiEventType } from "./events";

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
  if (!sessionId) {
    console.error("[mockCommandBlock] No active session");
    return;
  }

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
 * 3/4 — Sub-Agent Activity Block (group of 2)
 * Injects two SubAgent cards that appear as a grouped SubAgentGroup.
 */
export async function mockSubAgentBlocks(): Promise<void> {
  const { useStore } = await import("@/store/index");
  const state = useStore.getState();
  const sessionId = state.activeSessionId ?? Object.keys(state.sessions)[0];
  if (!sessionId) {
    console.error("[mockSubAgentBlocks] No active session");
    return;
  }

  const now = new Date().toISOString();
  const batchId = `batch-mock-${Date.now()}`;

  useStore.setState((s) => {
    if (!s.timelines[sessionId]) s.timelines[sessionId] = [];

    // Sub-agent 1: Pentester JS collection (completed)
    s.timelines[sessionId].push({
      id: `mock-sa-harvester-${Date.now()}`,
      type: "sub_agent_activity",
      timestamp: now,
      batchId,
      data: {
        agentId: "pentester_js_001",
        agentName: "Pentester",
        parentRequestId: `mock-parent-req-${Date.now()}`,
        task: "Collect ALL JS files from https://admin.target.example.com",
        depth: 1,
        status: "completed",
        toolCalls: [
          {
            id: "tc1",
            name: "run_pty_cmd",
            args: { command: "curl -sL https://admin.target.example.com/" },
            status: "completed",
            result: "<html>...</html>",
            startedAt: now,
            completedAt: now,
          },
          {
            id: "tc2",
            name: "write_file",
            args: { path: ".golish/js-assets/manifest.json" },
            status: "completed",
            result: "Written 12KB",
            startedAt: now,
            completedAt: now,
          },
          {
            id: "tc3",
            name: "run_pty_cmd",
            args: { command: "bash collect.sh https://admin.target.example.com/assets" },
            status: "completed",
            result: "TOTAL: 42 files collected (1.8MB)",
            startedAt: now,
            completedAt: now,
          },
        ],
        entries: [
          { kind: "text", text: "Starting JS collection from target..." },
          { kind: "tool_call", toolCallId: "tc1" },
          { kind: "text", text: "Found Vite manifest, extracting asset list..." },
          { kind: "tool_call", toolCallId: "tc2" },
          { kind: "tool_call", toolCallId: "tc3" },
        ],
        response:
          "Collection complete: 42 JS files (1.8MB) + 3 source maps. Strategy: manifest-based (Vite detected).",
        startedAt: new Date(Date.now() - 8500).toISOString(),
        completedAt: now,
        durationMs: 8500,
      },
    });

    // Sub-agent 2: Pentester JS analysis (running)
    s.timelines[sessionId].push({
      id: `mock-sa-analyzer-${Date.now() + 1}`,
      type: "sub_agent_activity",
      timestamp: new Date(Date.now() + 10).toISOString(),
      batchId,
      data: {
        agentId: "pentester_analysis_001",
        agentName: "Pentester",
        parentRequestId: `mock-parent-req-${Date.now() + 1}`,
        task: "Security analysis of 42 collected JS files from admin.target.example.com",
        depth: 1,
        status: "running",
        toolCalls: [
          {
            id: "tc4",
            name: "list_files",
            args: { pattern: ".golish/js-assets/**/*.js" },
            status: "completed",
            result: "42 files found",
            startedAt: now,
            completedAt: now,
          },
          {
            id: "tc5",
            name: "grep_file",
            args: { pattern: "api[_-]?key|secret|token", path: ".golish/js-assets/" },
            status: "running",
            startedAt: now,
          },
        ],
        entries: [
          { kind: "text", text: "Listing all collected JS files..." },
          { kind: "tool_call", toolCallId: "tc4" },
          {
            kind: "text",
            text: "Scanning for API endpoints and hardcoded secrets across 42 files...",
          },
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
  if (!sessionId) {
    console.error("[mockToolExecutionBlocks] No active session");
    return;
  }

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
        result:
          "https://api.target.example.com [200] [API Gateway]\nhttps://admin.target.example.com [200] [Admin Panel]\nhttps://staging.target.example.com [403] [Forbidden]",
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
        result:
          '{"entries":{"main.js":"assets/main-a1b2c3.js","vendor.js":"assets/vendor-d4e5f6.js"}}',
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
        args: {
          file_path: "src/config/targets.json",
          changes: "Add admin.target.example.com to scope",
        },
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
  console.log(
    "[mockToolExecutionBlocks] Injected 4 tool execution blocks (completed, read, running, error)"
  );
}

/**
 * Showcase all timeline block types at once.
 * Call from console: __mockShowAllBlocks()
 */
export async function mockShowAllBlocks(): Promise<void> {
  await mockCommandBlock();
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

  console.log(
    "[mockFullPlan] Starting with AI session:",
    aiSessionId,
    "terminal:",
    terminalSessionId
  );

  // Helper to emit AI event with proper session_id
  const emit = (event: AiEventType) =>
    dispatchMockEvent("ai-event", { ...event, session_id: aiSessionId });

  const turnId = `mock-plan-${Date.now()}`;
  const reqId = (name: string) =>
    `mock-${name}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

  // --- 1. AI turn starts ---
  await emit({ type: "started", turn_id: turnId });
  await delay(300);

  // --- 2. AI sends planning text ---
  const planText = "I'll create a task plan and execute each step.\n\n";
  await emit({ type: "text_delta", delta: planText, accumulated: planText });
  await delay(400);

  // --- 3. AI calls update_plan to create the plan (step 1 in_progress) ---
  const planReqId1 = reqId("update-plan-1");
  await emit({
    type: "tool_request",
    tool_name: "update_plan",
    args: {
      explanation: "System reconnaissance",
      steps: [
        { step: "Check system information", status: "in_progress" },
        { step: "List files in workspace", status: "pending" },
        { step: "Show network configuration", status: "pending" },
      ],
    },
    request_id: planReqId1,
  });
  await delay(200);

  // plan_updated event (emitted by the backend when update_plan runs)
  dispatchMockEvent("ai-event", {
    type: "plan_updated",
    session_id: aiSessionId,
    version: 1,
    explanation: "System reconnaissance",
    steps: [
      { id: "step-sysinfo", step: "Check system information", status: "in_progress" },
      { id: "step-listfiles", step: "List files in workspace", status: "pending" },
      { id: "step-network", step: "Show network configuration", status: "pending" },
    ],
    summary: { total: 3, completed: 0, in_progress: 1, pending: 2 },
  });
  await delay(100);

  await emit({
    type: "tool_result",
    tool_name: "update_plan",
    result: "Plan created with 3 steps",
    success: true,
    request_id: planReqId1,
  });
  await delay(300);

  // --- 4. Step 1: run_command uname -a ---
  const cmdReqId1 = reqId("run-cmd-1");
  await emit({
    type: "tool_request",
    tool_name: "run_command",
    args: { command: "uname -a && sw_vers" },
    request_id: cmdReqId1,
  });
  await delay(500);

  // Streaming output
  dispatchMockEvent("ai-event", {
    type: "tool_output_chunk",
    session_id: aiSessionId,
    request_id: cmdReqId1,
    tool_name: "run_command",
    chunk: "Darwin MacBook-Pro.local 24.4.0 Darwin Kernel Version 24.4.0\n",
    stream: "stdout",
  });
  await delay(300);
  dispatchMockEvent("ai-event", {
    type: "tool_output_chunk",
    session_id: aiSessionId,
    request_id: cmdReqId1,
    tool_name: "run_command",
    chunk: "ProductName:    macOS\nProductVersion: 15.4\nBuildVersion:   24E5238a\n",
    stream: "stdout",
  });
  await delay(300);

  await emit({
    type: "tool_result",
    tool_name: "run_command",
    result: "Darwin MacBook-Pro.local 24.4.0 ...",
    success: true,
    request_id: cmdReqId1,
  });
  await delay(200);

  // --- 5. Mark step 1 complete, step 2 in_progress ---
  const planReqId2 = reqId("update-plan-2");
  await emit({ type: "tool_request", tool_name: "update_plan", args: {}, request_id: planReqId2 });
  await delay(100);

  dispatchMockEvent("ai-event", {
    type: "plan_updated",
    session_id: aiSessionId,
    version: 2,
    explanation: "System reconnaissance",
    steps: [
      { id: "step-sysinfo", step: "Check system information", status: "completed" },
      { id: "step-listfiles", step: "List files in workspace", status: "in_progress" },
      { id: "step-network", step: "Show network configuration", status: "pending" },
    ],
    summary: { total: 3, completed: 1, in_progress: 1, pending: 1 },
  });
  await delay(100);

  await emit({
    type: "tool_result",
    tool_name: "update_plan",
    result: "Plan updated",
    success: true,
    request_id: planReqId2,
  });
  await delay(300);

  // --- 6. Step 2: list_files ---
  const listReqId = reqId("list-files");
  await emit({
    type: "tool_request",
    tool_name: "list_files",
    args: { path: "." },
    request_id: listReqId,
  });
  await delay(600);

  await emit({
    type: "tool_result",
    tool_name: "list_files",
    result: "backend/\nfrontend/\npackage.json\nCargo.toml\nREADME.md\njustfile\n... (196 entries)",
    success: true,
    request_id: listReqId,
  });
  await delay(200);

  // --- 7. Mark step 2 complete, step 3 in_progress ---
  const planReqId3 = reqId("update-plan-3");
  await emit({ type: "tool_request", tool_name: "update_plan", args: {}, request_id: planReqId3 });
  await delay(100);

  dispatchMockEvent("ai-event", {
    type: "plan_updated",
    session_id: aiSessionId,
    version: 3,
    explanation: "System reconnaissance",
    steps: [
      { id: "step-sysinfo", step: "Check system information", status: "completed" },
      { id: "step-listfiles", step: "List files in workspace", status: "completed" },
      { id: "step-network", step: "Show network configuration", status: "in_progress" },
    ],
    summary: { total: 3, completed: 2, in_progress: 1, pending: 0 },
  });
  await delay(100);

  await emit({
    type: "tool_result",
    tool_name: "update_plan",
    result: "Plan updated",
    success: true,
    request_id: planReqId3,
  });
  await delay(300);

  // --- 8. Step 3: run_command ifconfig ---
  const cmdReqId2 = reqId("run-cmd-2");
  await emit({
    type: "tool_request",
    tool_name: "run_command",
    args: { command: "ifconfig en0" },
    request_id: cmdReqId2,
  });
  await delay(500);

  dispatchMockEvent("ai-event", {
    type: "tool_output_chunk",
    session_id: aiSessionId,
    request_id: cmdReqId2,
    tool_name: "run_command",
    chunk:
      "en0: flags=8863<UP,BROADCAST,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\tinet 192.168.0.69 netmask 0xffffff00 broadcast 192.168.0.255\n",
    stream: "stdout",
  });
  await delay(300);

  await emit({
    type: "tool_result",
    tool_name: "run_command",
    result: "en0: flags=8863 ... inet 192.168.0.69",
    success: true,
    request_id: cmdReqId2,
  });
  await delay(200);

  // --- 9. Mark all steps complete ---
  const planReqId4 = reqId("update-plan-4");
  await emit({ type: "tool_request", tool_name: "update_plan", args: {}, request_id: planReqId4 });
  await delay(100);

  dispatchMockEvent("ai-event", {
    type: "plan_updated",
    session_id: aiSessionId,
    version: 4,
    explanation: "System reconnaissance",
    steps: [
      { id: "step-sysinfo", step: "Check system information", status: "completed" },
      { id: "step-listfiles", step: "List files in workspace", status: "completed" },
      { id: "step-network", step: "Show network configuration", status: "completed" },
    ],
    summary: { total: 3, completed: 3, in_progress: 0, pending: 0 },
  });
  await delay(100);

  await emit({
    type: "tool_result",
    tool_name: "update_plan",
    result: "All steps completed",
    success: true,
    request_id: planReqId4,
  });
  await delay(300);

  // --- 10. AI sends summary text ---
  const summary =
    "All 3 steps completed:\n\n1. **System Info**: macOS 15.4 (Darwin 24.4.0)\n2. **Files**: 196 entries in workspace (Rust + React project)\n3. **Network**: en0 active at 192.168.0.69";
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
  await emit({
    type: "completed",
    response: accumulated,
    tokens_used: 3200,
    duration_ms: 12000,
    input_tokens: 6400,
    output_tokens: 800,
  });

  console.log(
    "[mockFullPlan] Complete! Check right chat for plan card, click it to see left pane detail."
  );
}

// =============================================================================
// AI run_command Approval Demo (call from browser console)
// Call from console: __mockRunCommand()
// =============================================================================

/**
 * Simulate AI executing multiple tool calls (auto-approved) that appear as
 * compact badges in the right chat and as ToolExecutionCards in the center
 * panel. Click the badges to navigate.
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
  const reqId = (name: string) =>
    `mock-${name}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

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
    chunk:
      "Darwin MacBook-Pro.local 24.4.0 Darwin Kernel Version 24.4.0: root:xnu-11417.401.54~1/RELEASE_ARM64_T6031 arm64\n",
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
    chunk:
      "Filesystem       Size   Used  Avail Capacity  Mounted on\n/dev/disk3s1s1  460Gi  320Gi  140Gi    70%    /\n",
    stream: "stdout",
  });
  await delay(200);

  await emit({
    type: "tool_result",
    request_id: cmd2Id,
    tool_name: "run_command",
    result:
      "Filesystem       Size   Used  Avail Capacity  Mounted on\n/dev/disk3s1s1  460Gi  320Gi  140Gi    70%    /",
    success: true,
  });
  await delay(200);

  // 6. AI summary text
  const summary =
    "\n\nYour system is running macOS on Apple Silicon (arm64). Disk usage is at 70%.";
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

  console.log(
    "[mockRunCommand] Done! You should see tool badges (Shell, Read, Shell) in the right chat. Click them to open the tool detail panel in the center."
  );
}
