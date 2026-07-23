import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BackgroundJob, BackgroundRunMeta } from "@/store";
import { useStore } from "@/store";
import { BackgroundJobPanel } from "./BackgroundJobPanel";

const { cancelBackgroundJob } = vi.hoisted(() => ({ cancelBackgroundJob: vi.fn() }));

vi.mock("@/lib/ai", async (importOriginal) => {
  const original = await importOriginal<typeof import("@/lib/ai")>();
  return { ...original, cancelBackgroundJob };
});

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, string>) => {
      const labels: Record<string, string> = {
        "ai.backgroundJobs.panelLabel": "Managed process lifecycle",
        "ai.backgroundJobs.stop": "Stop managed process",
        "ai.backgroundJobs.stopAction": "Stop",
        "ai.backgroundJobs.backgroundFor": "Managed for {{duration}}",
        "ai.backgroundJobs.totalDuration": "Total duration {{duration}}",
        "ai.backgroundJobs.manualTermination": "No automatic process deadline · stop explicitly if needed",
        "ai.backgroundJobs.lastOutput": "Last output {{duration}} ago",
        "ai.backgroundJobs.status.running": "Managed process running",
        "ai.backgroundJobs.status.stopping": "Stopping…",
        "ai.backgroundJobs.status.completed": "Completed",
        "ai.backgroundJobs.status.failed": "Failed",
        "ai.backgroundJobs.status.stopped": "Stopped",
      };
      return (labels[key] ?? key).replace(
        /{{(\w+)}}/g,
        (_match, name: string) => values?.[name] ?? ""
      );
    },
  }),
}));

const SESSION_ID = "session-bg-panel";

function liveJob(overrides: Partial<BackgroundJob> = {}): BackgroundJob {
  return {
    jobId: "job-1",
    command: "naabu -host 10.0.0.1",
    toolName: "pentest_run",
    origin: { kind: "main_tool", requestId: "req-1" },
    startedAt: Date.now() - 40_000,
    backgroundedAt: Date.now() - 10_000,
    lastOutputAt: Date.now() - 2_000,
    initialYieldMs: 10_000,
    automaticKill: false,
    state: "running",
    ...overrides,
  };
}

describe("BackgroundJobPanel", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-21T00:02:00.000Z"));
    cancelBackgroundJob.mockReset();
    useStore.setState({ backgroundJobs: { [SESSION_ID]: [liveJob()] } });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows the live background lifecycle without an automatic deadline", () => {
    const job = useStore.getState().backgroundJobs[SESSION_ID][0];
    render(<BackgroundJobPanel sessionId={SESSION_ID} backgroundRun={job} job={job} />);

    expect(screen.getByText("Managed process running")).toBeInTheDocument();
    expect(screen.queryByText(/Moved to background/i)).not.toBeInTheDocument();
    expect(screen.getByText(/No automatic process deadline/i)).toBeInTheDocument();
    expect(screen.getByText(/Last output 2s ago/i)).toBeInTheDocument();
    expect(screen.getByText("job-1")).toBeInTheDocument();
  });

  it("moves to Stopping after a cancellation request and waits for completion", async () => {
    cancelBackgroundJob.mockResolvedValue(true);
    const job = useStore.getState().backgroundJobs[SESSION_ID][0];
    render(<BackgroundJobPanel sessionId={SESSION_ID} backgroundRun={job} job={job} />);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /stop managed process/i }));
    });

    expect(cancelBackgroundJob).toHaveBeenCalledWith("job-1");
    expect(useStore.getState().backgroundJobs[SESSION_ID][0].state).toBe("stopping");
    expect(screen.getAllByText("Stopping…")).toHaveLength(2);
    expect(screen.queryByText("Stopped")).not.toBeInTheDocument();
  });

  it("keeps a terminal history panel after the live registry entry is removed", () => {
    useStore.setState({ backgroundJobs: { [SESSION_ID]: [] } });
    const meta: BackgroundRunMeta = {
      jobId: "job-1",
      backgroundedAt: Date.now() - 10_000,
      initialYieldMs: 10_000,
      automaticKill: false,
    };
    render(
      <BackgroundJobPanel
        sessionId={SESSION_ID}
        backgroundRun={meta}
        job={null}
        terminalResult={{ status: "killed", duration_ms: 45_000 }}
      />
    );

    expect(screen.getByText("Stopped")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /stop managed process/i })).not.toBeInTheDocument();
  });

  it("updates elapsed time while live", () => {
    const job = useStore.getState().backgroundJobs[SESSION_ID][0];
    render(<BackgroundJobPanel sessionId={SESSION_ID} backgroundRun={job} job={job} />);
    expect(screen.getByText(/Managed for 10s/i)).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(2_000));

    expect(screen.getByText(/Managed for 12s/i)).toBeInTheDocument();
  });
});
