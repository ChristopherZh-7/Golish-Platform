/**
 * Tool-related AI event handlers.
 *
 * Handles tool request, approval, auto-approval, and result events.
 */

import type { ApprovalPattern, RiskLevel, ToolSource } from "@/lib/ai";
import { respondToToolApproval } from "@/lib/ai";
import { logger } from "@/lib/logger";
import type { JsonValue } from "@/lib/serde_json/JsonValue";
import type { AiToolExecution } from "@/store";
import type { EventHandler, EventHandlerContext } from "./types";

type ToolIntentSource = NonNullable<AiToolExecution["toolIntent"]>["source"];
type ToolIntentDecision = NonNullable<AiToolExecution["toolIntent"]>["decision"];

const EMPTY_STAGE_RUN_SUMMARY = { total: 0, covered: 0, active: 0, queued: 0, blocked: 0 };

function normalizeToolIntentSource(source: string): ToolIntentSource {
  return source === "textual_xml" ||
    source === "textual_json" ||
    source === "recovered" ||
    source === "native_tool_call"
    ? source
    : "recovered";
}

function normalizeToolIntentDecision(decision: string): ToolIntentDecision {
  return decision === "allow" ||
    decision === "require_approval" ||
    decision === "require_human_answer" ||
    decision === "reject"
    ? decision
    : "reject";
}

function seedStageRunToolRequest(
  state: ReturnType<EventHandlerContext["getState"]>,
  sessionId: string,
  toolName: string,
  requestId: string,
  source: ReturnType<EventHandlerContext["convertToolSource"]>
) {
  const isMainTool = !source || source.type === "main";
  if (toolName !== "stage_run" || !isMainTool) return;
  state.setSessionStageRun(sessionId, {
    rows: [],
    summary: { ...EMPTY_STAGE_RUN_SUMMARY },
    stageLabel: "Stage Run",
    roleLabel: "",
    coverageAxis: [],
    requestId,
  });
}

export const handleToolIntentObservation: EventHandler<{
  type: "tool_intent_observation";
  request_id: string;
  tool_name: string;
  source: string;
  decision: string;
  reason: string | null;
  raw_preview: string | null;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  ctx.getState().recordToolIntentObservation(ctx.sessionId, {
    requestId: event.request_id,
    modelWanted: event.tool_name,
    source: normalizeToolIntentSource(event.source),
    decision: normalizeToolIntentDecision(event.decision),
    reason: event.reason ?? undefined,
    rawPreview: event.raw_preview ?? undefined,
  });
};

/**
 * Handle tool request event.
 * Adds tool call to active calls and streaming blocks.
 */
export const handleToolRequest: EventHandler<{
  type: "tool_request";
  tool_name: string;
  args: JsonValue;
  request_id: string;
  source: ToolSource;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const state = ctx.getState();

  // Deduplicate: ignore already-processed requests
  if (state.isToolRequestProcessed(ctx.sessionId, event.request_id)) {
    logger.debug("Ignoring duplicate tool_request:", event.request_id);
    return;
  }

  // Mark as processed immediately to prevent duplicates
  state.markToolRequestProcessed(ctx.sessionId, event.request_id);

  state.setAgentThinking(ctx.sessionId, false);
  // Flush pending text deltas to ensure correct ordering
  ctx.flushSessionDeltas(ctx.sessionId);

  const source = ctx.convertToolSource(event.source);
  const toolCall = {
    id: event.request_id,
    name: event.tool_name,
    args: event.args as Record<string, unknown>,
    executedByAgent: true,
    source,
  };

  // Track the tool call as running (for UI display)
  state.addActiveToolCall(ctx.sessionId, toolCall);
  // Also add to streaming blocks for interleaved display
  state.addStreamingToolBlock(ctx.sessionId, toolCall);

  // Add to left timeline as a card (main agent tool calls only).
  // Skip sub-agent invocations — they get their own SubAgentCard via sub_agent_started.
  const isSubAgentCall = event.tool_name.startsWith("sub_agent_");
  if ((!source || source.type === "main") && !isSubAgentCall) {
    state.addToolExecutionBlock(ctx.sessionId, {
      requestId: event.request_id,
      toolName: event.tool_name,
      args: event.args as Record<string, unknown>,
      source,
    });
  }
  seedStageRunToolRequest(state, ctx.sessionId, event.tool_name, event.request_id, source);
};

/**
 * Handle tool approval request event.
 * Enhanced tool request with HITL metadata requiring user approval.
 */
export const handleToolApprovalRequest: EventHandler<{
  type: "tool_approval_request";
  request_id: string;
  tool_name: string;
  args: JsonValue;
  stats: ApprovalPattern | null;
  risk_level: RiskLevel;
  can_learn: boolean;
  suggestion: string | null;
  source: ToolSource;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const state = ctx.getState();

  // Deduplicate: ignore already-processed requests
  if (state.isToolRequestProcessed(ctx.sessionId, event.request_id)) {
    logger.debug("Ignoring duplicate tool_approval_request:", event.request_id);
    return;
  }

  // Mark as processed immediately to prevent duplicates
  state.markToolRequestProcessed(ctx.sessionId, event.request_id);

  state.setAgentThinking(ctx.sessionId, false);
  // Flush pending text deltas to ensure correct ordering
  ctx.flushSessionDeltas(ctx.sessionId);

  const source = ctx.convertToolSource(event.source);
  const toolCall = {
    id: event.request_id,
    name: event.tool_name,
    args: event.args as Record<string, unknown>,
    executedByAgent: true,
    riskLevel: event.risk_level,
    stats: event.stats ?? undefined,
    suggestion: event.suggestion ?? undefined,
    canLearn: event.can_learn,
    source,
  };

  // Track the tool call
  state.addActiveToolCall(ctx.sessionId, toolCall);
  state.addStreamingToolBlock(ctx.sessionId, toolCall);

  const isSubAgentCall = event.tool_name.startsWith("sub_agent_");
  if ((!source || source.type === "main") && !isSubAgentCall) {
    state.addToolExecutionBlock(ctx.sessionId, {
      requestId: event.request_id,
      toolName: event.tool_name,
      args: event.args as Record<string, unknown>,
      riskLevel: event.risk_level,
      source,
    });
  }
  seedStageRunToolRequest(state, ctx.sessionId, event.tool_name, event.request_id, source);

  // Check if auto-approve mode is enabled for this session
  // This acts as a frontend safeguard in case the backend sent an approval request
  // before the agent mode was fully synchronized
  const session = state.sessions[ctx.sessionId];
  if (session?.agentMode === "auto-approve") {
    respondToToolApproval(ctx.sessionId, {
      request_id: event.request_id,
      approved: true,
      reason: null,
      remember: false,
      always_allow: false,
    }).catch((err) => {
      logger.error("Failed to auto-approve tool:", err);
    });
    return;
  }

  // Set pending tool approval for the dialog
  state.setPendingToolApproval(ctx.sessionId, {
    ...toolCall,
    status: "pending",
  });
};

/**
 * Handle tool auto-approved event.
 * Tool was automatically approved based on learned patterns.
 */
export const handleToolAutoApproved: EventHandler<{
  type: "tool_auto_approved";
  request_id: string;
  tool_name: string;
  args: JsonValue;
  reason: string;
  source: ToolSource;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const state = ctx.getState();

  // Deduplicate: ignore already-processed requests
  if (state.isToolRequestProcessed(ctx.sessionId, event.request_id)) {
    logger.debug("Ignoring duplicate tool_auto_approved:", event.request_id);
    return;
  }

  // Mark as processed immediately to prevent duplicates
  state.markToolRequestProcessed(ctx.sessionId, event.request_id);

  logger.info("tool_auto_approved: Adding tool block", {
    request_id: event.request_id,
    tool_name: event.tool_name,
  });

  state.setAgentThinking(ctx.sessionId, false);
  // Flush pending text deltas to ensure correct ordering
  ctx.flushSessionDeltas(ctx.sessionId);

  const source = ctx.convertToolSource(event.source);
  const autoApprovedTool = {
    id: event.request_id,
    name: event.tool_name,
    args: event.args as Record<string, unknown>,
    executedByAgent: true,
    autoApproved: true,
    autoApprovalReason: event.reason,
    source,
  };

  state.addActiveToolCall(ctx.sessionId, autoApprovedTool);
  state.addStreamingToolBlock(ctx.sessionId, autoApprovedTool);

  const isSubAgentCall = event.tool_name.startsWith("sub_agent_");
  if ((!source || source.type === "main") && !isSubAgentCall) {
    state.addToolExecutionBlock(ctx.sessionId, {
      requestId: event.request_id,
      toolName: event.tool_name,
      args: event.args as Record<string, unknown>,
      autoApproved: true,
      source,
    });
  }
  seedStageRunToolRequest(state, ctx.sessionId, event.tool_name, event.request_id, source);
};

/** A soft-timeout result whose command was detached to a background job. */
export function isBackgroundedResult(result: unknown): boolean {
  return (
    result != null &&
    typeof result === "object" &&
    (result as { status?: unknown }).status === "backgrounded"
  );
}

/** Minimal store surface needed to register a background job. */
type BackgroundJobRegistrar = {
  addBackgroundJob: (
    sessionId: string,
    job: { jobId: string; command: string; startedAt: number }
  ) => void;
};

/**
 * Register a soft-timeout→backgrounded tool result into the Cursor-style
 * background-jobs indicator. Shared by the main-agent (`handleToolResult`) and
 * sub-agent (`handleSubAgentToolResult`) paths so both surface backgrounded
 * commands in the input-row badge + sub-agent detail header. No-op when the
 * result carries no `job_id`.
 */
export function registerBackgroundJobFromResult(
  state: BackgroundJobRegistrar,
  sessionId: string,
  result: unknown
): void {
  const bg = result as { job_id?: string; command?: string };
  if (bg.job_id) {
    state.addBackgroundJob(sessionId, {
      jobId: bg.job_id,
      command: bg.command ?? "(command)",
      startedAt: Date.now(),
    });
  }
}

/**
 * Handle tool result event.
 * Updates tool call status to completed/error.
 */
export const handleToolResult: EventHandler<{
  type: "tool_result";
  tool_name: string;
  result: JsonValue;
  success: boolean;
  request_id: string;
  source: ToolSource;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const state = ctx.getState();

  // Soft-timeout → backgrounded: the command exceeded its soft timeout and was
  // detached to a background job that is still running. The agent's turn
  // continues (it received this tool_result), so drop it from the active list,
  // but keep the timeline + interleaved cards visibly "running in background"
  // until a later `tool_background_completed` flips them to a terminal result.
  if (isBackgroundedResult(event.result)) {
    registerBackgroundJobFromResult(state, ctx.sessionId, event.result);
    state.completeActiveToolCall(ctx.sessionId, event.request_id, true, event.result);
    state.backgroundStreamingToolBlock(ctx.sessionId, event.request_id, event.result);
    state.backgroundToolExecutionBlock(ctx.sessionId, event.request_id, event.result);
    return;
  }

  // Update tool call status to completed/error
  state.completeActiveToolCall(ctx.sessionId, event.request_id, event.success, event.result);
  // Also update streaming block
  state.updateStreamingToolBlock(ctx.sessionId, event.request_id, event.success, event.result);
  // Update timeline card
  state.completeToolExecutionBlock(ctx.sessionId, event.request_id, event.success, event.result);
};

/**
 * Handle tool output chunk event.
 * Appends streaming output to a running tool call (for run_command).
 */
export const handleToolOutputChunk: EventHandler<{
  type: "tool_output_chunk";
  request_id: string;
  tool_name: string;
  chunk: string;
  stream: string;
  source: ToolSource;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const state = ctx.getState();

  if (event.source?.type === "sub_agent") {
    state.appendSubAgentToolOutput(ctx.sessionId, event.request_id, event.chunk);
    return;
  }

  // Debug: Log what blocks exist and which one we're trying to match
  const blocks = state.streamingBlocks[ctx.sessionId] ?? [];
  const toolBlocks = blocks.filter((b) => b.type === "tool");
  const matchingBlock = toolBlocks.find(
    (b) => b.type === "tool" && b.toolCall.id === event.request_id
  );

  if (!matchingBlock) {
    logger.warn("tool_output_chunk: No matching block found for request_id:", event.request_id, {
      availableToolIds: toolBlocks.map((b) => (b as { toolCall: { id: string } }).toolCall.id),
    });
  } else {
    logger.debug("tool_output_chunk: Found matching block for", event.request_id);
  }

  // Append the chunk to the tool's streaming output
  state.appendToolStreamingOutput(ctx.sessionId, event.request_id, event.chunk);
  // Also append to timeline card
  state.appendToolExecutionOutput(ctx.sessionId, event.request_id, event.chunk);
};

/**
 * Handle tool_background_completed event.
 *
 * A shell/pentest command that had exceeded its soft timeout (and was moved to
 * the background) has now finished — emitted asynchronously, outside the turn
 * that started it. We correlate it back to the originating tool card via
 * `job_id` (the backgrounded `tool_result` stored a result carrying the same
 * id) and flip that card from its "backgrounded" placeholder to the terminal
 * result. If the card can't be found (e.g. timeline trimmed), this is a no-op —
 * the agent still learns the outcome on its next turn via backend re-injection.
 */
export const handleToolBackgroundCompleted: EventHandler<{
  type: "tool_background_completed";
  job_id: string;
  command: string;
  status: string;
  exit_code: number | null;
  stdout_tail: string;
  stderr_tail: string;
  duration_ms: bigint;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const state = ctx.getState();
  // Drop it from the Cursor-style "running in background" indicator.
  state.removeBackgroundJob(ctx.sessionId, event.job_id);
  const success = event.status === "done";
  const result = {
    status: event.status,
    job_id: event.job_id,
    command: event.command,
    exit_code: event.exit_code,
    stdout: event.stdout_tail,
    stderr: event.stderr_tail,
    duration_ms: Number(event.duration_ms),
    backgrounded_completed: true,
  };

  const timeline = state.timelines[ctx.sessionId] ?? [];
  const block = timeline.find(
    (b) =>
      b.type === "ai_tool_execution" &&
      (b.data.result as { job_id?: string } | undefined)?.job_id === event.job_id
  );
  if (block && block.type === "ai_tool_execution") {
    state.completeToolExecutionBlock(ctx.sessionId, block.data.requestId, success, result);
    // Flip the interleaved (in-message) tool block out of its "backgrounded"
    // state too, so both views converge on the terminal result.
    state.updateStreamingToolBlock(ctx.sessionId, block.data.requestId, success, result);
  }
};

/**
 * Handle ask_human_request event.
 * AI is requesting input from the user (barrier tool — pauses execution).
 */
export const handleAskHumanRequest: EventHandler<{
  type: "ask_human_request";
  request_id: string;
  question: string;
  input_type: string;
  options: string[];
  context: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const state = ctx.getState();

  state.setAgentThinking(ctx.sessionId, false);
  ctx.flushSessionDeltas(ctx.sessionId);

  state.setPendingAskHuman(ctx.sessionId, {
    requestId: event.request_id,
    question: event.question,
    inputType: event.input_type as "credentials" | "choice" | "freetext" | "confirmation",
    options: event.options ?? [],
    context: event.context ?? "",
  });
};

/**
 * Handle ask_human_response event.
 * User responded to an ask_human request.
 */
export const handleAskHumanResponse: EventHandler<{
  type: "ask_human_response";
  request_id: string;
  response: string;
  skipped: boolean;
  session_id: string;
  seq?: number;
}> = (_event, ctx) => {
  const state = ctx.getState();
  state.clearPendingAskHuman(ctx.sessionId);
};
