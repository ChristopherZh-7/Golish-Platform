import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { StageTeamReadModel } from "@/lib/api/stage-team";
import type { ActiveSubAgent } from "@/store";
import {
  STAGE_TRANSCRIPT_RENDER_LIMIT,
  StageTeamWorkspaceView,
} from "./StageTeamWorkspaceView";

function readModel(): StageTeamReadModel {
  const worker = {
    generation: 1,
    agentPath: "main>stage_run:enumeration>js_analyst",
    messageChainId: "chain-js",
    status: "running",
    gateAttempt: 0,
    attemptEpoch: 1,
    checkpointVersion: 2,
    hasActiveTool: true,
    activeToolCallId: "tool-js",
    leaseState: "live",
    recoveryState: "none",
    evidenceWatermark: 41,
    startedAt: "2026-08-02T01:00:00Z",
    updatedAt: "2026-08-02T01:00:05Z",
    terminalAt: null,
  };
  return {
    operationId: "operation-184",
    stageExecutionId: "execution-enum-1",
    stageKind: "enumeration",
    executionStatus: "running",
    startedAt: "2026-08-02T01:00:00Z",
    completedAt: null,
    units: [
      {
        stageRunUnitId: "unit-acme",
        scopeSnapshotId: "scope-acme",
        organizationId: "org-acme",
        organizationName: "Acme Corp",
        stageKind: "enumeration",
        generation: 1,
        specialist: null,
        status: "running",
        startedAt: "2026-08-02T01:00:00Z",
        terminalAt: null,
        gate: {
          status: "running",
          attempt: 1,
          passWatermarkPresent: false,
          finalHandoffId: null,
          finalHandoffSha256: null,
          finalHandoffEvidenceCount: 0,
          evidenceWatermark: 41,
          gatePassedAt: null,
        },
        plan: {
          stageTeamPlanId: "plan-acme",
          schemaVersion: 1,
          planVersion: 1,
          planSha256: "sha256:plan",
          leaderRole: "company_stage_controller",
          aggregatorKind: "team_leader",
          aggregatorRole: "company_stage_controller",
          allowedRoles: ["company_stage_controller", "js_analyst"],
          maxWorkersTotal: 4,
          maxWorkersActive: 2,
          dynamicRequestsEnabled: true,
          dispatchEpoch: 1,
          requestsClosedAt: null,
          finalSubmitterKind: "worker",
          finalSubmitterWorkerRunId: null,
          barrier: {
            dispatchEpoch: 1,
            requestsClosed: false,
            requiredWorkItems: 1,
            terminalRequiredWorkItems: 0,
            liveWorkers: 2,
            retryPendingWorkItems: 0,
            recoveryRequiredWorkers: 0,
            missingOutputs: 0,
            manifestSha256: "sha256:manifest",
            readyToFinalize: false,
          },
          requests: [],
          workItems: [
            {
              workItemId: "controller-item",
              kind: "team_leader",
              stableKey: "leader:primary",
              role: "company_stage_controller",
              inputManifestSha256: "sha256:controller",
              subjectRefCount: 1,
              requiredForBarrier: false,
              isAggregator: true,
              conflictKey: null,
              priority: -100,
              status: "waiting_dependency",
              maxAttempts: 3,
              outputSchema: "stage_unit_aggregate.v1",
              createdBy: "server_seed",
              rowVersion: 1,
              dependencyWorkItemIds: [],
              workers: [
                {
                  ...worker,
                  workerRunId: "controller-worker",
                  specialist: "company_stage_controller",
                  agentPath: "main>stage_run:enumeration>controller",
                  messageChainId: "chain-controller",
                  activeToolCallId: null,
                  hasActiveTool: false,
                },
              ],
              output: null,
              startedAt: "2026-08-02T01:00:00Z",
              terminalAt: null,
            },
            {
              workItemId: "js-item",
              kind: "exact_origin_analysis",
              stableKey: "web-origin:https://x.com/a",
              role: "js_analyst",
              inputManifestSha256: "sha256:js",
              subjectRefCount: 1,
              requiredForBarrier: true,
              isAggregator: false,
              conflictKey: null,
              priority: 0,
              status: "running",
              maxAttempts: 2,
              outputSchema: "stage_worker_output.v1",
              createdBy: "controller",
              rowVersion: 2,
              dependencyWorkItemIds: [],
              workers: [{ ...worker, workerRunId: "js-worker", specialist: "js_analyst" }],
              output: {
                outputId: "output-js",
                workerRunId: "js-worker",
                outputSchema: "stage_worker_output.v1",
                outputVersion: 1,
                businessDisposition: "found",
                canonicalFactRefCount: 3,
                evidenceIds: [41, 42],
                checkedEmptyCellCount: 1,
                blockerCodes: [],
                outputSha256: "sha256:output-js",
                createdAt: "2026-08-02T01:00:05Z",
              },
              startedAt: "2026-08-02T01:00:00Z",
              terminalAt: null,
            },
          ],
        },
      },
    ],
  };
}

function activities(): ActiveSubAgent[] {
  return [
    {
      agentId: "controller-agent",
      agentName: "Controller",
      parentRequestId: "stage-run::lead:controller-worker",
      task: "审核 Enumeration worker 的产物并决定 Gate。",
      depth: 1,
      status: "running",
      toolCalls: [
        {
          id: "dispatch-js",
          name: "stage_team_dispatch_workers",
          args: {
            workers: [
              {
                role: "js_analyst",
                objective: "分析 exact origin https://x.com/a",
                dedupe_key: "js-x-com-a",
              },
            ],
          },
          status: "completed",
          result: {
            requests: [
              {
                dedupe_key: "js-x-com-a",
                decision: "accepted",
                created_work_item_id: "js-item",
              },
            ],
          },
          startedAt: "2026-08-02T01:00:00Z",
          completedAt: "2026-08-02T01:00:01Z",
        },
      ],
      entries: [
        {
          kind: "text",
          text: "Controller 正在等待 JS Analyst 返回。\n\n- **Evidence ledger** 已核对\n- `ready_to_submit=true`",
        },
        { kind: "tool_call", toolCallId: "dispatch-js" },
      ],
      startedAt: "2026-08-02T01:00:00Z",
    },
    {
      agentId: "js-agent",
      agentName: "JS Analyst",
      parentRequestId: "stage-run::worker:js-worker",
      task: "分析 exact origin https://x.com/a 的 JS、接口与参数。",
      depth: 2,
      status: "running",
      toolCalls: [
        {
          id: "plan-js",
          name: "update_plan",
          args: {
            explanation: "先建立接口发现计划，再逐步确认参数。",
            plan: [
              { step: "收集并分类 JS", status: "completed" },
              { step: "定位 API 与主路径", status: "in_progress" },
              { step: "确认请求参数", status: "pending" },
            ],
          },
          status: "completed",
          result: { updated: true },
          startedAt: "2026-08-02T01:00:02Z",
          completedAt: "2026-08-02T01:00:02Z",
        },
        {
          id: "tool-js",
          name: "js_extract_apis",
          args: { origin: "https://x.com/a" },
          status: "running",
          streamingOutput: "found POST /a/b/xxx",
          startedAt: "2026-08-02T01:00:03Z",
        },
      ],
      entries: [
        { kind: "tool_call", toolCallId: "plan-js" },
        {
          kind: "thinking",
          text: "先建立模块依赖和 HTTP client 清单。",
          startedAt: 1_000,
          endedAt: 2_200,
        },
        { kind: "tool_call", toolCallId: "tool-js" },
      ],
      startedAt: "2026-08-02T01:00:01Z",
    },
  ];
}

describe("StageTeamWorkspaceView", () => {
  it("bounds long transcript DOM while pinning the latest plan", () => {
    const model = readModel();
    const activity = activities()[1];
    activity.entries = [
      { kind: "tool_call", toolCallId: "plan-js" },
      ...Array.from({ length: STAGE_TRANSCRIPT_RENDER_LIMIT + 25 }, (_, index) => ({
        kind: "text" as const,
        text: `bounded transcript row ${index}`,
      })),
    ];

    render(
      <StageTeamWorkspaceView
        model={model}
        agentActivities={[activity]}
        agentRequestIdsByWorker={{ "js-worker": "stage-run::worker:js-worker" }}
        focusedAgentRequestId="stage-run::worker:js-worker"
      />
    );

    expect(screen.getByTestId("stage-transcript-omission-notice")).toHaveTextContent(
      "已隐藏较早的 26 条运行记录"
    );
    expect(screen.getByRole("region", { name: "Controller plan" })).toBeInTheDocument();
    expect(screen.getAllByTestId("agent-transcript-message")).toHaveLength(
      STAGE_TRANSCRIPT_RENDER_LIMIT
    );
    expect(screen.queryByText("bounded transcript row 0")).not.toBeInTheDocument();
    expect(screen.getByText("bounded transcript row 224")).toBeInTheDocument();
  });

  it("opens the active child Agent directly and keeps the Controller as a selectable node", () => {
    render(
      <StageTeamWorkspaceView
        model={readModel()}
        agentActivities={activities()}
        agentRequestIdsByWorker={{
          "controller-worker": "stage-run::lead:controller-worker",
          "js-worker": "stage-run::worker:js-worker",
        }}
      />
    );

    expect(screen.getByRole("heading", { name: "Enumeration" })).toBeInTheDocument();
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
    expect(screen.getByText("WEB ORIGIN · https://x.com/a")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Thought for 1.2s" })).toHaveAttribute(
      "aria-expanded",
      "false"
    );
    expect(screen.queryByText("先建立模块依赖和 HTTP client 清单。")).not.toBeInTheDocument();
    expect(screen.getByText("js_extract_apis")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Controller plan" })).toBeInTheDocument();
    expect(screen.getByText("定位 API 与主路径")).toBeInTheDocument();
    expect(screen.getByTestId("stage-team-workspace-layout")).toHaveClass(
      "h-full",
      "min-h-0",
      "overflow-hidden"
    );
    expect(screen.getByTestId("stage-agent-list")).toHaveClass(
      "min-h-0",
      "flex-1",
      "overflow-y-auto"
    );
    expect(screen.getByTestId("stage-agent-conversation")).toHaveClass(
      "min-h-0",
      "flex-1",
      "overflow-y-auto"
    );
    expect(screen.queryByTestId("stage-evidence-inspector")).not.toBeInTheDocument();

    const jsAgentButton = screen.getByRole("button", {
      name: /JS Analyst.*子 Agent/,
    });
    expect(jsAgentButton.closest("[data-parent-agent]")).toHaveAttribute(
      "data-parent-agent",
      "Company Controller"
    );
    expect(screen.getByText(/由 Company Controller 调用/)).toBeInTheDocument();

    const taskToggle = screen.getByTestId("stage-agent-task-toggle");
    expect(taskToggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("stage-agent-task-detail")).not.toBeInTheDocument();
    fireEvent.click(taskToggle);
    expect(taskToggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByTestId("stage-agent-task-detail")).toHaveTextContent(
      "分析 exact origin https://x.com/a 的 JS、接口与参数。"
    );

    const conversation = screen.getByTestId("stage-agent-conversation");
    Object.defineProperty(conversation, "scrollHeight", { configurable: true, value: 900 });
    Object.defineProperty(conversation, "clientHeight", { configurable: true, value: 200 });
    fireEvent.wheel(screen.getByTestId("stage-agent-conversation-surface"), { deltaY: 120 });
    expect(conversation.scrollTop).toBe(120);

    fireEvent.click(screen.getByRole("button", { name: "查看 Evidence" }));
    expect(screen.getByTestId("stage-evidence-inspector")).toHaveClass(
      "h-full",
      "min-h-0",
      "overflow-hidden"
    );
    expect(screen.getByText("evidence #41")).toBeInTheDocument();
    expect(screen.getByText("证据与阶段记忆")).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", {
        name: "Company Controller · 调度根节点 · Controller 正在监控 SubAgent",
      })
    );
    expect(screen.getByText("Controller 正在等待 JS Analyst 返回。")).toBeInTheDocument();
    expect(screen.getByText("Evidence ledger").tagName).toBe("STRONG");
    expect(screen.getByText("ready_to_submit=true").tagName).toBe("CODE");
    expect(screen.getByText("SubAgent 调用")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "切换到子 Agent JS Analyst" }));
    expect(screen.getByRole("region", { name: "Controller plan" })).toBeInTheDocument();
    expect(screen.queryByText(/打开 .*完整运行流/)).not.toBeInTheDocument();
  });

  it("shows durable state honestly when this session has no matching transcript", () => {
    render(<StageTeamWorkspaceView model={readModel()} />);

    expect(
      screen.getByText(/Durable worker state is available, but this session has no visible Agent transcript/)
    ).toBeInTheDocument();
    expect(screen.queryByText("确定性任务 · 无 LLM")).not.toBeInTheDocument();
  });

  it("expands only the live tail Thought even when its latest chunk has an endedAt timestamp", () => {
    const agentActivities = activities();
    const jsActivity = agentActivities[1];
    if (!jsActivity) throw new Error("expected JS activity fixture");
    jsActivity.entries = [
      ...jsActivity.entries,
      {
        kind: "thinking",
        text: "正在分析工具输出并决定下一步。",
        startedAt: 3_000,
        endedAt: 3_450,
      },
    ];

    render(
      <StageTeamWorkspaceView
        model={readModel()}
        agentActivities={agentActivities}
        agentRequestIdsByWorker={{ "js-worker": "stage-run::worker:js-worker" }}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /JS Analyst.*子 Agent/ }));
    expect(screen.getByRole("button", { name: "Thinking" })).toHaveAttribute(
      "aria-expanded",
      "true"
    );
    expect(screen.getByText("正在分析工具输出并决定下一步。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Thought for 1.2s" })).toHaveAttribute(
      "aria-expanded",
      "false"
    );
  });

  it("keeps the full Agent timeline and renders supplementary stage details inside evidence", () => {
    const agentActivities = activities();
    const jsActivity = agentActivities[1];
    if (!jsActivity) throw new Error("expected JS activity fixture");
    jsActivity.attemptEntryStart = jsActivity.entries.length;
    jsActivity.entries = [
      ...jsActivity.entries,
      { kind: "text", text: "第三条事件" },
      { kind: "thinking", text: "第四条事件" },
      { kind: "text", text: "第五条事件仍然可见" },
    ];

    render(
      <StageTeamWorkspaceView
        model={readModel()}
        agentActivities={agentActivities}
        agentRequestIdsByWorker={{ "js-worker": "stage-run::worker:js-worker" }}
      >
        <div>阶段补充诊断</div>
      </StageTeamWorkspaceView>
    );

    fireEvent.click(screen.getByRole("button", { name: /JS Analyst.*子 Agent/ }));
    const thought = screen.getByRole("button", { name: "Thought for 1.2s" });
    expect(thought).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("先建立模块依赖和 HTTP client 清单。")).not.toBeInTheDocument();
    fireEvent.click(thought);
    expect(screen.getByText("先建立模块依赖和 HTTP client 清单。")).toBeInTheDocument();
    expect(screen.getByText("第五条事件仍然可见")).toBeInTheDocument();
    expect(screen.queryByTestId("stage-evidence-inspector")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "查看 Evidence" }));
    expect(screen.getByTestId("stage-evidence-inspector")).toContainElement(
      screen.getByText("阶段补充诊断")
    );
  });

  it("applies a historical child focus once without stealing selection back from the Controller", () => {
    const agentActivities = activities();
    const view = render(
      <StageTeamWorkspaceView
        model={readModel()}
        agentActivities={agentActivities}
        agentRequestIdsByWorker={{
          "controller-worker": "stage-run::lead:controller-worker",
          "js-worker": "stage-run::worker:js-worker",
        }}
        focusedAgentRequestId="stage-run::worker:js-worker"
      />
    );

    expect(screen.getByText("WEB ORIGIN · https://x.com/a")).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", {
        name: "Company Controller · 调度根节点 · Controller 正在监控 SubAgent",
      })
    );
    expect(screen.getByText("Controller 正在等待 JS Analyst 返回。")).toBeInTheDocument();

    view.rerender(
      <StageTeamWorkspaceView
        model={readModel()}
        agentActivities={agentActivities.map((activity) => ({ ...activity }))}
        agentRequestIdsByWorker={{
          "controller-worker": "stage-run::lead:controller-worker",
          "js-worker": "stage-run::worker:js-worker",
        }}
        focusedAgentRequestId="stage-run::worker:js-worker"
      />
    );

    expect(screen.getByText("Controller 正在等待 JS Analyst 返回。")).toBeInTheDocument();
    expect(screen.queryByText("WEB ORIGIN · https://x.com/a")).toBeInTheDocument();
  });
});
