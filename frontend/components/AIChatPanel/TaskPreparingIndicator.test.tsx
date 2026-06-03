import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TaskPreparingIndicator } from "./TaskPreparingIndicator";

describe("TaskPreparingIndicator", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("title-cases the profile id and labels it as a mode in task modes", () => {
    render(<TaskPreparingIndicator modeId="red_team" />);
    expect(screen.getByText(/Red Team mode/)).toBeInTheDocument();
  });

  it("uses a neutral label without a profile name in chat mode", () => {
    render(<TaskPreparingIndicator modeId="chat" />);
    expect(screen.getByText(/Preparing/)).toBeInTheDocument();
    expect(screen.queryByText(/mode ·/)).not.toBeInTheDocument();
  });

  it("counts up elapsed seconds while mounted", () => {
    vi.useFakeTimers();
    render(<TaskPreparingIndicator modeId="assessment" />);
    expect(screen.getByText(/\(0s\)/)).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(screen.getByText(/\(3s\)/)).toBeInTheDocument();
  });

  it("anchors the counter to startedAt so it shows real elapsed on mount", () => {
    vi.useFakeTimers();
    render(<TaskPreparingIndicator modeId="assessment" startedAt={Date.now() - 12_000} />);
    expect(screen.getByText(/\(12s\)/)).toBeInTheDocument();
  });

  it("keeps counting from the same anchor across a remount (tab switch)", () => {
    vi.useFakeTimers();
    const started = Date.now();
    const { unmount } = render(
      <TaskPreparingIndicator modeId="assessment" startedAt={started} />
    );
    expect(screen.getByText(/\(0s\)/)).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    expect(screen.getByText(/\(5s\)/)).toBeInTheDocument();

    // Leave the tab (unmount) and let time pass before coming back.
    unmount();
    act(() => {
      vi.advanceTimersByTime(4000);
    });
    render(<TaskPreparingIndicator modeId="assessment" startedAt={started} />);

    // Remounted: reflects the total 9s elapsed, not a reset to 0.
    expect(screen.getByText(/\(9s\)/)).toBeInTheDocument();
  });
});
