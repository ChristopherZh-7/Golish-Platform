import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getStageAssetCoverage } from "@/lib/api/stage-coverage";
import {
  LIVE_WORK_RETENTION_MS,
  StageAssetCoverageBlock,
  StageAssetCoveragePanel,
} from "./StageAssetCoveragePanel";

vi.mock("@/lib/api/stage-coverage", () => ({
  getStageAssetCoverage: vi.fn(),
}));

const mockedGetStageAssetCoverage = vi.mocked(getStageAssetCoverage);

function coverageCell(technique: string, label: string) {
  return {
    technique,
    label,
    state: "pending",
    source: null,
    evidence_refs: [],
    note: null,
    suggested_tools: [],
  };
}

function snapshot() {
  return {
    stage: "external_attack_surface",
    organization_id: "org-1",
    session_id: "session-1",
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
        value: "10.18.2.4",
        target_type: "ip",
        real_ip: "",
        source: "seed",
        discovered_phase: "seed",
        created_at: "2026-06-27T10:00:00.000Z",
        parent_id: null,
        coverage: [
          coverageCell("GOLISH-EAS-LIVENESS", "LIVENESS"),
          coverageCell("GOLISH-EAS-PORT", "PORT"),
          coverageCell("GOLISH-EAS-SERVICE-FINGERPRINT", "SERVICE"),
        ],
      },
      {
        target_id: "target-2",
        value: "app.example.com",
        target_type: "domain",
        real_ip: "10.18.2.4",
        source: "asset_intel",
        discovered_phase: "seed",
        created_at: "2026-06-27T10:00:02.000Z",
        parent_id: null,
        coverage: [
          coverageCell("GOLISH-EAS-LIVENESS", "LIVENESS"),
          coverageCell("GOLISH-EAS-PORT", "PORT"),
          coverageCell("GOLISH-EAS-SERVICE-FINGERPRINT", "SERVICE"),
        ],
      },
      {
        target_id: "target-3",
        value: "124.196.83.50",
        target_type: "ip",
        real_ip: "",
        source: "seed",
        discovered_phase: "seed",
        created_at: "2026-06-27T10:00:03.000Z",
        parent_id: null,
        coverage: [
          coverageCell("GOLISH-EAS-LIVENESS", "LIVENESS"),
          coverageCell("GOLISH-EAS-PORT", "PORT"),
          coverageCell("GOLISH-EAS-SERVICE-FINGERPRINT", "SERVICE"),
        ],
      },
    ],
  };
}

describe("StageAssetCoveragePanel", () => {
  beforeEach(() => {
    mockedGetStageAssetCoverage.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("keeps the coverage block collapsed while live work is running", () => {
    render(
      <StageAssetCoverageBlock
        organizationId="org-1"
        stage="external_attack_surface"
        sessionId="session-1"
        pollWhileActive
        workItems={[
          {
            id: "tool-1",
            displayToolName: "nmap",
            rawToolName: "pentest_run",
            subject: "10.18.2.4",
            subjects: ["10.18.2.4"],
            primary: "nmap -sV 10.18.2.4 -p 443",
            techniques: ["PORT", "SERVICE"],
            status: "running",
            startedAt: "2026-06-27T10:00:01.000Z",
          },
        ]}
      />
    );

    expect(screen.getByRole("button", { name: /资产覆盖/ })).toHaveAttribute(
      "aria-expanded",
      "false"
    );
    expect(screen.getByText("1")).toBeInTheDocument();
    expect(screen.queryByTestId("stage-asset-coverage-scroll")).not.toBeInTheDocument();
  });

  it("renders coverage summary mode as a compact strip without the matrix", async () => {
    mockedGetStageAssetCoverage.mockResolvedValue(snapshot());
    const onOpenCoverage = vi.fn();

    render(
      <StageAssetCoverageBlock
        displayMode="summary"
        organizationId="org-1"
        stage="external_attack_surface"
        sessionId="session-1"
        subtitle="External Attack Surface · Acme"
        workItems={[
          {
            id: "tool-1",
            displayToolName: "nmap",
            rawToolName: "pentest_run",
            subject: "10.18.2.4",
            subjects: ["10.18.2.4"],
            primary: "nmap -sV 10.18.2.4 -p 443",
            techniques: ["PORT", "SERVICE"],
            status: "running",
            startedAt: "2026-06-27T10:00:01.000Z",
          },
        ]}
        onOpenCoverage={onOpenCoverage}
      />
    );

    expect(screen.queryByTestId("stage-asset-coverage-scroll")).not.toBeInTheDocument();
    expect(await screen.findByText("0/3 done")).toBeInTheDocument();
    expect(screen.getByText("nmap · PORT/SERVICE · 10.18.2.4")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /资产覆盖/ }));

    expect(onOpenCoverage).toHaveBeenCalledTimes(1);
  });

  it("renders coverage panel mode as the full matrix view", async () => {
    mockedGetStageAssetCoverage.mockResolvedValue(snapshot());

    render(
      <StageAssetCoverageBlock
        displayMode="panel"
        organizationId="org-1"
        stage="external_attack_surface"
        sessionId="session-1"
        subtitle="External Attack Surface · Acme"
      />
    );

    await screen.findByTestId("stage-asset-coverage-scroll");

    expect(screen.getByText("app.example.com")).toBeInTheDocument();
    expect(screen.queryByRole("separator", { name: "调整资产覆盖高度" })).not.toBeInTheDocument();
  });

  it("shows a compact return action in coverage panel mode", async () => {
    mockedGetStageAssetCoverage.mockResolvedValue(snapshot());
    const onBackToRun = vi.fn();

    render(
      <StageAssetCoverageBlock
        displayMode="panel"
        organizationId="org-1"
        stage="external_attack_surface"
        sessionId="session-1"
        onBackToRun={onBackToRun}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "运行流" }));

    expect(onBackToRun).toHaveBeenCalledTimes(1);
  });

  it("keeps the running filter available before live work appears", () => {
    render(
      <StageAssetCoveragePanel snapshot={snapshot()} loading={false} error={null} workItems={[]} />
    );

    expect(screen.getByRole("button", { name: "只看运行中" })).toBeInTheDocument();
    expect(screen.getByText("124.196.83.50")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "只看运行中" }));

    expect(screen.getByText("当前没有运行中的资产任务")).toBeInTheDocument();
    expect(screen.queryByText("124.196.83.50")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "看全部" }));

    expect(screen.getByText("124.196.83.50")).toBeInTheDocument();
  });

  it("defaults to the running asset slice when live work exists", () => {
    render(
      <StageAssetCoveragePanel
        snapshot={snapshot()}
        loading={false}
        error={null}
        workItems={[
          {
            id: "tool-1",
            displayToolName: "nmap",
            rawToolName: "pentest_run",
            subject: "10.18.2.4",
            subjects: ["10.18.2.4"],
            primary: "nmap -sV 10.18.2.4 -p 443",
            techniques: ["PORT", "SERVICE"],
            status: "running",
            startedAt: "2026-06-27T10:00:01.000Z",
          },
        ]}
      />
    );

    expect(screen.getAllByText("10.18.2.4").length).toBeGreaterThan(0);
    expect(screen.getByText("运行中")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "看全部" })).toBeInTheDocument();
    expect(screen.getByText("正在做的资产")).toBeInTheDocument();
    expect(screen.queryByText("124.196.83.50")).not.toBeInTheDocument();
    expect(screen.getByText("正在补 PORT/SERVICE · nmap")).toBeInTheDocument();
    expect(screen.queryByText("运行中但尚未匹配到资产行")).not.toBeInTheDocument();
    expect(screen.getByTestId("stage-asset-coverage-scroll")).toHaveStyle({
      height: "224px",
      maxHeight: "224px",
    });

    fireEvent.click(screen.getByRole("button", { name: "看全部" }));

    expect(screen.getByText("124.196.83.50")).toBeInTheDocument();
    expect(screen.getByTestId("stage-asset-coverage-scroll")).toHaveStyle({
      height: "224px",
      maxHeight: "224px",
    });
  });

  it("switches to the running slice when live work appears unless the user chose a view", () => {
    const runningWork = [
      {
        id: "tool-1",
        displayToolName: "nmap",
        rawToolName: "pentest_run",
        subject: "10.18.2.4",
        subjects: ["10.18.2.4"],
        primary: "nmap -sV 10.18.2.4 -p 443",
        techniques: ["PORT", "SERVICE"],
        status: "running" as const,
        startedAt: "2026-06-27T10:00:01.000Z",
      },
    ];
    const { rerender } = render(
      <StageAssetCoveragePanel snapshot={snapshot()} loading={false} error={null} workItems={[]} />
    );

    expect(screen.getByText("124.196.83.50")).toBeInTheDocument();

    rerender(
      <StageAssetCoveragePanel
        snapshot={snapshot()}
        loading={false}
        error={null}
        workItems={runningWork}
      />
    );

    expect(screen.getByText("正在做的资产")).toBeInTheDocument();
    expect(screen.queryByText("124.196.83.50")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "看全部" }));
    expect(screen.getByText("124.196.83.50")).toBeInTheDocument();
  });

  it("keeps the previous running slice briefly when live work clears", () => {
    vi.useFakeTimers();
    const runningWork = [
      {
        id: "tool-1",
        displayToolName: "nmap",
        rawToolName: "pentest_run",
        subject: "10.18.2.4",
        subjects: ["10.18.2.4"],
        primary: "nmap -sV 10.18.2.4 -p 443",
        techniques: ["PORT", "SERVICE"],
        status: "running" as const,
        startedAt: "2026-06-27T10:00:01.000Z",
      },
    ];
    const { rerender } = render(
      <StageAssetCoveragePanel
        snapshot={snapshot()}
        loading={false}
        error={null}
        workItems={runningWork}
      />
    );

    rerender(
      <StageAssetCoveragePanel snapshot={snapshot()} loading={false} error={null} workItems={[]} />
    );

    expect(screen.getByText("正在补 PORT/SERVICE · nmap")).toBeInTheDocument();
    expect(screen.queryByText("当前没有运行中的资产任务")).not.toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(LIVE_WORK_RETENTION_MS + 100);
    });

    expect(screen.getByText("当前没有运行中的资产任务")).toBeInTheDocument();
    expect(screen.queryByText("正在补 PORT/SERVICE · nmap")).not.toBeInTheDocument();
  });

  it("lets the asset coverage body be resized", () => {
    render(
      <StageAssetCoveragePanel snapshot={snapshot()} loading={false} error={null} workItems={[]} />
    );

    const scrollBody = screen.getByTestId("stage-asset-coverage-scroll");
    const resizeHandle = screen.getByRole("separator", { name: "调整资产覆盖高度" });

    expect(scrollBody).toHaveStyle({ height: "224px", maxHeight: "224px" });

    fireEvent.pointerDown(resizeHandle, { clientY: 100 });
    fireEvent.pointerMove(window, { clientY: 180 });
    fireEvent.pointerUp(window);

    expect(scrollBody).toHaveStyle({ height: "304px", maxHeight: "304px" });

    fireEvent.keyDown(resizeHandle, { key: "Home" });

    expect(scrollBody).toHaveStyle({ height: "160px", maxHeight: "160px" });
  });

  it("shows domain http probing as related activity under the resolved IP group", () => {
    render(
      <StageAssetCoveragePanel
        snapshot={snapshot()}
        loading={false}
        error={null}
        workItems={[
          {
            id: "tool-1",
            displayToolName: "httpx",
            rawToolName: "pentest_run",
            subject: "https://app.example.com",
            subjects: ["https://app.example.com"],
            primary: "httpx https://app.example.com",
            techniques: ["LIVENESS"],
            status: "running",
            startedAt: "2026-06-27T10:00:01.000Z",
          },
        ]}
      />
    );

    expect(screen.getByText("10.18.2.4")).toBeInTheDocument();
    expect(screen.getByText("app.example.com")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "看全部" })).toBeInTheDocument();
    expect(
      screen.getByText(/关联 https:\/\/app.example.com · LIVENESS · httpx/)
    ).toBeInTheDocument();
    expect(screen.getByText("正在补 LIVENESS · httpx")).toBeInTheDocument();
    expect(screen.queryByText("运行中但尚未匹配到资产行")).not.toBeInTheDocument();
  });

  it("keeps unmatched live work inside the coverage panel", () => {
    render(
      <StageAssetCoveragePanel
        snapshot={snapshot()}
        loading={false}
        error={null}
        workItems={[
          {
            id: "tool-1",
            displayToolName: "httpx",
            rawToolName: "pentest_run",
            subject: "https://new.example.com",
            subjects: ["https://new.example.com"],
            primary: "httpx https://new.example.com",
            techniques: ["LIVENESS"],
            status: "running",
            startedAt: "2026-06-27T10:00:01.000Z",
          },
        ]}
      />
    );

    expect(screen.getByText("运行中但尚未匹配到资产行")).toBeInTheDocument();
    expect(screen.getAllByText(/https:\/\/new.example.com/).length).toBeGreaterThan(0);
  });

  it("keeps the running filter selected when live work disappears", () => {
    vi.useFakeTimers();
    const runningWork = [
      {
        id: "tool-1",
        displayToolName: "nmap",
        rawToolName: "pentest_run",
        subject: "10.18.2.4",
        subjects: ["10.18.2.4"],
        primary: "nmap -sV 10.18.2.4 -p 443",
        techniques: ["PORT", "SERVICE"],
        status: "running" as const,
        startedAt: "2026-06-27T10:00:01.000Z",
      },
    ];
    const { rerender } = render(
      <StageAssetCoveragePanel
        snapshot={snapshot()}
        loading={false}
        error={null}
        workItems={runningWork}
      />
    );

    expect(screen.queryByText("124.196.83.50")).not.toBeInTheDocument();

    rerender(
      <StageAssetCoveragePanel snapshot={snapshot()} loading={false} error={null} workItems={[]} />
    );

    act(() => {
      vi.advanceTimersByTime(LIVE_WORK_RETENTION_MS + 100);
    });

    expect(screen.getByText("当前没有运行中的资产任务")).toBeInTheDocument();
    expect(screen.queryByText("124.196.83.50")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "看全部" }));

    expect(screen.getByText("124.196.83.50")).toBeInTheDocument();
  });

  it("keeps the active asset rows stable when live work switches briefly", () => {
    vi.useFakeTimers();
    const firstWork = {
      id: "tool-1",
      displayToolName: "httpx",
      rawToolName: "pentest_run",
      subject: "10.18.2.4",
      subjects: ["10.18.2.4"],
      primary: "httpx 10.18.2.4",
      techniques: ["LIVENESS"],
      status: "running" as const,
      startedAt: "2026-06-27T10:00:01.000Z",
    };
    const secondWork = {
      id: "tool-2",
      displayToolName: "httpx",
      rawToolName: "pentest_run",
      subject: "124.196.83.50",
      subjects: ["124.196.83.50"],
      primary: "httpx 124.196.83.50",
      techniques: ["LIVENESS"],
      status: "running" as const,
      startedAt: "2026-06-27T10:00:02.000Z",
    };
    const { rerender } = render(
      <StageAssetCoveragePanel
        snapshot={snapshot()}
        loading={false}
        error={null}
        workItems={[firstWork]}
      />
    );

    rerender(
      <StageAssetCoveragePanel
        snapshot={snapshot()}
        loading={false}
        error={null}
        workItems={[secondWork]}
      />
    );

    expect(screen.getByText("10.18.2.4")).toBeInTheDocument();
    expect(screen.getByText("124.196.83.50")).toBeInTheDocument();
    expect(screen.getByText("2 组 / 2 资产")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(LIVE_WORK_RETENTION_MS + 100);
    });

    expect(screen.queryByText("10.18.2.4")).not.toBeInTheDocument();
    expect(screen.getByText("124.196.83.50")).toBeInTheDocument();
  });
});
