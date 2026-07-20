import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ExecutionModePicker } from "./ExecutionModePicker";
import { readLastProfile, writeLastProfile } from "./executionModePicker.utils";

// The picker fetches modes on mount; stub it so it deterministically falls back
// to the embedded list (which already carries the profiles) without hitting IPC.
vi.mock("@/lib/ai", () => ({
  listExecutionModes: vi.fn().mockResolvedValue([]),
}));

afterEach(() => {
  try {
    globalThis.localStorage?.clear();
  } catch {
    // ignore
  }
  vi.clearAllMocks();
});

function setup(chatExecutionMode: string, disabled = false) {
  const onExecutionModeChange = vi.fn();
  const onAgentModeChange = vi.fn();
  render(
    <ExecutionModePicker
      chatExecutionMode={chatExecutionMode}
      disabled={disabled}
      onExecutionModeChange={onExecutionModeChange}
      onAgentModeChange={onAgentModeChange}
    />
  );
  return { onExecutionModeChange, onAgentModeChange };
}

describe("ExecutionModePicker", () => {
  it("renders a single trigger that reads 'Chat' in chat mode", () => {
    setup("chat");
    const triggers = screen.getAllByRole("button", { name: "Execution mode" });
    expect(triggers).toHaveLength(1);
    expect(triggers[0]).toHaveTextContent("Chat");
  });

  it("labels the trigger with the active profile in a Task profile mode", () => {
    setup("pentest");
    expect(screen.getByRole("button", { name: "Execution mode" })).toHaveTextContent("Pentest");
  });

  it("disables profile changes while a destructive stage reset owns the send lane", () => {
    setup("pentest", true);
    expect(screen.getByRole("button", { name: "Execution mode" })).toBeDisabled();
  });

  it("remembers the active Task profile in localStorage", async () => {
    setup("red_team");
    await waitFor(() => expect(readLastProfile()).toBe("red_team"));
  });

  it("repairs legacy bare Task mode to the remembered profile", async () => {
    writeLastProfile("red_team");
    const { onExecutionModeChange, onAgentModeChange } = setup("task");

    expect(screen.getByRole("button", { name: "Execution mode" })).toHaveTextContent("Red Team");
    await waitFor(() => expect(onExecutionModeChange).toHaveBeenCalledWith("red_team"));
    expect(onAgentModeChange).toHaveBeenCalledWith("auto-approve");
    expect(readLastProfile()).toBe("red_team");
  });

  it("repairs legacy bare Task mode to the default profile when no profile is remembered", async () => {
    const { onExecutionModeChange } = setup("task");

    expect(screen.getByRole("button", { name: "Execution mode" })).toHaveTextContent(
      "Security Assessment"
    );
    await waitFor(() => expect(onExecutionModeChange).toHaveBeenCalledWith("assessment"));
  });
});
