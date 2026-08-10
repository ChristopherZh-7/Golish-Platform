import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StageRunDetailShell } from "./StageRunDetailShell";

describe("StageRunDetailShell", () => {
  it("renders the stage selected by the parent card without duplicate navigation", () => {
    render(
      <StageRunDetailShell
        stageKey="enumeration"
        operationId="operation-184"
        statusLabel="3 agents running"
      >
        <div>Enumeration detail</div>
      </StageRunDetailShell>
    );

    expect(screen.getByRole("heading", { name: "Enumeration" })).toBeInTheDocument();
    expect(screen.getByText("operation-184")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("3 agents running");
    expect(screen.getByText("Enumeration detail")).toBeInTheDocument();

    expect(screen.queryByRole("navigation", { name: "Pentest stages" })).not.toBeInTheDocument();
    expect(screen.queryByRole("list")).not.toBeInTheDocument();
  });

  it("normalizes production stage aliases and renders optional metric and side-rail slots", () => {
    render(
      <StageRunDetailShell
        stageKey="vuln_triage"
        statusLabel="Evidence gate open"
        metricSlots={<div>13 / 18 evidence cells</div>}
        sideRail={<div>Vulnerability Controller</div>}
      >
        <div>Worker activity</div>
      </StageRunDetailShell>
    );

    expect(screen.getByRole("heading", { name: "Vulnerability" })).toBeInTheDocument();
    expect(screen.getByText("13 / 18 evidence cells")).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Stage agents" })).toHaveTextContent(
      "Vulnerability Controller"
    );
    expect(screen.getByTestId("stage-run-detail-body")).toHaveTextContent("Worker activity");
    expect(screen.queryByText("Operation")).not.toBeInTheDocument();
  });

  it("keeps Candidate distinct from Verification without adding a vertical scroll owner", () => {
    render(
      <StageRunDetailShell stageKey="attack_candidate" statusLabel="Review required">
        <div>Candidate review</div>
      </StageRunDetailShell>
    );

    const shell = screen.getByTestId("stage-run-detail-shell");
    const body = screen.getByTestId("stage-run-detail-body");
    expect(screen.getByRole("heading", { name: "Candidate" })).toBeInTheDocument();
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
    expect(shell.className).not.toContain("overflow-y-auto");
    expect(body.className).not.toContain("overflow-y-auto");
    expect(screen.queryByTestId("stage-run-detail-metrics")).not.toBeInTheDocument();
    expect(screen.queryByTestId("stage-run-detail-side-rail")).not.toBeInTheDocument();
  });
});
