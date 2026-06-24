import { describe, expect, it, vi } from "vitest";
import {
  handleToolApprovalRequest,
  handleToolAutoApproved,
  handleToolOutputChunk,
  handleToolRequest,
} from "./tool-handlers";
import type { EventHandlerContext } from "./types";

function mockCtx() {
  const state = {
    isToolRequestProcessed: vi.fn(() => false),
    markToolRequestProcessed: vi.fn(),
    setAgentThinking: vi.fn(),
    addActiveToolCall: vi.fn(),
    addStreamingToolBlock: vi.fn(),
    addToolExecutionBlock: vi.fn(),
    setDetailViewMode: vi.fn(),
    setSessionStageRun: vi.fn(),
    setPendingToolApproval: vi.fn(),
    sessions: { "sess-1": { id: "sess-1", agentMode: "manual" } },
  };
  const ctx: EventHandlerContext = {
    sessionId: "sess-1",
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

describe("tool event detail pane behavior", () => {
  it("records main tool requests without forcing the detail pane open", () => {
    const { state, ctx } = mockCtx();

    handleToolRequest(
      {
        type: "tool_request",
        tool_name: "stage_run",
        args: {},
        request_id: "T1",
        source: { type: "main" },
        session_id: "sess-1",
      } as Parameters<typeof handleToolRequest>[0],
      ctx
    );

    expect(state.addToolExecutionBlock).toHaveBeenCalledWith(
      "sess-1",
      expect.objectContaining({ requestId: "T1", toolName: "stage_run" })
    );
    expect(state.setSessionStageRun).toHaveBeenCalledWith(
      "sess-1",
      expect.objectContaining({ requestId: "T1" })
    );
    expect(state.setDetailViewMode).not.toHaveBeenCalled();
  });

  it("records approval-gated tools without forcing the detail pane open", () => {
    const { state, ctx } = mockCtx();

    handleToolApprovalRequest(
      {
        type: "tool_approval_request",
        tool_name: "stage_run",
        args: {},
        request_id: "T2",
        stats: null,
        risk_level: "low",
        can_learn: false,
        suggestion: null,
        source: { type: "main" },
        session_id: "sess-1",
      } as Parameters<typeof handleToolApprovalRequest>[0],
      ctx
    );

    expect(state.addToolExecutionBlock).toHaveBeenCalledWith(
      "sess-1",
      expect.objectContaining({ requestId: "T2", toolName: "stage_run" })
    );
    expect(state.setDetailViewMode).not.toHaveBeenCalled();
  });

  it("records auto-approved tools without forcing the detail pane open", () => {
    const { state, ctx } = mockCtx();

    handleToolAutoApproved(
      {
        type: "tool_auto_approved",
        tool_name: "stage_run",
        args: {},
        request_id: "T3",
        reason: "safe",
        source: { type: "main" },
        session_id: "sess-1",
      } as Parameters<typeof handleToolAutoApproved>[0],
      ctx
    );

    expect(state.addToolExecutionBlock).toHaveBeenCalledWith(
      "sess-1",
      expect.objectContaining({ requestId: "T3", toolName: "stage_run" })
    );
    expect(state.setDetailViewMode).not.toHaveBeenCalled();
  });

  it("batches main tool output chunks", () => {
    const { ctx } = mockCtx();

    handleToolOutputChunk(
      {
        type: "tool_output_chunk",
        tool_name: "pentest_run",
        request_id: "T4",
        chunk: "line 1\n",
        stream: "stdout",
        source: { type: "main" },
        session_id: "sess-1",
      } as Parameters<typeof handleToolOutputChunk>[0],
      ctx
    );

    expect(ctx.batchToolOutputChunk).toHaveBeenCalledWith("sess-1", "T4", "line 1\n", "main");
  });

  it("batches sub-agent tool output chunks", () => {
    const { ctx } = mockCtx();

    handleToolOutputChunk(
      {
        type: "tool_output_chunk",
        tool_name: "pentest_run",
        request_id: "T5",
        chunk: "sub line\n",
        stream: "stdout",
        source: { type: "sub_agent", agent_id: "a1", agent_name: "Recon" },
        session_id: "sess-1",
      } as Parameters<typeof handleToolOutputChunk>[0],
      ctx
    );

    expect(ctx.batchToolOutputChunk).toHaveBeenCalledWith(
      "sess-1",
      "T5",
      "sub line\n",
      "sub_agent"
    );
  });
});
