import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ActiveSubAgent } from "@/store";
import {
  StageRunOrgRows,
  type StageRunRow,
  stageTeamAgentRefreshSignature,
} from "./StageRunOrgRows";
import type { StageTeamReadApi } from "./StageTeamRunView";

function makeRow(): StageRunRow {
  return {
    id: "org-1",
    name: "Acme Root",
    ownershipPercent: null,
    status: "running",
    agentRequestId: "tool-1::org::org-1",
    activity: "recon_map_assets",
    evidenceCount: 2,
    coverage: { DNS: "found" },
    stage: "target_intel",
  };
}

describe("StageRunOrgRows", () => {
  it("refreshes exact Team truth for dispatch/lifecycle changes but ignores transcript deltas", () => {
    const row = {
      ...makeRow(),
      agentRequestId: "stage-call::team::org-1",
    };
    const controller: ActiveSubAgent = {
      agentId: "controller-1",
      agentName: "Company Controller",
      parentRequestId: "stage-call::team::org-1::lead:worker-1",
      task: "Coordinate this company",
      depth: 1,
      status: "running",
      toolCalls: [
        {
          id: "dispatch-1",
          name: "stage_team_dispatch_workers",
          args: {},
          status: "running",
          startedAt: "2026-08-02T00:00:00Z",
        },
      ],
      entries: [{ kind: "text", text: "dispatching" }],
      startedAt: "2026-08-02T00:00:00Z",
    };
    const runningSignature = stageTeamAgentRefreshSignature([row], [controller]);
    const textOnlySignature = stageTeamAgentRefreshSignature(
      [row],
      [{ ...controller, entries: [{ kind: "text", text: "more streamed text" }] }]
    );
    const completedSignature = stageTeamAgentRefreshSignature(
      [row],
      [
        {
          ...controller,
          toolCalls: [
            {
              ...controller.toolCalls[0],
              status: "completed",
              completedAt: "2026-08-02T00:00:01Z",
            },
          ],
        },
      ]
    );

    expect(textOnlySignature).toBe(runningSignature);
    expect(completedSignature).not.toBe(runningSignature);
    expect(
      stageTeamAgentRefreshSignature(
        [row],
        [{ ...controller, parentRequestId: "another-stage::team::org-1::lead:worker-1" }]
      )
    ).toBe("");
  });

  it("renders only the DB-backed Team view when an exact Team pointer exists", async () => {
    const row: StageRunRow = {
      ...makeRow(),
      operationId: "operation-1",
      stageExecutionId: "execution-1",
      stageRunUnitId: "unit-1",
    };
    const teamApi: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue({
        operationId: "operation-1",
        stageExecutionId: "execution-1",
        stageKind: "target_intel",
        executionStatus: "started",
        startedAt: "2026-07-15T00:00:00Z",
        completedAt: null,
        units: [],
      }),
      resolveRecovery: vi.fn(),
    };

    render(
      <StageRunOrgRows
        rows={[row]}
        teamApi={teamApi}
      />
    );

    expect(screen.queryByText(/Intel_aggregator Agent 正在/)).not.toBeInTheDocument();
    expect(await screen.findByText(/No StageRunUnits exist/)).toBeInTheDocument();
  });

  it("routes a downstream EAS exact Team pointer through the same Controller view", async () => {
    const row: StageRunRow = {
      ...makeRow(),
      operationId: "operation-eas",
      stageExecutionId: "execution-eas",
      stageRunUnitId: "unit-eas",
      stage: "external_attack_surface",
      activity: "eas_discover_ports",
      coverage: { PORT: "pending" },
    };
    const teamApi: StageTeamReadApi = {
      getReadModel: vi.fn().mockResolvedValue({
        operationId: "operation-eas",
        stageExecutionId: "execution-eas",
        stageKind: "external_attack_surface",
        executionStatus: "started",
        startedAt: "2026-07-16T00:00:00Z",
        completedAt: null,
        units: [],
      }),
      resolveRecovery: vi.fn(),
    };

    render(
      <StageRunOrgRows
        rows={[row]}
        teamApi={teamApi}
      />
    );

    expect(screen.queryByText("Main Agent")).not.toBeInTheDocument();
    expect(screen.queryByText(/Prober Agent 正在 eas_discover_ports/)).not.toBeInTheDocument();
    expect(await screen.findByText(/No StageRunUnits exist/)).toBeInTheDocument();
  });

  it("requires a rerun instead of restoring the legacy specialist view", () => {
    render(<StageRunOrgRows rows={[makeRow()]} />);

    expect(screen.getByTestId("stage-team-rerun-required")).toBeInTheDocument();
    expect(screen.getByText(/Company Controller data unavailable/)).toBeInTheDocument();
    expect(screen.getByText(/new V2 run and rerun this stage/)).toBeInTheDocument();
    expect(screen.queryByText("Main Agent")).not.toBeInTheDocument();
    expect(screen.queryByText(/Recon Agent/)).not.toBeInTheDocument();
    expect(screen.queryByText("Acme Root")).not.toBeInTheDocument();
  });

  it("fails the whole mixed snapshot closed when even one row lacks an exact Team pointer", () => {
    const pointed: StageRunRow = {
      ...makeRow(),
      operationId: "operation-1",
      stageExecutionId: "execution-1",
      stageRunUnitId: "unit-1",
    };
    const unpointed: StageRunRow = {
      ...makeRow(),
      id: "org-2",
      name: "Acme Child",
      agentRequestId: "tool-1::org::org-2",
    };

    render(
      <StageRunOrgRows
        rows={[pointed, unpointed]}
      />
    );

    expect(screen.getByTestId("stage-team-rerun-required")).toBeInTheDocument();
    expect(screen.queryByText("Main Agent")).not.toBeInTheDocument();
    expect(screen.queryByText(/Recon Agent/)).not.toBeInTheDocument();
  });

  it("leaves non-company stages to their separate typed views", () => {
    const candidate = {
      ...makeRow(),
      stage: "attack_candidate",
      operationId: "operation-candidate",
      stageExecutionId: "execution-candidate",
    };

    const { container } = render(<StageRunOrgRows rows={[candidate]} />);

    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByTestId("stage-team-rerun-required")).not.toBeInTheDocument();
  });
});
