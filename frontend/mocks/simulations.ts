/**
 * AI streaming simulation helpers. Each drives a realistic sequence of
 * `ai-event` payloads (started → text_delta(s) → tool calls / sub-agents →
 * completed) through [`emitAiEvent`] for browser-mode / e2e development.
 */

import { emitAiEvent } from "./events";

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
 * Simulate the pentester sub-agent flow for JS collection + analysis.
 * Triggered automatically when user sends a message containing "js" or "analyze".
 */
export async function simulateJsHarvest(): Promise<void> {
  const turnId = `mock-turn-${Date.now()}`;
  const harvesterId = `mock-pentester-js-${Date.now()}`;
  const analyzerId = `mock-pentester-analysis-${Date.now()}`;
  const harvesterReqId = `mock-sub-req-harvest-${Date.now()}`;
  const analyzerReqId = `mock-sub-req-analyze-${Date.now()}`;
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const emitSubAgentTools = async (
    agentId: string,
    tools: { name: string; args: Record<string, unknown>; result: string }[]
  ) => {
    for (const tool of tools) {
      const reqId = `mock-req-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      await emitAiEvent({
        type: "sub_agent_tool_request",
        agent_id: agentId,
        tool_name: tool.name,
        args: tool.args,
        request_id: reqId,
      });
      await delay(600);
      await emitAiEvent({
        type: "sub_agent_tool_result",
        agent_id: agentId,
        tool_name: tool.name,
        result: tool.result,
        success: true,
        request_id: reqId,
      });
      await delay(300);
    }
  };

  await emitAiEvent({ type: "started", turn_id: turnId });
  await delay(200);

  await emitAiEvent({
    type: "text_delta",
    delta:
      "I'll collect all JavaScript files from example.com and then analyze them for security issues.\n\n",
    accumulated:
      "I'll collect all JavaScript files from example.com and then analyze them for security issues.\n\n",
  });
  await delay(300);

  // === Phase 1: Pentester — JS Collection ===
  await emitAiEvent({
    type: "tool_request",
    tool_name: "sub_agent_pentester",
    args: { task: "Collect ALL JS files from https://example.com" },
    request_id: harvesterReqId,
  });
  await delay(200);
  await emitAiEvent({
    type: "sub_agent_started",
    agent_id: harvesterId,
    agent_name: "Pentester",
    task: "Collect ALL JS files from https://example.com",
    depth: 1,
    parent_request_id: harvesterReqId,
  });
  await delay(300);
  await emitAiEvent({
    type: "sub_agent_text_delta",
    agent_id: harvesterId,
    delta: "Probing target for bundler type...",
    accumulated: "Probing target for bundler type...",
    parent_request_id: harvesterReqId,
  });
  await delay(200);

  await emitSubAgentTools(harvesterId, [
    {
      name: "run_pty_cmd",
      args: { command: "curl -sLk -D- https://example.com" },
      result:
        'HTTP/2 200\nserver: nginx\ncontent-type: text/html\n\n<!DOCTYPE html>...<script type="module" src="/assets/index-BjK2xfA.js">',
    },
    {
      name: "run_pty_cmd",
      args: {
        command: "curl -sLk -w '%{http_code}' -o /dev/null https://example.com/.vite/manifest.json",
      },
      result: "404",
    },
    {
      name: "run_pty_cmd",
      args: {
        command: "curl -sLk -w '%{http_code}' -o /dev/null https://example.com/asset-manifest.json",
      },
      result: "404",
    },
    {
      name: "run_pty_cmd",
      args: { command: "curl -sLk https://example.com/assets/index-BjK2xfA.js | head -5" },
      result: 'import{c as createApp}from"./chunk-framework-De4f.js";const routes=[...]',
    },
    {
      name: "write_file",
      args: {
        path: "/tmp/js_harvest.sh",
        content: "#!/bin/bash\nBASE=https://example.com/assets ...",
      },
      result: "File written: /tmp/js_harvest.sh",
    },
    {
      name: "run_pty_cmd",
      args: { command: "bash /tmp/js_harvest.sh" },
      result:
        "Downloading index-BjK2xfA.js... OK\nDownloading vendor-Ca3dR2p.js... OK\n... (50 files from main bundle)\nRecursive pass 1: found 5 new refs in system.js\nRecursive pass 2: found 3 new refs in project.js\nRecursive pass 3: 0 new files\nTOTAL: 58 files downloaded (2.8MB)",
    },
    {
      name: "run_pty_cmd",
      args: {
        command:
          'for f in .golish/js-assets/example.com/*.js; do curl -sLk -w \'%{http_code}\' -o "${f}.map" "https://example.com/assets/$(basename $f).map"; done | grep 200 | wc -l',
      },
      result: "6 source maps found",
    },
    {
      name: "write_file",
      args: { path: ".golish/js-assets/example.com/index.json", content: "{...manifest...}" },
      result: "Manifest updated: 58 files, 6 sourcemaps, 2 failed (auth_required)",
    },
  ]);

  const harvesterResponse =
    "Collection complete: 58 JS files (2.8MB) + 6 source maps. Strategy: recursive script (no manifest found). 2 files require authentication.";
  await emitAiEvent({
    type: "sub_agent_completed",
    agent_id: harvesterId,
    response: harvesterResponse,
    duration_ms: 8500,
    parent_request_id: harvesterReqId,
  });
  await delay(200);
  await emitAiEvent({
    type: "tool_result",
    tool_name: "sub_agent_pentester",
    result: harvesterResponse,
    success: true,
    request_id: harvesterReqId,
  });
  await delay(400);

  // === Phase 2: Pentester — JS Security Analysis ===
  await emitAiEvent({
    type: "tool_request",
    tool_name: "sub_agent_pentester",
    args: { task: "Analyze collected JS in .golish/js-assets/example.com/ for security issues" },
    request_id: analyzerReqId,
  });
  await delay(200);
  await emitAiEvent({
    type: "sub_agent_started",
    agent_id: analyzerId,
    agent_name: "Pentester",
    task: "Security analysis of 58 collected JS files",
    depth: 1,
    parent_request_id: analyzerReqId,
  });
  await delay(300);
  await emitAiEvent({
    type: "sub_agent_text_delta",
    agent_id: analyzerId,
    delta: "Scanning for API endpoints and secrets...",
    accumulated: "Scanning for API endpoints and secrets...",
    parent_request_id: analyzerReqId,
  });
  await delay(200);

  await emitSubAgentTools(analyzerId, [
    {
      name: "read_file",
      args: { path: ".golish/js-assets/example.com/index.json" },
      result: '{"bundler":"vite","stats":{"total_files":58,"source_maps":6}}',
    },
    {
      name: "grep_file",
      args: { pattern: "/api/v[0-9]", path: ".golish/js-assets/example.com/" },
      result: "5 API endpoints found across 4 files",
    },
    {
      name: "grep_file",
      args: {
        pattern: "(api_key|secret|token|password|pk_live)",
        path: ".golish/js-assets/example.com/",
      },
      result: "3 secrets found in vendor-Ca3dR2p.js and index-BjK2xfA.js",
    },
    {
      name: "read_file",
      args: { path: ".golish/js-assets/example.com/Debug-Qr9s.js" },
      result: "window.__DEBUG__={env:process.env,dump:()=>...} // no auth check",
    },
  ]);

  const analyzerResponse = `**API Endpoints**: 5 found (POST /auth/login, GET /users/me, POST /payments/charge, DELETE /admin/users/:id, GET /config)
**Secrets**: STRIPE_PK, API_BASE (internal URL), AWS_REGION
**Hidden Routes**: /debug (NO AUTH — env dump), /admin (DELETE endpoint)
**Vulnerable**: lodash@4.17.15, axios@0.21.0`;

  await emitAiEvent({
    type: "sub_agent_completed",
    agent_id: analyzerId,
    response: analyzerResponse,
    duration_ms: 6200,
    parent_request_id: analyzerReqId,
  });
  await delay(200);
  await emitAiEvent({
    type: "tool_result",
    tool_name: "sub_agent_pentester",
    result: analyzerResponse,
    success: true,
    request_id: analyzerReqId,
  });
  await delay(400);

  // === Final summary in main chat ===
  const finalResponse =
    "JS 收集与分析完成。\n\n**收集**: 58 个文件 (2.8MB) + 6 个 source maps, Vite bundler\n**发现**:\n1. 5 个 API endpoints (含支付、管理员删除接口)\n2. 3 个硬编码密钥 (Stripe, 内网 API, AWS)\n3. /debug 路由无认证 — 可直接访问环境变量\n4. 2 个已知漏洞依赖\n\n优先处理: /debug 环境变量泄露 + Stripe 密钥硬编码。";
  const words = finalResponse.split(" ");
  let accumulated = "";
  for (const word of words) {
    const delta = accumulated ? ` ${word}` : word;
    accumulated += delta;
    await emitAiEvent({ type: "text_delta", delta, accumulated });
    await delay(30);
  }

  await emitAiEvent({
    type: "completed",
    response: accumulated,
    tokens_used: 2400,
    duration_ms: 18000,
    input_tokens: 4800,
    output_tokens: 520,
  });
}
