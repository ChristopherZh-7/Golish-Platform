import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getStageAssetCoverage } from "@/lib/api/stage-coverage";
import {
  ASSET_COVERAGE_READING_FREEZE_MS,
  LIVE_WORK_RETENTION_MS,
  StageAssetCoverageBlock,
  StageAssetCoveragePanel,
} from "./StageAssetCoveragePanel";

vi.mock("@/lib/api/stage-coverage", () => ({
  getStageAssetCoverage: vi.fn(),
}));

const mockedGetStageAssetCoverage = vi.mocked(getStageAssetCoverage);

function testRect({
  height = 0,
  top = 0,
  width = 0,
}: {
  height?: number;
  top?: number;
  width?: number;
}): DOMRect {
  return {
    bottom: top + height,
    height,
    left: 0,
    right: width,
    top,
    width,
    x: 0,
    y: top,
    toJSON: () => ({}),
  } as DOMRect;
}

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

function largeSnapshot(assetCount = 80) {
  return {
    ...snapshot(),
    summary: {
      total_assets: assetCount,
      seed_assets: assetCount,
      new_assets: 0,
      done_assets: 0,
      pending_assets: assetCount,
      blocked_assets: 0,
    },
    assets: Array.from({ length: assetCount }, (_, index) => ({
      target_id: `target-${index}`,
      value: `10.18.3.${index}`,
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
    })),
  };
}

function nextWaveSnapshot() {
  return {
    stage: "external_attack_surface",
    organization_id: "org-1",
    session_id: "session-1",
    summary: {
      total_assets: 1,
      seed_assets: 1,
      new_assets: 1,
      done_assets: 1,
      pending_assets: 0,
      blocked_assets: 0,
    },
    assets: [
      {
        target_id: "target-1",
        value: "seed.example.com",
        target_type: "domain",
        real_ip: "",
        source: "seed",
        discovered_phase: "seed",
        created_at: "2026-06-27T10:00:00.000Z",
        parent_id: null,
        coverage: [
          {
            ...coverageCell("GOLISH-EAS-LIVENESS", "LIVENESS"),
            state: "found",
            evidence_refs: [1],
          },
        ],
      },
      {
        target_id: "target-2",
        value: "new.example.com",
        target_type: "domain",
        real_ip: "",
        source: "active_discovered",
        discovered_phase: "new_in_stage",
        created_at: "2026-06-27T10:05:00.000Z",
        parent_id: null,
        coverage: [
          {
            ...coverageCell("GOLISH-EAS-LIVENESS", "LIVENESS"),
            state: "next_wave_pending",
            note: "newly discovered during this stage; queued for the next wave",
          },
        ],
      },
    ],
  };
}

function syntheticHostSnapshot() {
  const base = snapshot();
  return {
    ...base,
    summary: {
      ...base.summary,
      total_assets: 1,
      pending_assets: 1,
    },
    assets: [
      {
        ...base.assets[1],
        target_id: "resolved-domain-only",
        value: "resolved.example.com",
        real_ip: "203.0.113.10",
      },
    ],
  };
}

function targetIntelOrgOnlySnapshot() {
  return {
    stage: "target_intel",
    organization_id: "org-1",
    session_id: "session-1",
    summary: {
      total_assets: 0,
      seed_assets: 0,
      new_assets: 0,
      done_assets: 0,
      pending_assets: 0,
      blocked_assets: 0,
    },
    assets: [
      {
        target_id: "org-1",
        value: "中国平安保险公司证券交易珠海营业部",
        target_type: "organization",
        real_ip: "",
        source: "engagement_org",
        discovered_phase: "historical",
        created_at: "2026-06-27T10:00:00.000Z",
        parent_id: null,
        coverage: [
          coverageCell("GOLISH-INTEL-DNS", "DNS"),
          coverageCell("GOLISH-INTEL-WHOIS", "WHOIS"),
          coverageCell("GOLISH-INTEL-ASN", "ASN"),
          coverageCell("GOLISH-INTEL-CT", "CT"),
          coverageCell("GOLISH-INTEL-SUBDOMAIN", "Subdomain"),
          coverageCell("GOLISH-INTEL-OSINT", "OSINT"),
        ],
      },
    ],
  };
}

function enumerationFourAxisSnapshot() {
  return {
    stage: "enumeration",
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
        target_id: "target-enum-1",
        value: "203.0.113.10",
        target_type: "ip",
        real_ip: "",
        source: "seed",
        discovered_phase: "seed",
        created_at: "2026-07-01T10:00:00.000Z",
        parent_id: null,
        coverage: [
          coverageCell("GOLISH-ENUM-JS", "JS"),
          coverageCell("GOLISH-ENUM-DIR", "Directory"),
          coverageCell("GOLISH-ENUM-PARAM", "Parameter"),
          coverageCell("GOLISH-ENUM-JSAPI", "API"),
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
        stageStartedAt="2026-06-27T10:00:00.000Z"
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
    expect(await screen.findByText("0/1 done")).toBeInTheDocument();
    expect(screen.getByText("nmap · PORT/SERVICE · 10.18.2.4")).toBeInTheDocument();
    expect(mockedGetStageAssetCoverage).toHaveBeenCalledWith({
      organizationId: "org-1",
      sessionId: "session-1",
      stage: "external_attack_surface",
      stageStartedAt: "2026-06-27T10:00:00.000Z",
    });

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
    expect(screen.getAllByText(/未查 LIVE\/PORT\/SVC/).length).toBeGreaterThan(0);
    expect(screen.queryByRole("separator", { name: "调整资产覆盖高度" })).not.toBeInTheDocument();
  });

  it("shows next-wave assets without counting them in the current denominator", () => {
    render(
      <StageAssetCoveragePanel
        snapshot={nextWaveSnapshot()}
        loading={false}
        error={null}
        workItems={[]}
      />
    );

    expect(screen.getByText("1/1 done")).toBeInTheDocument();
    expect(screen.getByText("1 下批")).toBeInTheDocument();
    expect(screen.getByText("new.example.com")).toBeInTheDocument();
    expect(screen.getByText(/下批待查 LIVE/)).toBeInTheDocument();
    expect(screen.getAllByText(/下批/).length).toBeGreaterThan(0);
  });

  it("marks synthetic resolved IP groups as grouping rows instead of unchecked coverage", () => {
    render(
      <StageAssetCoveragePanel
        snapshot={syntheticHostSnapshot()}
        loading={false}
        error={null}
        workItems={[]}
      />
    );

    expect(screen.getByText("203.0.113.10")).toBeInTheDocument();
    expect(screen.getByText(/仅分组，不计覆盖/)).toBeInTheDocument();
    expect(screen.getByText("resolved.example.com")).toBeInTheDocument();
    expect(screen.getByText(/未查 LIVE\/PORT\/SVC/)).toBeInTheDocument();
  });

  it("shows readable target intel organization coverage dimensions", () => {
    render(
      <StageAssetCoveragePanel
        snapshot={targetIntelOrgOnlySnapshot()}
        loading={false}
        error={null}
        workItems={[]}
      />
    );

    expect(screen.getByText("组织情报")).toBeInTheDocument();
    expect(screen.getByText("中国平安保险公司证券交易珠海营业部")).toBeInTheDocument();
    for (const label of ["DNS", "WHOIS", "ASN", "CT证书", "子域", "OSINT"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  it("renders enumeration JS and API as separate coverage columns", () => {
    render(
      <StageAssetCoveragePanel
        snapshot={enumerationFourAxisSnapshot()}
        loading={false}
        error={null}
        workItems={[
          {
            id: "tool-js",
            displayToolName: "browser_collect_js_api",
            rawToolName: "browser_collect_js_api",
            subject: "203.0.113.10",
            subjects: ["203.0.113.10"],
            primary: "Collecting JavaScript",
            techniques: ["JS"],
            status: "running",
            startedAt: "2026-07-01T10:00:01.000Z",
          },
        ]}
      />
    );

    for (const label of ["JS", "DIR", "PARAM", "API"]) {
      expect(screen.getAllByText(label).length).toBeGreaterThan(0);
    }
    expect(screen.getByText("正在补 JS · browser_collect_js_api")).toBeInTheDocument();
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

  it("renders the current full coverage scale directly to avoid fast-scroll blanking", () => {
    render(
      <StageAssetCoveragePanel
        snapshot={largeSnapshot(332)}
        loading={false}
        error={null}
        workItems={[]}
      />
    );

    expect(screen.queryByTestId("stage-asset-coverage-virtual-groups")).not.toBeInTheDocument();
    expect(screen.getByText("10.18.3.0")).toBeInTheDocument();
    expect(screen.getByText("10.18.3.331")).toBeInTheDocument();
  });

  it("virtualizes very large coverage matrices so fast scrolling only paints visible groups", async () => {
    render(
      <StageAssetCoveragePanel
        snapshot={largeSnapshot(600)}
        loading={false}
        error={null}
        workItems={[]}
      />
    );

    expect(screen.getByTestId("stage-asset-coverage-virtual-groups")).toBeInTheDocument();
    expect(await screen.findByText("10.18.3.0")).toBeInTheDocument();
    expect(screen.queryByText("10.18.3.599")).not.toBeInTheDocument();
  });

  it("renders medium running slices directly to keep wheel scrolling smooth", () => {
    render(
      <StageAssetCoveragePanel
        snapshot={largeSnapshot(89)}
        loading={false}
        error={null}
        workItems={[]}
      />
    );

    expect(screen.queryByTestId("stage-asset-coverage-virtual-groups")).not.toBeInTheDocument();
    expect(screen.getByText("10.18.3.0")).toBeInTheDocument();
    expect(screen.getByText("10.18.3.88")).toBeInTheDocument();
  });

  it("does not replace the visible matrix while the user is reading after scroll", () => {
    vi.useFakeTimers();
    const first = largeSnapshot(3);
    const second = {
      ...largeSnapshot(3),
      assets: largeSnapshot(3).assets.map((asset, index) => ({
        ...asset,
        target_id: `refreshed-${index}`,
        value: `198.51.100.${index}`,
      })),
    };
    const { rerender } = render(
      <StageAssetCoveragePanel snapshot={first} loading={false} error={null} workItems={[]} />
    );
    const scrollBody = screen.getByTestId("stage-asset-coverage-scroll");

    expect(screen.getByText("10.18.3.0")).toBeInTheDocument();
    fireEvent.wheel(scrollBody);
    rerender(
      <StageAssetCoveragePanel snapshot={second} loading={false} error={null} workItems={[]} />
    );

    expect(screen.getByText("10.18.3.0")).toBeInTheDocument();
    expect(screen.queryByText("198.51.100.0")).not.toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(ASSET_COVERAGE_READING_FREEZE_MS + 100);
    });

    expect(screen.getByText("198.51.100.0")).toBeInTheDocument();
    expect(screen.queryByText("10.18.3.0")).not.toBeInTheDocument();
  });

  it("updates the virtual coverage window immediately on fast scroll", async () => {
    let scrollTop = 0;
    const rectSpy = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: HTMLElement) {
        const testId = this.getAttribute("data-testid");
        if (testId === "stage-asset-coverage-scroll") {
          return testRect({ height: 224, width: 800 });
        }
        if (testId === "stage-asset-coverage-groups") {
          return testRect({ top: -scrollTop, width: 800 });
        }
        return testRect({ width: 800 });
      });
    try {
      render(
        <StageAssetCoveragePanel
          snapshot={largeSnapshot(600)}
          loading={false}
          error={null}
          workItems={[]}
        />
      );

      const scrollBody = screen.getByTestId("stage-asset-coverage-scroll");
      Object.defineProperty(scrollBody, "scrollTop", {
        configurable: true,
        get: () => scrollTop,
        set: (value) => {
          scrollTop = Number(value);
        },
      });
      Object.defineProperty(scrollBody, "clientHeight", {
        configurable: true,
        value: 224,
      });
      Object.defineProperty(scrollBody, "scrollHeight", {
        configurable: true,
        value: 33600,
      });

      await screen.findByText("10.18.3.0");
      act(() => {
        scrollBody.scrollTop = 32000;
        scrollBody.dispatchEvent(new Event("scroll"));
      });

      expect(await screen.findByText("10.18.3.599")).toBeInTheDocument();
      expect(screen.queryByText("10.18.3.0")).not.toBeInTheDocument();
    } finally {
      rectSpy.mockRestore();
    }
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
