import { beforeEach, describe, expect, it } from "vitest";
import { type ActiveToolCall, type ToolCallSource, useStore } from "./index";

describe("Store Workflow Actions", () => {
  const testSessionId = "test-session-123";

  beforeEach(() => {
    // Reset store to initial state
    useStore.setState({
      activeWorkflows: {},
      workflowHistory: {},
      activeToolCalls: {},
    });
  });

  describe("startWorkflow", () => {
    it("creates a new active workflow", () => {
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      const workflow = useStore.getState().activeWorkflows[testSessionId];

      expect(workflow).toBeDefined();
      expect(workflow?.workflowId).toBe("wf-123");
      expect(workflow?.workflowName).toBe("git_commit");
      expect(workflow?.sessionId).toBe("session-456");
      expect(workflow?.status).toBe("running");
      expect(workflow?.steps).toEqual([]);
      expect(workflow?.currentStepIndex).toBe(-1);
      expect(workflow?.totalSteps).toBe(0);
      expect(workflow?.startedAt).toBeDefined();
    });
  });

  describe("workflowStepStarted", () => {
    it("initializes a new step when not present", () => {
      // Start workflow first
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      useStore.getState().workflowStepStarted(testSessionId, {
        stepName: "gatherer",
        stepIndex: 0,
        totalSteps: 4,
      });

      const workflow = useStore.getState().activeWorkflows[testSessionId];

      expect(workflow?.currentStepIndex).toBe(0);
      expect(workflow?.totalSteps).toBe(4);
      expect(workflow?.steps[0]).toEqual(
        expect.objectContaining({
          name: "gatherer",
          index: 0,
          status: "running",
        })
      );
      expect(workflow?.steps[0].startedAt).toBeDefined();
    });

    it("updates existing step to running status", () => {
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      // First call creates the step
      useStore.getState().workflowStepStarted(testSessionId, {
        stepName: "gatherer",
        stepIndex: 0,
        totalSteps: 4,
      });

      // Manually set to pending to simulate restart
      useStore.setState((state) => {
        if (state.activeWorkflows[testSessionId]?.steps[0]) {
          state.activeWorkflows[testSessionId].steps[0].status = "pending";
        }
      });

      // Second call should update to running
      useStore.getState().workflowStepStarted(testSessionId, {
        stepName: "gatherer",
        stepIndex: 0,
        totalSteps: 4,
      });

      const workflow = useStore.getState().activeWorkflows[testSessionId];
      expect(workflow?.steps[0].status).toBe("running");
    });

    it("does nothing when no active workflow", () => {
      useStore.getState().workflowStepStarted(testSessionId, {
        stepName: "gatherer",
        stepIndex: 0,
        totalSteps: 4,
      });

      expect(useStore.getState().activeWorkflows[testSessionId]).toBeUndefined();
    });
  });

  describe("workflowStepCompleted", () => {
    it("marks step as completed with output and duration", () => {
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      useStore.getState().workflowStepStarted(testSessionId, {
        stepName: "gatherer",
        stepIndex: 0,
        totalSteps: 4,
      });

      useStore.getState().workflowStepCompleted(testSessionId, {
        stepName: "gatherer",
        output: "Gathered git data successfully",
        durationMs: 1500,
      });

      const workflow = useStore.getState().activeWorkflows[testSessionId];
      const step = workflow?.steps.find((s) => s.name === "gatherer");

      expect(step?.status).toBe("completed");
      expect(step?.output).toBe("Gathered git data successfully");
      expect(step?.durationMs).toBe(1500);
      expect(step?.completedAt).toBeDefined();
    });

    it("finds step by name rather than index", () => {
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      // Start multiple steps
      useStore.getState().workflowStepStarted(testSessionId, {
        stepName: "gatherer",
        stepIndex: 0,
        totalSteps: 4,
      });
      useStore.getState().workflowStepStarted(testSessionId, {
        stepName: "analyzer",
        stepIndex: 1,
        totalSteps: 4,
      });

      // Complete the second step
      useStore.getState().workflowStepCompleted(testSessionId, {
        stepName: "analyzer",
        output: "Analysis complete",
        durationMs: 2000,
      });

      const workflow = useStore.getState().activeWorkflows[testSessionId];
      expect(workflow?.steps[0].status).toBe("running"); // gatherer still running
      expect(workflow?.steps[1].status).toBe("completed"); // analyzer completed
    });
  });

  describe("completeWorkflow", () => {
    it("marks workflow as completed and moves to history", () => {
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      useStore.getState().completeWorkflow(testSessionId, {
        finalOutput: "## Git Commit Plan\n\n1 commit planned",
        totalDurationMs: 5000,
      });

      const workflow = useStore.getState().activeWorkflows[testSessionId];
      const history = useStore.getState().workflowHistory[testSessionId];

      // Workflow should still be in activeWorkflows for display
      expect(workflow?.status).toBe("completed");
      expect(workflow?.finalOutput).toBe("## Git Commit Plan\n\n1 commit planned");
      expect(workflow?.totalDurationMs).toBe(5000);
      expect(workflow?.completedAt).toBeDefined();

      // Should also be in history
      expect(history).toHaveLength(1);
      expect(history[0].status).toBe("completed");
    });
  });

  describe("failWorkflow", () => {
    it("marks workflow as error and moves to history", () => {
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      useStore.getState().failWorkflow(testSessionId, {
        stepName: "analyzer",
        error: "Failed to analyze changes",
      });

      const workflow = useStore.getState().activeWorkflows[testSessionId];
      const history = useStore.getState().workflowHistory[testSessionId];

      expect(workflow?.status).toBe("error");
      expect(workflow?.error).toBe("Failed to analyze changes");
      expect(workflow?.completedAt).toBeDefined();

      expect(history).toHaveLength(1);
      expect(history[0].status).toBe("error");
    });

    it("marks specified step as error", () => {
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      // Start first step to populate index 0
      useStore.getState().workflowStepStarted(testSessionId, {
        stepName: "gatherer",
        stepIndex: 0,
        totalSteps: 4,
      });

      // Start second step
      useStore.getState().workflowStepStarted(testSessionId, {
        stepName: "analyzer",
        stepIndex: 1,
        totalSteps: 4,
      });

      useStore.getState().failWorkflow(testSessionId, {
        stepName: "analyzer",
        error: "LLM returned empty response",
      });

      const workflow = useStore.getState().activeWorkflows[testSessionId];
      const step = workflow?.steps.find((s) => s.name === "analyzer");

      expect(step?.status).toBe("error");
    });
  });

  describe("clearActiveWorkflow", () => {
    it("removes active workflow for session", () => {
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      expect(useStore.getState().activeWorkflows[testSessionId]).toBeDefined();

      useStore.getState().clearActiveWorkflow(testSessionId);

      expect(useStore.getState().activeWorkflows[testSessionId]).toBeNull();
    });
  });

  describe("preserveWorkflowToolCalls", () => {
    it("preserves tool calls belonging to the workflow", () => {
      const workflowSource: ToolCallSource = {
        type: "workflow",
        workflowId: "wf-123",
        workflowName: "git_commit",
        stepName: "gatherer",
        stepIndex: 0,
      };

      const workflowToolCall: ActiveToolCall = {
        id: "tool-1",
        name: "run_pty_cmd",
        args: { command: "git status" },
        status: "completed",
        startedAt: new Date().toISOString(),
        source: workflowSource,
      };

      const mainToolCall: ActiveToolCall = {
        id: "tool-2",
        name: "read_file",
        args: { path: "/tmp/test.txt" },
        status: "completed",
        startedAt: new Date().toISOString(),
        source: { type: "main" },
      };

      // Set up state
      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      useStore.setState({
        activeToolCalls: {
          [testSessionId]: [workflowToolCall, mainToolCall],
        },
      });

      // Preserve tool calls
      useStore.getState().preserveWorkflowToolCalls(testSessionId);

      const workflow = useStore.getState().activeWorkflows[testSessionId];

      // Only workflow tool calls should be preserved
      expect(workflow?.toolCalls).toHaveLength(1);
      expect(workflow?.toolCalls?.[0].id).toBe("tool-1");
      expect(workflow?.toolCalls?.[0].name).toBe("run_pty_cmd");
    });

    it("does not preserve tool calls from different workflow", () => {
      const differentWorkflowSource: ToolCallSource = {
        type: "workflow",
        workflowId: "wf-999", // Different workflow ID
        workflowName: "other_workflow",
      };

      const toolCall: ActiveToolCall = {
        id: "tool-1",
        name: "run_pty_cmd",
        args: { command: "git status" },
        status: "completed",
        startedAt: new Date().toISOString(),
        source: differentWorkflowSource,
      };

      useStore.getState().startWorkflow(testSessionId, {
        workflowId: "wf-123",
        workflowName: "git_commit",
        workflowSessionId: "session-456",
      });

      useStore.setState({
        activeToolCalls: {
          [testSessionId]: [toolCall],
        },
      });

      useStore.getState().preserveWorkflowToolCalls(testSessionId);

      const workflow = useStore.getState().activeWorkflows[testSessionId];

      expect(workflow?.toolCalls).toHaveLength(0);
    });

    it("does nothing when no active workflow", () => {
      const toolCall: ActiveToolCall = {
        id: "tool-1",
        name: "run_pty_cmd",
        args: { command: "git status" },
        status: "completed",
        startedAt: new Date().toISOString(),
      };

      useStore.setState({
        activeToolCalls: {
          [testSessionId]: [toolCall],
        },
      });

      // Should not throw
      useStore.getState().preserveWorkflowToolCalls(testSessionId);

      // activeToolCalls should be unchanged
      expect(useStore.getState().activeToolCalls[testSessionId]).toHaveLength(1);
    });
  });

  describe("addActiveToolCall with source", () => {
    beforeEach(() => {
      // Add session to store
      useStore.setState({
        sessions: {
          [testSessionId]: {
            id: testSessionId,
            name: "Test Session",
            workingDirectory: "/tmp",
            createdAt: new Date().toISOString(),
            mode: "agent",
          },
        },
        activeToolCalls: {
          [testSessionId]: [],
        },
      });
    });

    it("stores tool call with workflow source", () => {
      const workflowSource: ToolCallSource = {
        type: "workflow",
        workflowId: "wf-123",
        workflowName: "git_commit",
        stepName: "gatherer",
        stepIndex: 0,
      };

      useStore.getState().addActiveToolCall(testSessionId, {
        id: "tool-1",
        name: "run_pty_cmd",
        args: { command: "git status" },
        source: workflowSource,
      });

      const toolCalls = useStore.getState().activeToolCalls[testSessionId];

      expect(toolCalls).toHaveLength(1);
      expect(toolCalls[0].source).toEqual(workflowSource);
    });

    it("stores tool call with main source by default", () => {
      useStore.getState().addActiveToolCall(testSessionId, {
        id: "tool-1",
        name: "read_file",
        args: { path: "/tmp/test.txt" },
      });

      const toolCalls = useStore.getState().activeToolCalls[testSessionId];

      expect(toolCalls).toHaveLength(1);
      expect(toolCalls[0].source).toBeUndefined();
    });
  });

  describe("addStreamingToolBlock with source", () => {
    beforeEach(() => {
      useStore.setState({
        streamingBlocks: {
          [testSessionId]: [],
        },
      });
    });

    it("stores streaming tool block with workflow source", () => {
      const workflowSource: ToolCallSource = {
        type: "workflow",
        workflowId: "wf-123",
        workflowName: "git_commit",
        stepName: "gatherer",
        stepIndex: 0,
      };

      useStore.getState().addStreamingToolBlock(testSessionId, {
        id: "tool-1",
        name: "run_pty_cmd",
        args: { command: "git diff" },
        source: workflowSource,
      });

      const blocks = useStore.getState().streamingBlocks[testSessionId];

      expect(blocks).toHaveLength(1);
      expect(blocks[0].type).toBe("tool");
      if (blocks[0].type === "tool") {
        expect(blocks[0].toolCall.source).toEqual(workflowSource);
      }
    });
  });

  describe("markStagePassed (per-stage roadmap completion)", () => {
    beforeEach(() => {
      useStore.setState({ sessions: {} });
    });

    it("records a passed stage, creating the session row if it does not exist yet", () => {
      // Regression: `markStagePassed` used to early-return when the session row
      // was absent, so the authoritative `stage_passed` write was silently
      // dropped and the per-stage card stayed stuck on "starting…".
      expect(useStore.getState().sessions[testSessionId]).toBeUndefined();

      useStore.getState().markStagePassed(testSessionId, "scoping");

      expect(useStore.getState().sessions[testSessionId]?.passedStages).toEqual(["scoping"]);
    });

    it("is idempotent for a repeated stage and appends new ones in run order", () => {
      useStore.getState().markStagePassed(testSessionId, "scoping");
      useStore.getState().markStagePassed(testSessionId, "scoping");
      useStore.getState().markStagePassed(testSessionId, "target_intel");

      expect(useStore.getState().sessions[testSessionId]?.passedStages).toEqual([
        "scoping",
        "target_intel",
      ]);
    });

    it("co-locates passedStages with the per-stage plan buckets under one session id", () => {
      // The roadmap reads stageOrder / plansByStage / passedStages from ONE
      // resolved store session id; setStagePlan + markStagePassed must agree on
      // that id or a stage never flips from active → passed.
      useStore.getState().setStagePlan(testSessionId, "scoping", {
        version: 1,
        steps: [{ id: "s1", step: "confirm scope", status: "completed" }],
        summary: { total: 1, completed: 1, in_progress: 0, pending: 0 },
        explanation: null,
        updated_at: new Date().toISOString(),
      });
      useStore.getState().markStagePassed(testSessionId, "scoping");

      const session = useStore.getState().sessions[testSessionId];
      expect(session?.stageOrder).toEqual(["scoping"]);
      expect(session?.plansByStage?.scoping?.version).toBe(1);
      expect(session?.passedStages).toEqual(["scoping"]);
    });
  });

  describe("setStagePlan (seed vs real plan)", () => {
    beforeEach(() => {
      useStore.setState({ sessions: {} });
    });

    const seed = (status: "pending" | "in_progress") => ({
      version: 0,
      steps: [{ step: "Target Intel", status }],
      summary: { total: 1, completed: 0, in_progress: status === "in_progress" ? 1 : 0, pending: status === "pending" ? 1 : 0 },
      explanation: null,
      updated_at: "t",
    });
    const real = () => ({
      version: 1,
      steps: [
        { id: "1", step: "DNS resolution", status: "in_progress" as const },
        { id: "2", step: "WHOIS", status: "pending" as const },
      ],
      summary: { total: 2, completed: 0, in_progress: 1, pending: 1 },
      explanation: null,
      updated_at: "t",
    });

    it("lets a real plan (v>=1) supersede a v0 seed", () => {
      useStore.getState().setStagePlan(testSessionId, "target_intel", seed("in_progress"));
      useStore.getState().setStagePlan(testSessionId, "target_intel", real());
      expect(useStore.getState().sessions[testSessionId]?.plansByStage?.target_intel?.version).toBe(1);
    });

    it("does NOT let a v0 re-entry seed downgrade an existing real plan (plan must not 'disappear')", () => {
      // Regression: a stage re-entry re-emits the v0 entry seed; it used to wipe
      // the agent's todos and revert the card to "starting…".
      useStore.getState().setStagePlan(testSessionId, "target_intel", real());
      useStore.getState().setStagePlan(testSessionId, "target_intel", seed("in_progress"));
      const plan = useStore.getState().sessions[testSessionId]?.plansByStage?.target_intel;
      expect(plan?.version).toBe(1);
      expect(plan?.steps).toHaveLength(2);
    });

    it("lets a newer v0 seed replace an older v0 seed", () => {
      useStore.getState().setStagePlan(testSessionId, "target_intel", seed("pending"));
      useStore.getState().setStagePlan(testSessionId, "target_intel", seed("in_progress"));
      const plan = useStore.getState().sessions[testSessionId]?.plansByStage?.target_intel;
      expect(plan?.version).toBe(0);
      expect(plan?.steps[0].status).toBe("in_progress");
    });
  });

  describe("rewindStagePlans (committed stage reset receipt)", () => {
    beforeEach(() => {
      useStore.setState({ sessions: {} });
      for (const stage of ["scoping", "target_intel", "external_attack_surface", "enumeration"]) {
        useStore.getState().setStagePlan(testSessionId, stage, {
          version: 1,
          steps: [{ step: stage, status: "completed" }],
          summary: { total: 1, completed: 1, in_progress: 0, pending: 0 },
          explanation: null,
          updated_at: "old-epoch",
        });
        useStore.getState().markStagePassed(testSessionId, stage);
      }
    });

    it("rewinds affected plans while immediately seeding the committed selected stage", () => {
      useStore
        .getState()
        .rewindStagePlans(
          testSessionId,
          ["external_attack_surface", "enumeration", "reporting"],
          "external_attack_surface"
        );

      const session = useStore.getState().sessions[testSessionId];
      expect(session?.stageOrder).toEqual([
        "scoping",
        "target_intel",
        "external_attack_surface",
      ]);
      expect(Object.keys(session?.plansByStage ?? {})).toEqual([
        "scoping",
        "target_intel",
        "external_attack_surface",
      ]);
      expect(session?.plansByStage?.external_attack_surface?.version).toBe(0);
      expect(session?.plansByStage?.external_attack_surface?.steps[0]?.status).toBe("in_progress");
      expect(session?.passedStages).toEqual(["scoping", "target_intel"]);
    });

    it("allows the replacement v0 seed after removing the old real plan", () => {
      useStore
        .getState()
        .rewindStagePlans(
          testSessionId,
          ["external_attack_surface"],
          "external_attack_surface"
        );
      useStore.getState().setStagePlan(testSessionId, "external_attack_surface", {
        version: 0,
        steps: [{ step: "EAS", status: "in_progress" }],
        summary: { total: 1, completed: 0, in_progress: 1, pending: 0 },
        explanation: null,
        updated_at: "new-epoch",
      });

      expect(
        useStore.getState().sessions[testSessionId]?.plansByStage?.external_attack_surface?.version
      ).toBe(0);
    });

    it("does not create a fake session alias while reconciling a reset", () => {
      useStore.getState().rewindStagePlans("missing-ai-session", ["target_intel"], "target_intel");
      expect(useStore.getState().sessions["missing-ai-session"]).toBeUndefined();
    });
  });
});

describe("Store Workflow Actions — sub-agent streaming entries", () => {
  const sessionId = "sub-agent-stream-session";
  const parentRequestId = "call-sub-agent";

  const entries = () => useStore.getState().activeSubAgents[sessionId][0].entries;

  beforeEach(() => {
    useStore.setState({
      activeSubAgents: {},
      subAgentBatchCounter: {},
      timelines: {},
      sessions: {},
    });

    useStore.getState().startSubAgent(sessionId, {
      agentId: "prober",
      agentName: "Prober",
      parentRequestId,
      task: "probe target",
      depth: 1,
    });
  });

  it("reactivates an interrupted durable sub-agent when the same request identity resumes", () => {
    const store = useStore.getState();
    store.updateSubAgentThinking(sessionId, parentRequestId, "Previous turn checkpointed.");
    store.addSubAgentToolCall(sessionId, parentRequestId, {
      id: "tool-before-restart",
      name: "eas_discover_ports",
      args: { targets: ["101.42.9.109"] },
    });

    useStore.setState((state) => {
      const agent = state.activeSubAgents[sessionId][0];
      agent.status = "interrupted";
      agent.error = "runtime restarted";
      agent.response = "stale terminal response";
      agent.completedAt = "2026-07-16T12:23:59.000Z";
      agent.durationMs = 12_000;

      const block = state.timelines[sessionId].find(
        (candidate) => candidate.type === "sub_agent_activity"
      );
      if (block?.type === "sub_agent_activity") {
        block.data = { ...agent };
      }
    });

    store.startSubAgent(sessionId, {
      agentId: "prober",
      agentName: "Prober",
      parentRequestId,
      task: "resume the exact durable worker",
      depth: 1,
    });

    const resumed = useStore.getState().activeSubAgents[sessionId][0];
    expect(resumed.status).toBe("running");
    expect(resumed.error).toBeUndefined();
    expect(resumed.response).toBeUndefined();
    expect(resumed.completedAt).toBeUndefined();
    expect(resumed.durationMs).toBeUndefined();
    expect(resumed.entries).toEqual([
      expect.objectContaining({ kind: "thinking", text: "Previous turn checkpointed." }),
      { kind: "tool_call", toolCallId: "tool-before-restart" },
    ]);
    expect(resumed.toolCalls).toEqual([
      expect.objectContaining({ id: "tool-before-restart", name: "eas_discover_ports" }),
    ]);

    const timelineAgent = useStore
      .getState()
      .timelines[sessionId].find((block) => block.type === "sub_agent_activity");
    expect(timelineAgent?.type).toBe("sub_agent_activity");
    if (timelineAgent?.type === "sub_agent_activity") {
      expect(timelineAgent.data.status).toBe("running");
      expect(timelineAgent.data.parentRequestId).toBe(parentRequestId);
    }
  });

  it("starts a new accumulated-response boundary when a durable sub-agent resumes", () => {
    const store = useStore.getState();
    store.addSubAgentToolCall(sessionId, parentRequestId, {
      id: "tool-before-interruption",
      name: "eas_discover_ports",
      args: {},
    });
    store.updateSubAgentThinking(sessionId, parentRequestId, "Shared reasoning prefix");
    store.updateSubAgentStreamingText(sessionId, parentRequestId, "Shared answer prefix");

    useStore.setState((state) => {
      state.activeSubAgents[sessionId][0].status = "interrupted";
    });

    store.startSubAgent(sessionId, {
      agentId: "prober",
      agentName: "Prober",
      parentRequestId,
      task: "resume without rewriting the prior response",
      depth: 1,
    });
    store.updateSubAgentThinking(
      sessionId,
      parentRequestId,
      "Shared reasoning prefix from the resumed attempt"
    );
    store.updateSubAgentStreamingText(
      sessionId,
      parentRequestId,
      "Shared answer prefix from the resumed attempt"
    );

    const entries = useStore.getState().activeSubAgents[sessionId][0].entries;
    expect(entries.filter((entry) => entry.kind === "thinking").map((entry) => entry.text)).toEqual([
      "Shared reasoning prefix",
      "Shared reasoning prefix from the resumed attempt",
    ]);
    expect(entries.filter((entry) => entry.kind === "text").map((entry) => entry.text)).toEqual([
      "Shared answer prefix",
      "Shared answer prefix from the resumed attempt",
    ]);
  });

  it("keeps a duplicate started event idempotent while the same sub-agent is live", () => {
    const store = useStore.getState();
    store.updateSubAgentThinking(sessionId, parentRequestId, "Current live reasoning");
    store.updateSubAgentStreamingText(sessionId, parentRequestId, "Current live answer");
    const before = useStore.getState().activeSubAgents[sessionId][0];

    store.startSubAgent(sessionId, {
      agentId: "prober",
      agentName: "Prober",
      parentRequestId,
      task: "duplicate delivery of the same started event",
      depth: 1,
    });

    const after = useStore.getState().activeSubAgents[sessionId][0];
    expect(after.status).toBe("running");
    expect(after.thinking).toBe("Current live reasoning");
    expect(after.streamingText).toBe("Current live answer");
    expect(after.entries).toEqual(before.entries);
    expect(after.attemptEntryStart).toBe(before.attemptEntryStart);
  });

  it("updates the current text response across interleaved thinking instead of duplicating prefixes", () => {
    const store = useStore.getState();

    store.updateSubAgentStreamingText(sessionId, parentRequestId, "n");
    store.updateSubAgentThinking(sessionId, parentRequestId, "thinking");
    store.updateSubAgentStreamingText(sessionId, parentRequestId, "nmap needs root for SYN scan.");

    expect(entries()).toEqual([
      { kind: "text", text: "nmap needs root for SYN scan." },
      expect.objectContaining({ kind: "thinking", text: "thinking" }),
    ]);
  });

  it("updates the current thinking response even when public text arrived after it", () => {
    const store = useStore.getState();

    store.updateSubAgentThinking(sessionId, parentRequestId, "T");
    store.updateSubAgentStreamingText(sessionId, parentRequestId, "Let me run");
    store.updateSubAgentThinking(sessionId, parentRequestId, "Thought through the next probe.");

    expect(entries()).toEqual([
      expect.objectContaining({ kind: "thinking", text: "Thought through the next probe." }),
      { kind: "text", text: "Let me run" },
    ]);
  });

  it("preserves the reasoning batch arrival window instead of timing only the flush", () => {
    const store = useStore.getState();

    store.updateSubAgentThinking(sessionId, parentRequestId, "Thought through the probe.", {
      startedAt: 1_000,
      endedAt: 1_240,
    });

    expect(entries()).toEqual([
      expect.objectContaining({
        kind: "thinking",
        text: "Thought through the probe.",
        startedAt: 1_000,
        endedAt: 1_240,
      }),
    ]);
  });

  it("keeps text entries separate across tool-call boundaries", () => {
    const store = useStore.getState();

    store.updateSubAgentStreamingText(sessionId, parentRequestId, "Let me run");
    store.addSubAgentToolCall(sessionId, parentRequestId, {
      id: "tool-1",
      name: "pentest_run",
      args: {},
    });
    store.updateSubAgentStreamingText(sessionId, parentRequestId, "Let me run the next probe.");

    expect(entries()).toEqual([
      { kind: "text", text: "Let me run" },
      { kind: "tool_call", toolCallId: "tool-1" },
      { kind: "text", text: "Let me run the next probe." },
    ]);
  });
});
