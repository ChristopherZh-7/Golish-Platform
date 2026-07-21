/**
 * Tests for the AI event handler registry.
 */

import { beforeEach, describe, expect, it, type Mock, vi } from "vitest";
import { dispatchEvent, eventHandlerRegistry } from "./registry";
import type { EventHandlerContext, EventHandlerRegistry } from "./types";

// Mock logger
vi.mock("@/lib/logger", () => ({
  logger: {
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
  },
}));

describe("eventHandlerRegistry", () => {
  it("contains handlers for all core lifecycle events", () => {
    expect(eventHandlerRegistry.started).toBeDefined();
    expect(eventHandlerRegistry.text_delta).toBeDefined();
    expect(eventHandlerRegistry.reasoning).toBeDefined();
    expect(eventHandlerRegistry.completed).toBeDefined();
    expect(eventHandlerRegistry.error).toBeDefined();
    expect(eventHandlerRegistry.system_hooks_injected).toBeDefined();
  });

  it("contains handlers for all tool events", () => {
    expect(eventHandlerRegistry.tool_request).toBeDefined();
    expect(eventHandlerRegistry.tool_intent_observation).toBeDefined();
    expect(eventHandlerRegistry.tool_approval_request).toBeDefined();
    expect(eventHandlerRegistry.tool_auto_approved).toBeDefined();
    expect(eventHandlerRegistry.tool_result).toBeDefined();
  });

  it("contains handlers for all workflow events", () => {
    expect(eventHandlerRegistry.workflow_started).toBeDefined();
    expect(eventHandlerRegistry.workflow_step_started).toBeDefined();
    expect(eventHandlerRegistry.workflow_step_completed).toBeDefined();
    expect(eventHandlerRegistry.workflow_completed).toBeDefined();
    expect(eventHandlerRegistry.workflow_error).toBeDefined();
  });

  it("contains handlers for all sub-agent events", () => {
    expect(eventHandlerRegistry.sub_agent_started).toBeDefined();
    expect(eventHandlerRegistry.sub_agent_tool_request).toBeDefined();
    expect(eventHandlerRegistry.sub_agent_tool_result).toBeDefined();
    expect(eventHandlerRegistry.sub_agent_reasoning).toBeDefined();
    expect(eventHandlerRegistry.sub_agent_completed).toBeDefined();
    expect(eventHandlerRegistry.sub_agent_error).toBeDefined();
  });

  it("contains handlers for all context management events", () => {
    expect(eventHandlerRegistry.context_warning).toBeDefined();
    expect(eventHandlerRegistry.compaction_started).toBeDefined();
    expect(eventHandlerRegistry.compaction_completed).toBeDefined();
    expect(eventHandlerRegistry.compaction_failed).toBeDefined();
    expect(eventHandlerRegistry.tool_response_truncated).toBeDefined();
  });

  it("contains handlers for all miscellaneous events", () => {
    expect(eventHandlerRegistry.plan_updated).toBeDefined();
    expect(eventHandlerRegistry.server_tool_started).toBeDefined();
    expect(eventHandlerRegistry.web_search_result).toBeDefined();
    expect(eventHandlerRegistry.web_fetch_result).toBeDefined();
  });

  it("contains a handler for background tool completion", () => {
    expect(eventHandlerRegistry.tool_background_completed).toBeDefined();
  });

  it("contains a handler for harness-trace (stage-run progress)", () => {
    expect(eventHandlerRegistry.harness_trace).toBeDefined();
  });

  it("has exactly 46 registered handlers", () => {
    const registeredHandlers = Object.keys(eventHandlerRegistry).filter(
      (key) => eventHandlerRegistry[key as keyof EventHandlerRegistry] !== undefined
    );
    expect(registeredHandlers.length).toBe(46);
  });
});

describe("dispatchEvent", () => {
  let mockCtx: EventHandlerContext;
  let mockState: Record<string, Mock>;

  beforeEach(() => {
    mockState = {
      clearAgentStreaming: vi.fn(),
      clearActiveToolCalls: vi.fn(),
      clearThinkingContent: vi.fn(),
      setAgentThinking: vi.fn(),
      setAgentResponding: vi.fn(),
      appendThinkingContent: vi.fn(),
      addStreamingSystemHooksBlock: vi.fn(),
      addSystemHookBlock: vi.fn(),
      recordToolIntentObservation: vi.fn(),
    };

    mockCtx = {
      sessionId: "test-session",
      getState: vi.fn(() => mockState) as unknown as EventHandlerContext["getState"],
      flushTextDeltas: vi.fn(),
      flushSessionDeltas: vi.fn(),
      batchTextDelta: vi.fn(),
      batchThinkingContent: vi.fn(),
      batchSubAgentThinking: vi.fn(),
      batchToolOutputChunk: vi.fn(),
      convertToolSource: vi.fn(),
    };
  });

  it("returns true when event is handled", () => {
    const event = {
      type: "started" as const,
      turn_id: "turn-1",
      session_id: "test-session",
    };
    const result = dispatchEvent(event, mockCtx);
    expect(result).toBe(true);
  });

  it("returns false for unknown event types", () => {
    // Use type assertion to test runtime behavior with unknown event type
    const event = {
      type: "unknown_event",
      session_id: "test-session",
    } as unknown as Parameters<typeof dispatchEvent>[0];
    const result = dispatchEvent(event, mockCtx);
    expect(result).toBe(false);
  });

  it("dispatches started event to correct handler", () => {
    const event = {
      type: "started" as const,
      turn_id: "turn-1",
      session_id: "test-session",
    };
    dispatchEvent(event, mockCtx);

    expect(mockCtx.getState).toHaveBeenCalled();
    expect(mockState.clearAgentStreaming).toHaveBeenCalledWith("test-session");
    expect(mockState.setAgentThinking).toHaveBeenCalledWith("test-session", true);
    expect(mockState.setAgentResponding).toHaveBeenCalledWith("test-session", true);
  });

  it("dispatches text_delta event to correct handler", () => {
    const event = {
      type: "text_delta" as const,
      delta: "Hello",
      accumulated: "Hello",
      session_id: "test-session",
    };
    dispatchEvent(event, mockCtx);

    expect(mockCtx.batchTextDelta).toHaveBeenCalledWith("test-session", "Hello");
    expect(mockState.setAgentThinking).toHaveBeenCalledWith("test-session", false);
  });

  it("dispatches tool_intent_observation event to correct handler", () => {
    const event = {
      type: "tool_intent_observation" as const,
      request_id: "req-1",
      tool_name: "ask_human",
      source: "textual_xml",
      decision: "require_human_answer",
      reason: "needs user",
      raw_preview: null,
      session_id: "test-session",
    };
    dispatchEvent(event, mockCtx);

    expect(mockState.recordToolIntentObservation).toHaveBeenCalledWith("test-session", {
      requestId: "req-1",
      modelWanted: "ask_human",
      source: "textual_xml",
      decision: "require_human_answer",
      reason: "needs user",
      rawPreview: undefined,
    });
  });

  it("dispatches reasoning event to correct handler", () => {
    const event = {
      type: "reasoning" as const,
      content: "Thinking about this...",
      session_id: "test-session",
    };
    dispatchEvent(event, mockCtx);

    expect(mockCtx.flushTextDeltas).toHaveBeenCalledWith("test-session");
    expect(mockCtx.batchThinkingContent).toHaveBeenCalledWith(
      "test-session",
      "Thinking about this..."
    );
    expect(mockState.appendThinkingContent).not.toHaveBeenCalled();
  });

  it("dispatches tool_background_completed and completes the matching tool card", () => {
    const completeToolExecutionBlock = vi.fn();
    const updateStreamingToolBlock = vi.fn();
    const completeBackgroundedSubAgentToolCall = vi.fn();
    const stateWithTimeline = {
      completeToolExecutionBlock,
      updateStreamingToolBlock,
      completeBackgroundedSubAgentToolCall,
      removeBackgroundJob: vi.fn(),
      timelines: {
        "test-session": [
          {
            type: "ai_tool_execution",
            data: { requestId: "req-bg", result: { job_id: "job_x", status: "backgrounded" } },
          },
        ],
      },
    };
    const ctx = {
      ...mockCtx,
      getState: vi.fn(() => stateWithTimeline) as unknown as EventHandlerContext["getState"],
    };
    const event = {
      type: "tool_background_completed" as const,
      job_id: "job_x",
      command: "sleep 99 && echo done",
      status: "done",
      exit_code: 0,
      stdout_tail: "done",
      stderr_tail: "",
      duration_ms: 1234n,
      session_id: "test-session",
    };

    const handled = dispatchEvent(event, ctx);

    expect(handled).toBe(true);
    expect(completeToolExecutionBlock).toHaveBeenCalledWith(
      "test-session",
      "req-bg",
      true,
      expect.objectContaining({ job_id: "job_x", status: "done", exit_code: 0 })
    );
    // Both views (timeline + interleaved) must converge on the terminal result.
    expect(updateStreamingToolBlock).toHaveBeenCalledWith(
      "test-session",
      "req-bg",
      true,
      expect.objectContaining({ job_id: "job_x", status: "done" })
    );
    expect(completeBackgroundedSubAgentToolCall).toHaveBeenCalledWith(
      "test-session",
      "job_x",
      true,
      expect.objectContaining({ job_id: "job_x", status: "done", exit_code: 0 })
    );
  });

  it("routes a backgrounded tool_result to the live (non-terminal) state", () => {
    const backgroundState = {
      completeActiveToolCall: vi.fn(),
      backgroundStreamingToolBlock: vi.fn(),
      backgroundToolExecutionBlock: vi.fn(),
      updateStreamingToolBlock: vi.fn(),
      completeToolExecutionBlock: vi.fn(),
      addBackgroundJob: vi.fn(),
    };
    const ctx = {
      ...mockCtx,
      getState: vi.fn(() => backgroundState) as unknown as EventHandlerContext["getState"],
    };
    const result = {
      status: "backgrounded",
      job_id: "job_42",
      command: "naabu -host 10.0.0.1",
      partial_stdout: "scanning...",
      soft_timeout_ms: 30_000,
      hard_timeout_ms: 1_800_000,
    };
    const event = {
      type: "tool_result" as const,
      tool_name: "pentest_run",
      result,
      success: true,
      request_id: "req-bg2",
      source: { type: "main" as const },
      session_id: "test-session",
    };

    const handled = dispatchEvent(event, ctx);

    expect(handled).toBe(true);
    expect(backgroundState.backgroundToolExecutionBlock).toHaveBeenCalledWith(
      "test-session",
      "req-bg2",
      result
    );
    expect(backgroundState.backgroundStreamingToolBlock).toHaveBeenCalledWith(
      "test-session",
      "req-bg2",
      result
    );
    expect(backgroundState.completeActiveToolCall).toHaveBeenCalledWith(
      "test-session",
      "req-bg2",
      true,
      result
    );
    expect(backgroundState.addBackgroundJob).toHaveBeenCalledWith(
      "test-session",
      expect.objectContaining({
        jobId: "job_42",
        toolName: "pentest_run",
        origin: { kind: "main_tool", requestId: "req-bg2" },
        softTimeoutMs: 30_000,
        hardTimeoutMs: 1_800_000,
        state: "running",
      })
    );
    // Must NOT terminally complete the card while the job is still running.
    expect(backgroundState.completeToolExecutionBlock).not.toHaveBeenCalled();
  });

  it("routes a normal tool_result to terminal completion", () => {
    const normalState = {
      completeActiveToolCall: vi.fn(),
      backgroundStreamingToolBlock: vi.fn(),
      backgroundToolExecutionBlock: vi.fn(),
      updateStreamingToolBlock: vi.fn(),
      completeToolExecutionBlock: vi.fn(),
    };
    const ctx = {
      ...mockCtx,
      getState: vi.fn(() => normalState) as unknown as EventHandlerContext["getState"],
    };
    const result = { stdout: "ok", exit_code: 0 };
    const event = {
      type: "tool_result" as const,
      tool_name: "run_pty_cmd",
      result,
      success: true,
      request_id: "req-ok",
      source: { type: "main" as const },
      session_id: "test-session",
    };

    dispatchEvent(event, ctx);

    expect(normalState.completeToolExecutionBlock).toHaveBeenCalledWith(
      "test-session",
      "req-ok",
      true,
      result
    );
    expect(normalState.backgroundToolExecutionBlock).not.toHaveBeenCalled();
  });

  it("registers a background job when a sub-agent tool_result is backgrounded", () => {
    const subAgentState = {
      addBackgroundJob: vi.fn(),
      completeSubAgentToolCall: vi.fn(),
    };
    const ctx = {
      ...mockCtx,
      getState: vi.fn(() => subAgentState) as unknown as EventHandlerContext["getState"],
    };
    const result = {
      status: "backgrounded",
      job_id: "job_sa",
      command: "nmap -p- -sV 10.0.0.0/24",
    };
    const event = {
      type: "sub_agent_tool_result" as const,
      agent_id: "recon",
      tool_name: "pentest_run",
      success: true,
      result,
      request_id: "req-sa-bg",
      parent_request_id: "parent-1",
      session_id: "test-session",
    };

    const handled = dispatchEvent(event, ctx);

    expect(handled).toBe(true);
    // Surfaces in the Cursor-style background-jobs indicator (badge + detail).
    expect(subAgentState.addBackgroundJob).toHaveBeenCalledWith(
      "test-session",
      expect.objectContaining({
        jobId: "job_sa",
        command: "nmap -p- -sV 10.0.0.0/24",
        toolName: "pentest_run",
        origin: {
          kind: "sub_agent_tool",
          parentRequestId: "parent-1",
          requestId: "req-sa-bg",
        },
        state: "running",
      })
    );
    // The sub-agent card is still resolved, carrying the backgrounded result.
    expect(subAgentState.completeSubAgentToolCall).toHaveBeenCalledWith(
      "test-session",
      "parent-1",
      "req-sa-bg",
      true,
      result
    );
  });

  it("does not register a background job for a normal sub-agent tool_result", () => {
    const subAgentState = {
      addBackgroundJob: vi.fn(),
      completeSubAgentToolCall: vi.fn(),
    };
    const ctx = {
      ...mockCtx,
      getState: vi.fn(() => subAgentState) as unknown as EventHandlerContext["getState"],
    };
    const result = { stdout: "PORT 80 open", exit_code: 0 };
    const event = {
      type: "sub_agent_tool_result" as const,
      agent_id: "recon",
      tool_name: "pentest_run",
      success: true,
      result,
      request_id: "req-sa-ok",
      parent_request_id: "parent-1",
      session_id: "test-session",
    };

    dispatchEvent(event, ctx);

    expect(subAgentState.addBackgroundJob).not.toHaveBeenCalled();
    expect(subAgentState.completeSubAgentToolCall).toHaveBeenCalledWith(
      "test-session",
      "parent-1",
      "req-sa-ok",
      true,
      result
    );
  });

  it("records live output activity for the exact background job request", () => {
    const state = {
      markBackgroundJobOutput: vi.fn(),
    };
    const ctx = {
      ...mockCtx,
      getState: vi.fn(() => state) as unknown as EventHandlerContext["getState"],
    };
    const event = {
      type: "tool_output_chunk" as const,
      request_id: "req-bg2",
      tool_name: "pentest_run",
      chunk: "open 443\n",
      stream: "stdout",
      source: { type: "main" as const },
      session_id: "test-session",
    };

    dispatchEvent(event, ctx);

    expect(state.markBackgroundJobOutput).toHaveBeenCalledWith(
      "test-session",
      "req-bg2",
      expect.any(Number)
    );
    expect(ctx.batchToolOutputChunk).toHaveBeenCalledWith(
      "test-session",
      "req-bg2",
      "open 443\n",
      "main"
    );
  });

  it("dispatches system_hooks_injected event to correct handler", () => {
    const event = {
      type: "system_hooks_injected" as const,
      hooks: ["hook1", "hook2"],
      session_id: "test-session",
    };
    dispatchEvent(event, mockCtx);

    expect(mockCtx.flushSessionDeltas).toHaveBeenCalledWith("test-session");
    expect(mockState.addStreamingSystemHooksBlock).toHaveBeenCalledWith("test-session", [
      "hook1",
      "hook2",
    ]);
    expect(mockState.addSystemHookBlock).toHaveBeenCalledWith("test-session", ["hook1", "hook2"]);
  });
});
