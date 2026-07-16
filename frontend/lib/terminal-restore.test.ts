import { afterEach, describe, expect, it, vi } from "vitest";
import { writeLastProfile } from "@/lib/ai/execution-mode";
import { useStore } from "@/store";
import { restoreTerminalForConv } from "./terminal-restore";

function seedConversationTerminal(executionMode = "chat") {
  useStore.setState({
    activeSessionId: null,
    activeSubAgents: {},
    conversationTerminals: { "conv-1": ["term-1"] },
    subAgentBatchCounter: {},
    timelines: { "term-1": [] },
    sessions: {
      "term-1": {
        id: "term-1",
        name: "Terminal",
        workingDirectory: "/tmp",
        createdAt: new Date().toISOString(),
        mode: "agent",
        tabType: "terminal",
        executionMode,
      },
    },
  });
}

describe("restoreTerminalForConv execution mode restore", () => {
  afterEach(() => {
    globalThis.localStorage?.clear();
    useStore.setState({
      activeSessionId: null,
      activeSubAgents: {},
      conversationTerminals: {},
      subAgentBatchCounter: {},
      timelines: {},
      sessions: {},
    });
    vi.clearAllMocks();
  });

  it("preserves concrete Task profile ids during restore", async () => {
    seedConversationTerminal();

    await restoreTerminalForConv(
      "conv-1",
      { workingDirectory: "/tmp", scrollback: "", executionMode: "red_team" },
      true,
      vi.fn()
    );

    expect(useStore.getState().sessions["term-1"]?.executionMode).toBe("red_team");
  });

  it("normalizes legacy bare task restore data to the remembered profile", async () => {
    writeLastProfile("red_team");
    seedConversationTerminal();

    await restoreTerminalForConv(
      "conv-1",
      { workingDirectory: "/tmp", scrollback: "", executionMode: "task" },
      true,
      vi.fn()
    );

    expect(useStore.getState().sessions["term-1"]?.executionMode).toBe("red_team");
  });

  it("restores stale child work as interrupted and reactivates the same durable identity", async () => {
    seedConversationTerminal("red_team");
    const parentRequestId =
      "dispatch-request::worker:dc2d374b-8870-49ed-9864-5ca45e28bbf5";

    await restoreTerminalForConv(
      "conv-1",
      {
        workingDirectory: "/tmp",
        scrollback: "",
        executionMode: "red_team",
        timelineBlocks: [
          {
            id: `sub-agent-${parentRequestId}`,
            type: "sub_agent_activity",
            timestamp: "2026-07-16T12:11:26.000Z",
            data: {
              agentId: "prober",
              agentName: "Prober",
              parentRequestId,
              task: "probe the external attack surface",
              depth: 1,
              status: "running",
              toolCalls: [
                {
                  id: "tool-before-restart",
                  name: "eas_discover_ports",
                  args: { targets: ["101.42.9.109"] },
                  status: "running",
                  startedAt: "2026-07-16T12:11:58.000Z",
                },
              ],
              entries: [{ kind: "tool_call", toolCallId: "tool-before-restart" }],
              startedAt: "2026-07-16T12:11:26.000Z",
            },
          },
        ],
      },
      true,
      vi.fn()
    );

    const restored = useStore.getState().activeSubAgents["term-1"][0];
    expect(restored.status).toBe("interrupted");
    expect(restored.toolCalls[0].status).toBe("interrupted");

    const store = useStore.getState();
    store.startSubAgent("term-1", {
      agentId: "prober",
      agentName: "Prober",
      parentRequestId,
      task: "resume the exact durable worker",
      depth: 1,
    });
    store.addSubAgentToolCall("term-1", parentRequestId, {
      id: "tool-after-resume",
      name: "eas_fingerprint_services",
      args: { targets: ["101.42.9.109:80"] },
    });

    const resumed = useStore.getState().activeSubAgents["term-1"][0];
    expect(resumed.status).toBe("running");
    expect(resumed.parentRequestId).toBe(parentRequestId);
    expect(resumed.toolCalls.map((tool) => [tool.id, tool.status])).toEqual([
      ["tool-before-restart", "interrupted"],
      ["tool-after-resume", "running"],
    ]);
    expect(resumed.entries).toEqual([
      { kind: "tool_call", toolCallId: "tool-before-restart" },
      { kind: "tool_call", toolCallId: "tool-after-resume" },
    ]);
  });

  it("repairs a legacy interrupted parent whose nested tool was still persisted running", async () => {
    seedConversationTerminal("red_team");

    await restoreTerminalForConv(
      "conv-1",
      {
        workingDirectory: "/tmp",
        scrollback: "",
        timelineBlocks: [
          {
            id: "sub-agent-legacy-interrupted-worker",
            type: "sub_agent_activity",
            timestamp: "2026-07-16T12:11:26.000Z",
            data: {
              agentId: "prober",
              agentName: "Prober",
              parentRequestId: "legacy-interrupted-worker",
              task: "probe the external attack surface",
              depth: 1,
              status: "interrupted",
              toolCalls: [
                {
                  id: "legacy-running-tool",
                  name: "eas_discover_ports",
                  args: {},
                  status: "running",
                  startedAt: "2026-07-16T12:11:58.000Z",
                },
                {
                  id: "legacy-backgrounded-tool",
                  name: "eas_fingerprint_services",
                  args: {},
                  status: "backgrounded",
                  result: { status: "backgrounded", job_id: "job_lost_on_restart" },
                  startedAt: "2026-07-16T12:12:58.000Z",
                },
              ],
              entries: [{ kind: "tool_call", toolCallId: "legacy-running-tool" }],
              startedAt: "2026-07-16T12:11:26.000Z",
              promptGeneration: {
                status: "generating",
                architectSystemPrompt: "architect system",
                architectUserMessage: "architect user",
              },
            },
          },
        ],
      },
      true,
      vi.fn()
    );

    const restored = useStore.getState().activeSubAgents["term-1"][0];
    expect(restored.status).toBe("interrupted");
    expect(restored.toolCalls.map((tool) => tool.status)).toEqual([
      "interrupted",
      "interrupted",
    ]);
    expect(restored.promptGeneration?.status).toBe("failed");

    useStore.getState().completeBackgroundedSubAgentToolCall(
      "term-1",
      "job_lost_on_restart",
      true,
      { status: "completed", stdout: "authoritative late result" }
    );
    const completed = useStore.getState().activeSubAgents["term-1"][0].toolCalls[1];
    expect(completed.status).toBe("completed");
    expect(completed.result).toEqual({
      status: "completed",
      stdout: "authoritative late result",
    });
  });
});
