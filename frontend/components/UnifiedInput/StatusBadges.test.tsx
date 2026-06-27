import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BackgroundJobsBadge } from "./StatusBadges";

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
        jobs={[{ jobId: "job-1", command: "naabu --host 127.0.0.1", startedAt: Date.now() - 7000 }]}
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
      <BackgroundJobsBadge
        jobs={[{ jobId: "job-1", command: "naabu --host 127.0.0.1", startedAt: Date.now() }]}
      />
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
});
