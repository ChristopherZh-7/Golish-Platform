import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ExecutionModePicker } from "./ExecutionModePicker";
import { readLastProfile } from "./executionModePicker.utils";

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

function setup(chatExecutionMode: string) {
  const onExecutionModeChange = vi.fn();
  const onAgentModeChange = vi.fn();
  render(
    <ExecutionModePicker
      chatExecutionMode={chatExecutionMode}
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

  it("remembers the active Task profile in localStorage", async () => {
    setup("red_team");
    await waitFor(() => expect(readLastProfile()).toBe("red_team"));
  });
});
