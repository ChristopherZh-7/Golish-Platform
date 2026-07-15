import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { StageRunOrgRows, type StageRunRow } from "./StageRunOrgRows";
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
  };
}

describe("StageRunOrgRows", () => {
  it("renders even a single org as a specialist AI worker boundary", () => {
    render(
      <StageRunOrgRows
        rows={[makeRow()]}
        summary={{ total: 1, covered: 0, active: 1, queued: 0, blocked: 0 }}
        stageLabel="Target Intel"
        roleLabel="Recon"
        coverageAxis={["DNS"]}
        onDrillIn={vi.fn()}
      />
    );

    expect(screen.getByText("Main Agent")).toBeInTheDocument();
    expect(screen.getAllByText(/Recon Agent/).length).toBeGreaterThan(0);
    expect(screen.getByText(/1 worker/)).toBeInTheDocument();
    expect(screen.getByText(/0\/1 passed/)).toBeInTheDocument();
    expect(screen.getByTitle(/打开 Acme Root 的 Recon Agent 详情/)).toBeInTheDocument();
    expect(screen.getByText(/Recon Agent 正在 recon_map_assets/)).toBeInTheDocument();
  });

  it("projects stale running workers as stopped when the parent tool is inactive", () => {
    render(
      <StageRunOrgRows
        rows={[makeRow()]}
        summary={{ total: 1, covered: 0, active: 1, queued: 0, blocked: 0 }}
        stageLabel="Target Intel"
        roleLabel="Recon"
        coverageAxis={["DNS"]}
        isActive={false}
      />
    );

    expect(screen.getByText(/1 stopped/)).toBeInTheDocument();
    expect(screen.getByText("Stopped")).toBeInTheDocument();
    expect(screen.queryByText("Running")).not.toBeInTheDocument();
    expect(screen.queryByText(/Recon Agent 正在 recon_map_assets/)).not.toBeInTheDocument();
  });

  it("opens the org specialist detail when the org row is clicked", () => {
    const drillIn = vi.fn();

    render(
      <StageRunOrgRows
        rows={[makeRow()]}
        summary={{ total: 1, covered: 0, active: 1, queued: 0, blocked: 0 }}
        stageLabel="Target Intel"
        roleLabel="Recon"
        coverageAxis={["DNS", "WHOIS", "ASN", "CT", "Subdomain", "OSINT"]}
        onDrillIn={drillIn}
      />
    );

    fireEvent.click(screen.getByText("Acme Root"));

    expect(drillIn).toHaveBeenCalledWith("tool-1::org::org-1");
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
        summary={{ total: 1, covered: 0, active: 1, queued: 0, blocked: 0 }}
        stageLabel="Target Intel"
        roleLabel="Intel_aggregator"
        coverageAxis={["DNS"]}
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
        summary={{ total: 1, covered: 0, active: 1, queued: 0, blocked: 0 }}
        stageLabel="External Attack Surface"
        roleLabel="Prober"
        coverageAxis={["LIVENESS", "PORT", "SERVICE", "WEB"]}
        teamApi={teamApi}
      />
    );

    expect(screen.queryByText("Main Agent")).not.toBeInTheDocument();
    expect(screen.queryByText(/Prober Agent 正在 eas_discover_ports/)).not.toBeInTheDocument();
    expect(await screen.findByText(/No StageRunUnits exist/)).toBeInTheDocument();
  });

  it("keeps the legacy view when even one row lacks the exact Team pointer", () => {
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
        summary={{ total: 2, covered: 0, active: 2, queued: 0, blocked: 0 }}
        stageLabel="Target Intel"
        roleLabel="Recon"
        coverageAxis={["DNS"]}
      />
    );

    expect(screen.getAllByText(/Recon Agent 正在 recon_map_assets/)).toHaveLength(2);
  });
});
