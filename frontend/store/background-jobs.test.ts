import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "./index";

const SESSION_ID = "session-background-jobs";

function seedSession() {
  useStore.setState((state) => {
    state.sessions[SESSION_ID] = {
      id: SESSION_ID,
      name: "Background jobs",
      workingDirectory: "/tmp",
      createdAt: "2026-07-21T00:00:00.000Z",
      mode: "agent",
    };
    state.timelines[SESSION_ID] = [];
    state.backgroundJobs[SESSION_ID] = [];
  });
}

describe("background job lifecycle state", () => {
  beforeEach(() => {
    useStore.setState({ sessions: {}, timelines: {}, backgroundJobs: {} });
    seedSession();
  });

  it("retains background metadata on the originating main tool after completion", () => {
    const state = useStore.getState();
    state.addToolExecutionBlock(SESSION_ID, {
      requestId: "req-main",
      toolName: "pentest_run",
      args: { tool_name: "naabu" },
    });
    state.backgroundToolExecutionBlock(SESSION_ID, "req-main", {
      status: "backgrounded",
      job_id: "job-main",
      soft_timeout_ms: 30_000,
      hard_timeout_ms: 1_800_000,
    });
    state.completeToolExecutionBlock(SESSION_ID, "req-main", true, {
      status: "done",
      job_id: "job-main",
      exit_code: 0,
    });

    const block = useStore
      .getState()
      .timelines[SESSION_ID]?.find((candidate) => candidate.type === "ai_tool_execution");
    expect(block?.type).toBe("ai_tool_execution");
    if (block?.type !== "ai_tool_execution") return;
    expect(block.data.status).toBe("completed");
    expect(block.data.backgroundRun).toEqual(
      expect.objectContaining({
        jobId: "job-main",
        softTimeoutMs: 30_000,
        hardTimeoutMs: 1_800_000,
      })
    );
  });

  it("opens the exact main tool detail from a live job", () => {
    useStore.getState().addBackgroundJob(SESSION_ID, {
      jobId: "job-main",
      command: "naabu -host 10.0.0.1",
      toolName: "pentest_run",
      origin: { kind: "main_tool", requestId: "req-main" },
      startedAt: Date.now(),
      backgroundedAt: Date.now(),
      state: "running",
    });

    useStore.getState().openBackgroundJobDetail(SESSION_ID, "job-main");

    expect(useStore.getState().sessions[SESSION_ID]).toEqual(
      expect.objectContaining({
        detailViewMode: "tool-detail",
        toolDetailRequestIds: ["req-main"],
        backgroundToolFocusRequestId: null,
      })
    );
  });

  it("opens the parent sub-agent and records the exact child tool focus", () => {
    useStore.getState().addBackgroundJob(SESSION_ID, {
      jobId: "job-child",
      command: "nmap -sV 10.0.0.1",
      toolName: "pentest_run",
      origin: {
        kind: "sub_agent_tool",
        parentRequestId: "agent-parent",
        requestId: "req-child",
      },
      startedAt: Date.now(),
      backgroundedAt: Date.now(),
      state: "running",
    });

    useStore.getState().openBackgroundJobDetail(SESSION_ID, "job-child");

    expect(useStore.getState().sessions[SESSION_ID]).toEqual(
      expect.objectContaining({
        detailViewMode: "sub-agent-detail",
        toolDetailRequestIds: ["agent-parent"],
        backgroundToolFocusRequestId: "req-child",
      })
    );
  });
});
