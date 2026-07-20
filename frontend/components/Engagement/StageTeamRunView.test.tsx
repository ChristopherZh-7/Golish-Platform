import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { StageAssetCoverageSnapshot } from "@/lib/api/stage-coverage";
import type { StageTeamReadModel } from "@/lib/api/stage-team";
import { type StageTeamReadApi, StageTeamRunView } from "./StageTeamRunView";

function model(): StageTeamReadModel {
  return {
    operationId: "operation-1",
    stageExecutionId: "execution-1",
    stageKind: "target_intel",
    executionStatus: "started",
    startedAt: "2026-07-14T00:00:00Z",
    completedAt: null,
    units: [
      {
        stageRunUnitId: "unit-1",
        scopeSnapshotId: "snapshot-1",
        organizationId: "org-1",
        organizationName: "Acme Root",
        stageKind: "target_intel",
        generation: 1,
        specialist: "intel_provider",
        status: "running",
        startedAt: "2026-07-14T00:00:01Z",
        terminalAt: null,
        gate: {
          status: "running",
          attempt: 0,
          passWatermarkPresent: false,
          finalHandoffId: null,
          finalHandoffSha256: null,
          finalHandoffEvidenceCount: 0,
          evidenceWatermark: null,
          gatePassedAt: null,
        },
        plan: {
          stageTeamPlanId: "plan-1",
          schemaVersion: 1,
          planVersion: 1,
          planSha256: `sha256:${"a".repeat(64)}`,
          leaderRole: "company_stage_controller",
          aggregatorKind: "worker",
          aggregatorRole: "company_stage_controller",
          allowedRoles: [
            "company_stage_controller",
            "intel_provider",
            "intel_coverage_critic",
          ],
          maxWorkersTotal: 4,
          maxWorkersActive: 2,
          dynamicRequestsEnabled: true,
          dispatchEpoch: 0,
          requestsClosedAt: null,
          finalSubmitterKind: "worker",
          finalSubmitterWorkerRunId: null,
          barrier: {
            dispatchEpoch: 0,
            requestsClosed: false,
            requiredWorkItems: 1,
            terminalRequiredWorkItems: 0,
            liveWorkers: 1,
            retryPendingWorkItems: 0,
            recoveryRequiredWorkers: 0,
            missingOutputs: 0,
            manifestSha256: `sha256:${"b".repeat(64)}`,
            readyToFinalize: false,
          },
          requests: [
            {
              requestId: "request-1",
              parentWorkItemId: "item-1",
              parentWorkerRunId: "worker-1",
              dispatchEpoch: 0,
              requestedRole: "intel_provider",
              requestKind: "provider_followup",
              subjectRefCount: 1,
              reasonCode: "coverage_gap",
              expectedOutputSchema: "stage_worker_output.v1",
              dedupeKey: "followup-1",
              requestSha256: `sha256:${"c".repeat(64)}`,
              status: "accepted",
              decisionReasonCode: null,
              acceptedWorkItemId: "item-2",
              createdAt: "2026-07-14T00:00:02Z",
            },
          ],
          workItems: [
            {
              workItemId: "item-1",
              kind: "provider",
              stableKey: "provider:fofa",
              role: "intel_provider",
              inputManifestSha256: `sha256:${"d".repeat(64)}`,
              subjectRefCount: 1,
              requiredForBarrier: true,
              isAggregator: false,
              conflictKey: null,
              priority: 0,
              status: "running",
              maxAttempts: 2,
              outputSchema: "stage_worker_output.v1",
              createdBy: "server_seed",
              rowVersion: 3,
              dependencyWorkItemIds: [],
              startedAt: "2026-07-14T00:00:02Z",
              terminalAt: null,
              workers: [
                {
                  workerRunId: "worker-1",
                  generation: 1,
                  specialist: "intel_provider",
                  agentPath: "main>stage_run:target_intel>intel_provider",
                  messageChainId: "chain-1",
                  status: "running",
                  gateAttempt: 0,
                  attemptEpoch: 1,
                  checkpointVersion: 4,
                  hasActiveTool: true,
                  activeToolCallId: "tool-call-1",
                  leaseState: "live",
                  recoveryState: "wait_for_live_lease",
                  evidenceWatermark: null,
                  startedAt: "2026-07-14T00:00:02Z",
                  updatedAt: "2026-07-14T00:00:03Z",
                  terminalAt: null,
                },
              ],
              output: null,
            },
          ],
        },
      },
    ],
  };
}

function addCompanyController(
  readModel: StageTeamReadModel,
  status: "queued" | "running" | "waiting_dependency" = "running"
) {
  const plan = readModel.units[0].plan!;
  const workerTemplate = plan.workItems[0].workers[0];
  plan.workItems.unshift({
    workItemId: "leader-item",
    kind: "team_leader",
    stableKey: "leader:primary",
    role: "company_stage_controller",
    inputManifestSha256: `sha256:${"8".repeat(64)}`,
    subjectRefCount: 1,
    requiredForBarrier: false,
    isAggregator: true,
    conflictKey: null,
    priority: -100,
    status,
    maxAttempts: 2,
    outputSchema: "stage_team_leader.v1",
    createdBy: "server_seed",
    rowVersion: 1,
    dependencyWorkItemIds: [],
    startedAt: status === "queued" ? null : "2026-07-14T00:00:01Z",
    terminalAt: null,
    workers:
      status !== "queued"
        ? [
            {
              ...workerTemplate,
              workerRunId: "leader-worker",
              specialist: "recon",
              agentPath: "main>stage_run:target_intel>company_controller",
              messageChainId: "leader-chain",
            },
          ]
        : [],
    output: null,
  });
}

describe("StageTeamRunView", () => {
  it("shows DB-authoritative Vuln coverage and separates failed, retry, and operator lanes", async () => {
    const vuln = model();
    vuln.stageKind = "vuln_triage";
    vuln.units[0].stageKind = "vuln_triage";
    vuln.units[0].specialist = "vuln_scanner";
    addCompanyController(vuln);
    const child = vuln.units[0].plan!.workItems[1];
    const historical = {
      ...child.workers[0],
      workerRunId: "worker-history",
      status: "failed" as const,
      hasActiveTool: false,
      activeToolCallId: null,
      recoveryState: "none",
      terminalAt: "2026-07-14T00:00:04Z",
    };
    child.status = "retry_pending";
    child.workers[0].generation = 2;
    child.workers[0].attemptEpoch = 2;
    child.workers.unshift(historical);
    vuln.units[0].plan!.workItems.push({
      ...child,
      workItemId: "item-recovery",
      stableKey: "vuln:recovery",
      status: "recovery_required",
      workers: [
        {
          ...child.workers[1],
          workerRunId: "worker-recovery",
          status: "recovery_required",
          recoveryState: "manual_required",
        },
      ],
    });
    const coverage: StageAssetCoverageSnapshot = {
      stage: "vuln_triage",
      organization_id: "org-1",
      session_id: "operation-1",
      summary: {
        total_assets: 1,
        seed_assets: 1,
        new_assets: 0,
        done_assets: 0,
        pending_assets: 1,
        blocked_assets: 0,
      },
      assets: [
        {
          target_id: "target-1",
          value: "https://example.test:443",
          target_type: "url",
          real_ip: "",
          source: "enumeration",
          discovered_phase: "enumeration",
          created_at: "2026-07-14T00:00:00Z",
          parent_id: null,
          coverage: Array.from({ length: 360 }, (_, index) => ({
            technique: `TECH-${index}`,
            label: `Technique ${index}`,
            state: index < 340 ? "checked_empty" : index < 350 ? "partial" : "error",
            source: "vuln_nuclei_general",
            evidence_refs: [index + 1],
            note: null,
            suggested_tools: [],
          })),
        },
      ],
    };
    const api: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue(vuln),
      getCoverage: vi.fn().mockResolvedValue(coverage),
      resolveRecovery: vi.fn(),
    };

    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={api}
      />
    );

    expect(await screen.findByText("漏洞扫描进度")).toBeInTheDocument();
    expect(screen.getByText("漏洞扫描调度器")).toBeInTheDocument();
    expect(screen.getByText("2 个扫描分片")).toBeInTheDocument();
    expect(screen.getByText("证据覆盖")).toBeInTheDocument();
    expect(screen.getByText("20 待检查")).toBeInTheDocument();
    expect(screen.queryByText("Company Controller")).not.toBeInTheDocument();
    expect(screen.queryByText(/个 SubAgent/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /调度详情|扫描队列/ })
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Plan v1")).not.toBeInTheDocument();
    expect(screen.queryByText(/active workers max/)).not.toBeInTheDocument();
    expect(screen.queryByText(/chain leader-chain/)).not.toBeInTheDocument();
    expect(screen.getByText("历史失败 20 cells")).toBeInTheDocument();
    expect(screen.getByText("340/360 cells 终态 · 剩余 20")).toBeInTheDocument();
    expect(screen.getByText("340/360 cells · 剩余 20")).toBeInTheDocument();
    expect(screen.getByText("自动重试 1")).toBeInTheDocument();
    expect(screen.getByText("待人工恢复 1")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "检测到上次运行中断" })
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "解除中断状态" })).toBeInTheDocument();
    expect(screen.getByText(/不会自动重放/)).toBeInTheDocument();
    expect(api.getCoverage).toHaveBeenCalledWith({
      operationId: "operation-1",
      organizationId: "org-1",
      stage: "vuln_triage",
      stageStartedAt: "2026-07-14T00:00:00Z",
    });
  });

  it("keeps Vuln recovery reachable and guides continue or reset after exact recovery", async () => {
    const recoveryModel = model();
    recoveryModel.stageKind = "vuln_triage";
    recoveryModel.units[0].stageKind = "vuln_triage";
    addCompanyController(recoveryModel);
    const recoveryItem = recoveryModel.units[0].plan!.workItems[1];
    const recoveryWorker = recoveryItem.workers[0];
    recoveryItem.status = "recovery_required";
    recoveryWorker.status = "recovery_required";
    recoveryWorker.leaseState = "expired";
    recoveryWorker.recoveryState = "manual_required";
    recoveryModel.units[0].plan!.barrier.recoveryRequiredWorkers = 1;

    const resolvedModel = structuredClone(recoveryModel);
    const resolvedItem = resolvedModel.units[0].plan!.workItems[1];
    const resolvedWorker = resolvedItem.workers[0];
    resolvedItem.status = "exhausted";
    resolvedWorker.status = "failed";
    resolvedWorker.hasActiveTool = false;
    resolvedWorker.activeToolCallId = null;
    resolvedWorker.recoveryState = "none";
    resolvedItem.output = {
      outputId: "output-1",
      workerRunId: resolvedWorker.workerRunId,
      outputSchema: resolvedItem.outputSchema,
      outputVersion: 1,
      businessDisposition: "blocked",
      canonicalFactRefCount: 0,
      evidenceIds: [],
      checkedEmptyCellCount: 0,
      blockerCodes: ["STAGE_TEAM_ACTIVE_TOOL_RECOVERY_BLOCKED"],
      outputSha256: `sha256:${"f".repeat(64)}`,
      createdAt: "2026-07-14T00:00:05Z",
    };
    resolvedModel.units[0].plan!.barrier.recoveryRequiredWorkers = 0;

    const getReadModel = vi
      .fn()
      .mockResolvedValueOnce(recoveryModel)
      .mockResolvedValueOnce(resolvedModel);
    const resolveRecovery = vi.fn().mockResolvedValue({
      decisionId: "decision-1",
      decisionSha256: `sha256:${"e".repeat(64)}`,
      workItemStatus: "exhausted",
      workerStatus: "failed",
      outputId: "output-1",
      blockerCode: "STAGE_TEAM_ACTIVE_TOOL_RECOVERY_BLOCKED",
      replayed: false,
    });

    const active = render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={{ getReadModel, resolveRecovery }}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "解除中断状态" }));
    await waitFor(() => expect(resolveRecovery).toHaveBeenCalledTimes(1));
    expect(resolveRecovery.mock.calls[0][0]).toMatchObject({
      operationId: "operation-1",
      stageExecutionId: "execution-1",
      stageRunUnitId: "unit-1",
      scopeSnapshotId: "snapshot-1",
      stageTeamPlanId: "plan-1",
      workItemId: "item-1",
      workerRunId: "worker-1",
      toolCallRecordId: "tool-call-1",
      expectedWorkItemRowVersion: 3,
      expectedCheckpointVersion: 4,
      expectedAttemptEpoch: 1,
    });
    expect(
      await screen.findByText(/发送“继续”恢复剩余任务，或使用“重置阶段”/)
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "解除中断状态" })).not.toBeInTheDocument();

    active.unmount();
    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={{ getReadModel: vi.fn().mockResolvedValue(resolvedModel), resolveRecovery }}
      />
    );
    expect(
      await screen.findByText(/此前的中断项已安全记为结果未知且不会重放/)
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "解除中断状态" })).not.toBeInTheDocument();
  });

  it("renders Vuln worklist loading, error, and empty states separately", async () => {
    const vuln = model();
    vuln.stageKind = "vuln_triage";
    vuln.units[0].stageKind = "vuln_triage";
    addCompanyController(vuln);
    const pending = render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={{
          getReadModel: vi.fn().mockResolvedValue(vuln),
          getCoverage: vi.fn().mockReturnValue(new Promise(() => undefined)),
          resolveRecovery: vi.fn(),
        }}
      />
    );
    expect(await screen.findByText("正在读取扫描进度")).toBeInTheDocument();
    pending.unmount();

    const failed = render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={{
          getReadModel: vi.fn().mockResolvedValue(vuln),
          getCoverage: vi.fn().mockRejectedValue(new Error("coverage unavailable")),
          resolveRecovery: vi.fn(),
        }}
      />
    );
    expect(await screen.findByText("扫描进度读取失败：coverage unavailable")).toBeInTheDocument();
    failed.unmount();

    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={{
          getReadModel: vi.fn().mockResolvedValue(vuln),
          getCoverage: vi.fn().mockResolvedValue({
            stage: "vuln_triage",
            organization_id: "org-1",
            session_id: "operation-1",
            summary: {
              total_assets: 0,
              seed_assets: 0,
              new_assets: 0,
              done_assets: 0,
              pending_assets: 0,
              blocked_assets: 0,
            },
            assets: [],
          }),
          resolveRecovery: vi.fn(),
        }}
      />
    );
    expect(await screen.findByText("当前没有待扫描项")).toBeInTheDocument();
  });

  it("renders Unit to Plan to WorkItem to Worker and Request/Barrier from DB truth", async () => {
    const controllerModel = model();
    addCompanyController(controllerModel);
    const api: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue(controllerModel),
      resolveRecovery: vi.fn(),
    };
    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={api}
        onOpenAgent={vi.fn()}
      />
    );

    expect(await screen.findByText("Acme Root")).toBeInTheDocument();
    expect(screen.getByText("采集 0/1 已返回")).toBeInTheDocument();
    expect(screen.getByText("1 个采集中")).toBeInTheDocument();
    expect(screen.getByText("阶段未完成")).toBeInTheDocument();
    expect(screen.queryByText("Plan v1")).not.toBeInTheDocument();
    expect(screen.queryByText(/epoch 0/)).not.toBeInTheDocument();
    expect(screen.queryByText(/schema stage_worker_output.v1/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "展开调度详情" }));
    expect(screen.getByText("Plan v1")).toBeInTheDocument();
    expect(screen.getByText("2 active workers max")).toBeInTheDocument();
    expect(screen.queryByText("2 active / 4 total")).not.toBeInTheDocument();
    expect(screen.getByText("intel_provider")).toBeInTheDocument();
    expect(screen.getByText(/Worker worker-1/)).toBeInTheDocument();
    expect(screen.getByText(/Barrier waiting/)).toBeInTheDocument();
    expect(screen.getByText(/provider_followup/)).toBeInTheDocument();
    expect(api.getReadModel).toHaveBeenCalledWith({
      operationId: "operation-1",
      stageExecutionId: "execution-1",
    });
  });

  it("counts an invalid child output as blocked without exposing its old direct flow", async () => {
    const blocked = model();
    addCompanyController(blocked);
    const item = blocked.units[0].plan!.workItems[1];
    item.status = "completed";
    item.workers[0].status = "passed";
    item.output = {
      outputId: "output-1",
      workerRunId: "worker-1",
      outputSchema: "stage_worker_output.v1",
      outputVersion: 1,
      businessDisposition: "blocked",
      canonicalFactRefCount: 0,
      evidenceIds: [],
      checkedEmptyCellCount: 0,
      blockerCodes: ["STAGE_TEAM_WORKER_OUTPUT_INVALID"],
      outputSha256: `sha256:${"f".repeat(64)}`,
      createdAt: "2026-07-14T00:00:04Z",
    };
    const api: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue(blocked),
      resolveRecovery: vi.fn(),
    };

    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={api}
      />
    );

    expect(await screen.findByText("Company Controller")).toBeInTheDocument();
    expect(screen.getByText("1 个阻塞")).toBeInTheDocument();
    expect(screen.queryByText("采集 1/1 已返回")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /PROVIDER Agent 运行流/ })).not.toBeInTheDocument();
  });

  it("rejects a fixed Team run without leader:primary and exposes no legacy Agent flow", async () => {
    const unsupportedReadModel = model();
    const openAgent = vi.fn();
    const api: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue(unsupportedReadModel),
      resolveRecovery: vi.fn(),
    };

    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={api}
        agentRequestIdsByWorker={{ "worker-1": "tool-1::team::org-1::worker:worker-1" }}
        onOpenAgent={openAgent}
      />
    );

    expect(await screen.findByRole("status")).toHaveTextContent(
      "旧版固定 Team 运行已不再支持，请重新运行本阶段以启动 Company Controller。"
    );
    expect(screen.queryByRole("button", { name: /运行流/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "展开调度详情" }));
    expect(screen.queryByRole("button", { name: /运行流/ })).not.toBeInTheDocument();
    expect(openAgent).not.toHaveBeenCalled();
  });

  it("renders a lead:primary Company Controller as the sole default flow entry", async () => {
    const controllerModel = model();
    addCompanyController(controllerModel);
    const child = controllerModel.units[0].plan!.workItems[1];
    child.status = "completed";
    child.workers[0].status = "passed";
    child.workers[0].hasActiveTool = false;
    child.workers[0].activeToolCallId = null;
    controllerModel.units[0].plan!.workItems.push({
      ...child,
      workItemId: "child-item-2",
      stableKey: "child:whois",
      status: "running",
      workers: [
        {
          ...child.workers[0],
          workerRunId: "child-worker-2",
          status: "running",
        },
      ],
    });
    const openAgent = vi.fn();
    const api: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue(controllerModel),
      resolveRecovery: vi.fn(),
    };

    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={api}
        agentRequestIdsByWorker={{
          "leader-worker": "tool-1::team::org-1::lead:leader-worker",
          "worker-1": "tool-1::team::org-1::worker:worker-1",
          "child-worker-2": "tool-1::team::org-1::worker:child-worker-2",
        }}
        onOpenAgent={openAgent}
      />
    );

    expect(await screen.findByText("Company Controller")).toBeInTheDocument();
    expect(screen.getByText("2 个 SubAgent")).toBeInTheDocument();
    expect(screen.getByText("1 个运行中")).toBeInTheDocument();
    expect(screen.getByText("1 个已完成")).toBeInTheDocument();
    expect(screen.getByText("Gate 未完成")).toBeInTheDocument();
    expect(screen.queryByText("PROVIDER")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "查看 Controller 运行流" }));
    expect(openAgent).toHaveBeenCalledWith(
      "tool-1::team::org-1::lead:leader-worker"
    );
  });

  it("keeps a queued Company Controller waiting without opening a child identity", async () => {
    const queued = model();
    addCompanyController(queued, "queued");
    const child = queued.units[0].plan!.workItems[1];
    child.status = "running";
    const openAgent = vi.fn();
    const api: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue(queued),
      resolveRecovery: vi.fn(),
    };

    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={api}
        agentRequestIdsByWorker={{
          "worker-1": "tool-1::team::org-1::worker:worker-1",
        }}
        onOpenAgent={openAgent}
      />
    );

    expect(await screen.findByText("Controller 排队中")).toBeInTheDocument();
    expect(screen.getByText("Gate 未完成")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "查看 Controller 运行流" })).not.toBeInTheDocument();
    expect(openAgent).not.toHaveBeenCalled();
  });

  it("shows a dependency-waiting Controller as continuously monitoring its SubAgents", async () => {
    const monitoring = model();
    addCompanyController(monitoring, "waiting_dependency");
    const api: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue(monitoring),
      resolveRecovery: vi.fn(),
    };

    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={api}
      />
    );

    expect(await screen.findByText("Controller 正在监控 SubAgent")).toBeInTheDocument();
    expect(screen.queryByText("Controller 排队中")).not.toBeInTheDocument();
  });

  it("shows authoritative business states only after the Unit is final-sealed", async () => {
    const passed = model();
    addCompanyController(passed);
    const unit = passed.units[0];
    const item = unit.plan!.workItems[1];
    item.stableKey = "child:asn";
    item.status = "completed";
    item.workers[0].status = "passed";
    item.output = {
      outputId: "output-asn",
      workerRunId: "worker-1",
      outputSchema: "stage_worker_output.v1",
      outputVersion: 1,
      businessDisposition: "checked_empty",
      canonicalFactRefCount: 0,
      evidenceIds: [41],
      checkedEmptyCellCount: 1,
      blockerCodes: [],
      outputSha256: `sha256:${"f".repeat(64)}`,
      createdAt: "2026-07-14T00:00:04Z",
    };
    unit.status = "passed";
    unit.gate.status = "passed";
    unit.gate.finalHandoffId = "handoff-1";
    unit.gate.finalHandoffEvidenceCount = 1;
    unit.gate.gatePassedAt = "2026-07-14T00:00:06Z";
    const api: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue(passed),
      resolveRecovery: vi.fn(),
    };

    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={api}
      />
    );

    expect(await screen.findByText("Gate 已通过")).toBeInTheDocument();
    expect(screen.getByText("阶段已通过")).toBeInTheDocument();
    expect(screen.queryByText("已查空")).not.toBeInTheDocument();
  });

  it("renders explicit loading, error and empty states", async () => {
    const pendingApi: StageTeamReadApi = {
      getReadModel: vi.fn().mockReturnValue(new Promise(() => undefined)),
      resolveRecovery: vi.fn(),
    };
    const loading = render(
      <StageTeamRunView operationId="op" stageExecutionId="execution" api={pendingApi} />
    );
    expect(screen.getByText("Loading durable Team scheduler…")).toBeInTheDocument();
    loading.unmount();

    const errorApi: StageTeamReadApi = {
      getReadModel: vi.fn().mockRejectedValue(new Error("DB unavailable")),
      resolveRecovery: vi.fn(),
    };
    const failed = render(
      <StageTeamRunView operationId="op" stageExecutionId="execution" api={errorApi} />
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("DB unavailable");
    failed.unmount();

    const empty = model();
    empty.units = [];
    const emptyApi: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue(empty),
      resolveRecovery: vi.fn(),
    };
    render(<StageTeamRunView operationId="op" stageExecutionId="execution" api={emptyApi} />);
    expect(await screen.findByText(/No StageRunUnits exist/)).toBeInTheDocument();
  });

  it("resolves only manual active-tool recovery with a stable request id and never-replay warning", async () => {
    const recoveryModel = model();
    addCompanyController(recoveryModel);
    const unit = recoveryModel.units[0];
    const plan = unit.plan!;
    const item = plan.workItems[1];
    const worker = item.workers[0];
    item.status = "recovery_required";
    worker.status = "recovery_required";
    worker.leaseState = "expired";
    worker.recoveryState = "manual_required";
    plan.barrier.recoveryRequiredWorkers = 1;

    let rejectFirst!: (error: Error) => void;
    const firstAttempt = new Promise<never>((_resolve, reject) => {
      rejectFirst = reject;
    });
    const resolveRecovery = vi
      .fn()
      .mockReturnValueOnce(firstAttempt)
      .mockResolvedValueOnce({
        decisionId: "decision-1",
        decisionSha256: `sha256:${"e".repeat(64)}`,
        workItemStatus: "exhausted",
        workerStatus: "failed",
        outputId: "output-1",
        blockerCode: "STAGE_TEAM_ACTIVE_TOOL_RECOVERY_BLOCKED",
        replayed: true,
      });
    const getReadModel = vi.fn().mockResolvedValue(recoveryModel);
    const api: StageTeamReadApi = { getReadModel, resolveRecovery };
    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={api}
      />
    );

    const action = await screen.findByRole("button", {
      name: "解除中断状态",
    });
    fireEvent.click(action);
    expect(
      screen.getByRole("button", {
        name: "正在安全封存未知结果…",
      })
    ).toBeDisabled();

    await act(async () => rejectFirst(new Error("response lost")));
    expect(await screen.findByRole("alert")).toHaveTextContent("response lost");
    const firstRequest = resolveRecovery.mock.calls[0][0];
    expect(firstRequest).toMatchObject({
      operationId: "operation-1",
      stageExecutionId: "execution-1",
      stageRunUnitId: "unit-1",
      scopeSnapshotId: "snapshot-1",
      stageTeamPlanId: "plan-1",
      workItemId: "item-1",
      workerRunId: "worker-1",
      toolCallRecordId: "tool-call-1",
      expectedWorkItemRowVersion: 3,
      expectedCheckpointVersion: 4,
      expectedAttemptEpoch: 1,
    });

    fireEvent.click(
      screen.getByRole("button", {
        name: "重试解除中断状态",
      })
    );
    await waitFor(() => expect(resolveRecovery).toHaveBeenCalledTimes(2));
    expect(resolveRecovery.mock.calls[1][0].requestId).toBe(firstRequest.requestId);
    await waitFor(() => expect(getReadModel).toHaveBeenCalledTimes(2));
  });

  it("keeps multiple interrupted workers independently recoverable", async () => {
    const recoveryModel = model();
    const plan = recoveryModel.units[0].plan!;
    const firstItem = plan.workItems[0];
    const firstWorker = firstItem.workers[0];
    firstItem.status = "recovery_required";
    firstWorker.status = "recovery_required";
    firstWorker.leaseState = "expired";
    firstWorker.recoveryState = "manual_required";

    const secondItem = structuredClone(firstItem);
    secondItem.workItemId = "item-2";
    secondItem.stableKey = "provider:quake";
    secondItem.role = "intel_coverage_critic";
    secondItem.workers[0].workerRunId = "worker-2";
    secondItem.workers[0].activeToolCallId = "tool-call-2";
    plan.workItems.push(secondItem);
    plan.barrier.recoveryRequiredWorkers = 2;

    const firstAttempt = new Promise<never>(() => undefined);
    const resolveRecovery = vi.fn((request: { workerRunId: string }) => {
      if (request.workerRunId === "worker-1") return firstAttempt;
      return Promise.reject(new Error("second recovery failed"));
    });
    render(
      <StageTeamRunView
        operationId="operation-1"
        stageExecutionId="execution-1"
        api={{ getReadModel: vi.fn().mockResolvedValue(recoveryModel), resolveRecovery }}
      />
    );

    const actions = await screen.findAllByRole("button", { name: "解除中断状态" });
    expect(actions).toHaveLength(2);
    fireEvent.click(actions[0]);
    expect(screen.getByRole("button", { name: "正在安全封存未知结果…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "解除中断状态" })).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: "解除中断状态" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("second recovery failed");
    expect(screen.getByRole("button", { name: "正在安全封存未知结果…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "重试解除中断状态" })).toBeEnabled();
  });
});
