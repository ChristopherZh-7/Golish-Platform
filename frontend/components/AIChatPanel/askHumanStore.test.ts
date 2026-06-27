import { describe, expect, it, vi } from "vitest";
import { type AskHumanStoreClearState, clearMatchingPendingAskHuman } from "./askHumanStore";

describe("clearMatchingPendingAskHuman", () => {
  it("clears the same ask_human request from AI and terminal session keys", () => {
    const state: AskHumanStoreClearState = {
      pendingAskHuman: {
        "ai-session": { requestId: "req-1" },
        "terminal-session": { requestId: "req-1" },
        "other-session": { requestId: "req-2" },
      },
      clearPendingAskHuman: vi.fn((sessionId: string) => {
        state.pendingAskHuman[sessionId] = null;
      }),
    };

    clearMatchingPendingAskHuman(state, { requestId: "req-1", sessionId: "ai-session" }, [
      "terminal-session",
      "other-session",
      "ai-session",
      null,
    ]);

    expect(state.clearPendingAskHuman).toHaveBeenCalledTimes(2);
    expect(state.clearPendingAskHuman).toHaveBeenCalledWith("ai-session");
    expect(state.clearPendingAskHuman).toHaveBeenCalledWith("terminal-session");
    expect(state.pendingAskHuman["other-session"]).toEqual({ requestId: "req-2" });
  });

  it("does not clear a newer prompt that reused the same session key", () => {
    const state: AskHumanStoreClearState = {
      pendingAskHuman: {
        "ai-session": { requestId: "req-new" },
      },
      clearPendingAskHuman: vi.fn(),
    };

    clearMatchingPendingAskHuman(state, { requestId: "req-old", sessionId: "ai-session" }, [
      "ai-session",
    ]);

    expect(state.clearPendingAskHuman).not.toHaveBeenCalled();
  });
});
