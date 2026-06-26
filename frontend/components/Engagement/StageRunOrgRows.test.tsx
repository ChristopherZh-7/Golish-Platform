import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { StageRunOrgRows, type StageRunRow } from "./StageRunOrgRows";

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
});
