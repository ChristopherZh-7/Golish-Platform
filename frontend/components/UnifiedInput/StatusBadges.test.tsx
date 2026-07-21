import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { type BackgroundJob, useStore } from "@/store";
import { BackgroundJobsBadge } from "./StatusBadges";

function backgroundJob(overrides: Partial<BackgroundJob> = {}): BackgroundJob {
  return {
    jobId: "job-1",
    command: "naabu --host 127.0.0.1",
    toolName: "pentest_run",
    origin: { kind: "main_tool", requestId: "req-main" },
    startedAt: Date.now(),
    backgroundedAt: Date.now(),
    state: "running",
    ...overrides,
  };
}

describe("BackgroundJobsBadge", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders a fallback count when background job details are unavailable", () => {
    render(<BackgroundJobsBadge jobs={[]} fallbackCount={2} />);

    expect(screen.getByRole("button", { name: /2 running/i })).toBeInTheDocument();
  });

  it("updates visible job uptime every second while jobs are running", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-06-25T00:00:10.000Z"));

    render(
      <BackgroundJobsBadge
        jobs={[backgroundJob({ startedAt: Date.now() - 7000 })]}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /1 running/i }));

    expect(screen.getByText("7s")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(2000);
    });

    expect(screen.getByText("9s")).toBeInTheDocument();
  });

  it("uses a high-contrast live style for the background jobs trigger", () => {
    render(
      <BackgroundJobsBadge jobs={[backgroundJob()]} />
    );

    const button = screen.getByRole("button", { name: /1 running/i });

    expect(button.className).toContain("text-[var(--ansi-blue)]");
    expect(button.className).toContain("border-[var(--ansi-blue)]/35");
  });

  it("can reserve a stable header slot while no jobs are running", () => {
    const { container } = render(<BackgroundJobsBadge jobs={[]} reserveSpace />);

    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    const placeholder = container.querySelector('[aria-hidden="true"]');
    expect(placeholder).toBeInTheDocument();
    expect(placeholder?.className).toContain("invisible");
    expect(placeholder?.className).toContain("w-[7.25rem]");
  });

  it("opens the exact originating tool when a job row is selected", () => {
    const sessionId = "session-badge";
    useStore.setState((state) => {
      state.sessions[sessionId] = {
        id: sessionId,
        name: "Badge",
        workingDirectory: "/tmp",
        createdAt: "2026-07-21T00:00:00.000Z",
        mode: "agent",
      };
      state.backgroundJobs[sessionId] = [backgroundJob()];
    });

    render(
      <BackgroundJobsBadge jobs={useStore.getState().backgroundJobs[sessionId]} sessionId={sessionId} />
    );
    fireEvent.click(screen.getByRole("button", { name: /1 running/i }));
    fireEvent.click(screen.getByRole("button", { name: /open naabu/i }));

    expect(useStore.getState().sessions[sessionId]).toEqual(
      expect.objectContaining({
        detailViewMode: "tool-detail",
        toolDetailRequestIds: ["req-main"],
      })
    );
  });
});
