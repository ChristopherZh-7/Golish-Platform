import { afterEach, describe, expect, it, vi } from "vitest";
import { writeLastProfile } from "@/lib/ai/execution-mode";
import { useStore } from "@/store";
import { restoreTerminalForConv } from "./terminal-restore";

function seedConversationTerminal(executionMode = "chat") {
  useStore.setState({
    activeSessionId: null,
    conversationTerminals: { "conv-1": ["term-1"] },
    sessions: {
      "term-1": {
        id: "term-1",
        name: "Terminal",
        workingDirectory: "/tmp",
        createdAt: new Date().toISOString(),
        mode: "agent",
        tabType: "terminal",
        executionMode,
      },
    },
  });
}

describe("restoreTerminalForConv execution mode restore", () => {
  afterEach(() => {
    globalThis.localStorage?.clear();
    useStore.setState({
      activeSessionId: null,
      conversationTerminals: {},
      sessions: {},
    });
    vi.clearAllMocks();
  });

  it("preserves concrete Task profile ids during restore", async () => {
    seedConversationTerminal();

    await restoreTerminalForConv(
      "conv-1",
      { workingDirectory: "/tmp", scrollback: "", executionMode: "red_team" },
      true,
      vi.fn()
    );

    expect(useStore.getState().sessions["term-1"]?.executionMode).toBe("red_team");
  });

  it("normalizes legacy bare task restore data to the remembered profile", async () => {
    writeLastProfile("red_team");
    seedConversationTerminal();

    await restoreTerminalForConv(
      "conv-1",
      { workingDirectory: "/tmp", scrollback: "", executionMode: "task" },
      true,
      vi.fn()
    );

    expect(useStore.getState().sessions["term-1"]?.executionMode).toBe("red_team");
  });
});
