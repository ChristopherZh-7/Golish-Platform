import { afterEach, describe, expect, it, vi } from "vitest";
import {
  clearTerminalAutoFocusSuppression,
  isTerminalAutoFocusSuppressed,
} from "@/lib/terminal/terminalAutoFocus";
import { useStore } from "@/store";
import { activateConversationTerminalFromChat } from "./conversationTerminalActivation";
import { writeLastProfile } from "./executionModePicker.utils";

vi.mock("@/lib/api/pty", () => ({
  setActiveTerminalSession: vi.fn(() => Promise.resolve()),
}));

describe("activateConversationTerminalFromChat", () => {
  afterEach(() => {
    clearTerminalAutoFocusSuppression("term-chat");
    globalThis.localStorage?.clear();
    vi.clearAllMocks();
  });

  it("activates the conversation terminal without allowing terminal auto-focus", () => {
    const setMode = vi.fn();
    writeLastProfile("red_team");
    useStore.setState({
      activeSessionId: "other",
      activeConversationId: "conv-1",
      conversationTerminals: { "conv-1": ["term-chat"] },
      sessions: {
        "term-chat": {
          id: "term-chat",
          name: "Terminal",
          workingDirectory: "/tmp",
          createdAt: new Date().toISOString(),
          mode: "agent",
          tabType: "terminal",
          executionMode: "task",
        },
      },
      tabActivationHistory: [],
      tabHasNewActivity: {},
    });

    expect(
      activateConversationTerminalFromChat("conv-1", {
        setChatExecutionMode: setMode,
        emptyExecutionMode: "chat",
      })
    ).toBe("term-chat");

    expect(useStore.getState().activeSessionId).toBe("term-chat");
    expect(isTerminalAutoFocusSuppressed("term-chat")).toBe(true);
    expect(setMode).toHaveBeenCalledWith("red_team");
    expect(useStore.getState().sessions["term-chat"]?.executionMode).toBe("red_team");
  });

  it("uses the empty fallback mode when the conversation has no terminal", () => {
    const setMode = vi.fn();
    useStore.setState({
      activeConversationId: "conv-empty",
      conversationTerminals: { "conv-empty": [] },
      sessions: {},
    });

    expect(
      activateConversationTerminalFromChat("conv-empty", {
        setChatExecutionMode: setMode,
        emptyExecutionMode: () => "last-mode",
      })
    ).toBeNull();

    expect(setMode).toHaveBeenCalledWith("last-mode");
    expect(isTerminalAutoFocusSuppressed("term-chat")).toBe(false);
  });
});
