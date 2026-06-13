import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  clearTerminalAutoFocusSuppression,
  isTerminalAutoFocusSuppressed,
} from "@/lib/terminal/terminalAutoFocus";
import { useChatConversationOps } from "./useChatConversationOps";

describe("useChatConversationOps.handleNewChat", () => {
  afterEach(() => {
    vi.clearAllMocks();
    clearTerminalAutoFocusSuppression("term-new");
  });

  it("suppresses the new terminal's auto-focus so the cursor stays in the chat input", async () => {
    const createTerminalTab = vi.fn().mockResolvedValue("term-new");
    const { result } = renderHook(() => useChatConversationOps(createTerminalTab));

    await act(async () => {
      await result.current.handleNewChat();
    });

    // The "+" path links the terminal to its conversation itself, hence
    // skipConversationLink=true.
    expect(createTerminalTab).toHaveBeenCalledWith(undefined, true);
    // It must mark the fresh terminal so the terminal + UnifiedInput startup
    // focus effects skip it (chat-first) for the suppression window.
    expect(isTerminalAutoFocusSuppressed("term-new")).toBe(true);
  });

  it("marks nothing when terminal creation fails", async () => {
    const createTerminalTab = vi.fn().mockResolvedValue(null);
    const { result } = renderHook(() => useChatConversationOps(createTerminalTab));

    await act(async () => {
      await result.current.handleNewChat();
    });

    // A never-created session id was never marked.
    expect(isTerminalAutoFocusSuppressed("term-null-case")).toBe(false);
  });
});
