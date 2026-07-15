import { describe, expect, it, vi } from "vitest";
import {
  handleSubAgentCompleted,
  handleSubAgentError,
  handleSubAgentToolRequest,
  handleSubAgentToolResult,
} from "./sub-agent-handlers";
import type { EventHandlerContext } from "./types";

function mockCtx() {
  const state = {
    addSubAgentToolCall: vi.fn(),
    completeSubAgentToolCall: vi.fn(),
    completeSubAgent: vi.fn(),
    failSubAgent: vi.fn(),
  };
  const ctx: EventHandlerContext = {
    sessionId: "session-1",
    getState: vi.fn(() => state) as unknown as EventHandlerContext["getState"],
    flushTextDeltas: vi.fn(),
    flushSessionDeltas: vi.fn(),
    batchTextDelta: vi.fn(),
    batchThinkingContent: vi.fn(),
    batchSubAgentThinking: vi.fn(),
    batchToolOutputChunk: vi.fn(),
    convertToolSource: vi.fn(() => undefined),
  };
  return { state, ctx };
}

describe("sub-agent reasoning boundaries", () => {
  it("flushes pending reasoning before a tool request", () => {
    const { state, ctx } = mockCtx();

    handleSubAgentToolRequest(
      {
        type: "sub_agent_tool_request",
        agent_id: "recon",
        tool_name: "recon_list_providers",
        args: {},
        request_id: "tool-1",
        parent_request_id: "worker-1",
        session_id: "session-1",
      },
      ctx
    );

    expect(ctx.flushSessionDeltas).toHaveBeenCalledWith("session-1");
    expect(state.addSubAgentToolCall).toHaveBeenCalled();
  });

  it.each(["result", "completed", "error"] as const)(
    "flushes pending reasoning before the %s boundary",
    (boundary) => {
      const { ctx } = mockCtx();
      if (boundary === "result") {
        handleSubAgentToolResult(
          {
            type: "sub_agent_tool_result",
            agent_id: "recon",
            tool_name: "recon_list_providers",
            success: true,
            result: {},
            request_id: "tool-1",
            parent_request_id: "worker-1",
            session_id: "session-1",
          },
          ctx
        );
      } else if (boundary === "completed") {
        handleSubAgentCompleted(
          {
            type: "sub_agent_completed",
            agent_id: "recon",
            response: "done",
            duration_ms: 1n,
            parent_request_id: "worker-1",
            session_id: "session-1",
          },
          ctx
        );
      } else {
        handleSubAgentError(
          {
            type: "sub_agent_error",
            agent_id: "recon",
            error: "failed",
            parent_request_id: "worker-1",
            session_id: "session-1",
          },
          ctx
        );
      }

      expect(ctx.flushSessionDeltas).toHaveBeenCalledWith("session-1");
    }
  );
});
