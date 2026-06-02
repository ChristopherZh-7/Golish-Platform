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
});
