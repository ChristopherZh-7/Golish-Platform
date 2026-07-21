/**
 * Sub-agent related AI event handlers.
 *
 * Handles sub-agent lifecycle events: started, tool_request, tool_result,
 * completed, error.
 */

import type { JsonValue } from "@/lib/serde_json/JsonValue";
import { isBackgroundedResult, registerBackgroundJobFromResult } from "./tool-handlers";
import type { EventHandler } from "./types";

/**
 * Handle prompt generation started event.
 */
export const handlePromptGenerationStarted: EventHandler<{
  type: "prompt_generation_started";
  agent_id: string;
  parent_request_id: string;
  architect_system_prompt: string;
  architect_user_message: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  ctx.getState().startPromptGeneration(ctx.sessionId, event.agent_id, event.parent_request_id, {
    architectSystemPrompt: event.architect_system_prompt,
    architectUserMessage: event.architect_user_message,
  });
};

/**
 * Handle prompt generation completed event.
 */
export const handlePromptGenerationCompleted: EventHandler<{
  type: "prompt_generation_completed";
  agent_id: string;
  parent_request_id: string;
  generated_prompt: string | null;
  success: boolean;
  duration_ms: bigint;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  ctx.getState().completePromptGeneration(ctx.sessionId, event.agent_id, event.parent_request_id, {
    generatedPrompt: event.generated_prompt ?? undefined,
    success: event.success,
    durationMs: Number(event.duration_ms),
  });
};

/**
 * Handle sub-agent started event.
 */
export const handleSubAgentStarted: EventHandler<{
  type: "sub_agent_started";
  agent_id: string;
  agent_name: string;
  task: string;
  depth: number;
  parent_request_id: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const state = ctx.getState();
  state.startSubAgent(ctx.sessionId, {
    agentId: event.agent_id,
    agentName: event.agent_name,
    parentRequestId: event.parent_request_id,
    task: event.task,
    depth: event.depth,
  });

  // Sub-agent detail is shown on-demand when the user clicks the inline card.
  // No auto-switch here since sub-agent-detail only shows a single agent.
};

/**
 * Handle sub-agent tool request event.
 */
export const handleSubAgentToolRequest: EventHandler<{
  type: "sub_agent_tool_request";
  agent_id: string;
  tool_name: string;
  args: JsonValue;
  request_id: string;
  parent_request_id: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  ctx.flushSessionDeltas(ctx.sessionId);
  ctx.getState().addSubAgentToolCall(ctx.sessionId, event.parent_request_id, {
    id: event.request_id,
    name: event.tool_name,
    args: event.args as Record<string, unknown>,
  });
};

/**
 * Handle sub-agent tool result event.
 */
export const handleSubAgentToolResult: EventHandler<{
  type: "sub_agent_tool_result";
  agent_id: string;
  tool_name: string;
  success: boolean;
  result: JsonValue;
  request_id: string;
  parent_request_id: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  ctx.flushSessionDeltas(ctx.sessionId);
  const state = ctx.getState();
  // Soft-timeout → backgrounded: a sub-agent command exceeded its soft timeout
  // and was detached to a background job (still running). Register it into the
  // Cursor-style background-jobs indicator (mirrors the main-agent path) so it
  // surfaces in the input-row badge + sub-agent detail header. The sub-agent's
  // turn continues, so still resolve the card carrying the backgrounded result.
  if (isBackgroundedResult(event.result)) {
    registerBackgroundJobFromResult(state, ctx.sessionId, event.result, {
      requestId: event.request_id,
      toolName: event.tool_name,
      source: "sub_agent",
      parentRequestId: event.parent_request_id,
    });
  }
  state.completeSubAgentToolCall(
    ctx.sessionId,
    event.parent_request_id,
    event.request_id,
    event.success,
    event.result
  );
};

/**
 * Handle sub-agent streaming text delta.
 */
export const handleSubAgentTextDelta: EventHandler<{
  type: "sub_agent_text_delta";
  agent_id: string;
  delta: string;
  accumulated: string;
  parent_request_id: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  ctx
    .getState()
    .updateSubAgentStreamingText(ctx.sessionId, event.parent_request_id, event.accumulated);
};

/**
 * Handle sub-agent reasoning/thinking delta.
 */
export const handleSubAgentReasoning: EventHandler<{
  type: "sub_agent_reasoning";
  agent_id: string;
  delta: string;
  accumulated: string;
  parent_request_id: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  ctx.batchSubAgentThinking(ctx.sessionId, event.parent_request_id, event.accumulated);
};

/**
 * Handle sub-agent completed event.
 */
export const handleSubAgentCompleted: EventHandler<{
  type: "sub_agent_completed";
  agent_id: string;
  response: string;
  duration_ms: bigint;
  parent_request_id: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  ctx.flushSessionDeltas(ctx.sessionId);
  const state = ctx.getState();

  // Handle coder results with special rendering
  if (event.agent_id === "coder") {
    state.addUdiffResultBlock(ctx.sessionId, event.response, Number(event.duration_ms));
  }

  state.completeSubAgent(ctx.sessionId, event.parent_request_id, {
    response: event.response,
    durationMs: Number(event.duration_ms),
  });
};

/**
 * Handle sub-agent error event.
 */
export const handleSubAgentError: EventHandler<{
  type: "sub_agent_error";
  agent_id: string;
  error: string;
  parent_request_id: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  ctx.flushSessionDeltas(ctx.sessionId);
  ctx.getState().failSubAgent(ctx.sessionId, event.parent_request_id, event.error);
};
