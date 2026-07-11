import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  clearTimeline: vi.fn(),
  clearCurrent: vi.fn(),
  clearLegacy: vi.fn(),
  warn: vi.fn(),
}));

vi.mock("./index", () => ({
  useStore: {
    getState: () => ({ clearTimeline: mocks.clearTimeline }),
  },
}));

vi.mock("@/lib/ai", () => ({
  clearAiConversationSession: mocks.clearCurrent,
  clearAiConversation: mocks.clearLegacy,
}));

vi.mock("@/lib/logger", () => ({
  logger: { warn: mocks.warn },
}));

import { clearConversation } from "./actions";

describe("clearConversation backend/frontend atomicity", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.clearCurrent.mockResolvedValue(undefined);
    mocks.clearLegacy.mockResolvedValue(undefined);
  });

  it("clears the local timeline only after the current backend command succeeds", async () => {
    await clearConversation("session-1");

    expect(mocks.clearCurrent).toHaveBeenCalledWith("session-1");
    expect(mocks.clearLegacy).not.toHaveBeenCalled();
    expect(mocks.clearTimeline).toHaveBeenCalledWith("session-1");
    expect(mocks.clearCurrent.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.clearTimeline.mock.invocationCallOrder[0]
    );
  });

  it("preserves the timeline and does not invoke legacy clear when the session is busy", async () => {
    const busy = new Error("another request is already running for this agent session");
    mocks.clearCurrent.mockRejectedValueOnce(busy);

    await expect(clearConversation("session-1")).rejects.toBe(busy);

    expect(mocks.clearLegacy).not.toHaveBeenCalled();
    expect(mocks.clearTimeline).not.toHaveBeenCalled();
  });

  it("uses the legacy command only for an explicit unavailable-command error", async () => {
    mocks.clearCurrent.mockRejectedValueOnce(
      new Error("command 'clear_ai_conversation_session' not found")
    );

    await clearConversation("session-1");

    expect(mocks.clearLegacy).toHaveBeenCalledWith("session-1");
    expect(mocks.clearTimeline).toHaveBeenCalledWith("session-1");
  });
});
