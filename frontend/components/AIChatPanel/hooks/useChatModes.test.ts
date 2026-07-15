import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LAST_MODE_STORAGE_KEY } from "@/lib/ai/execution-mode";
import { useStore } from "@/store";

const aiMocks = vi.hoisted(() => ({
  respondToToolApproval: vi.fn(),
  setAgentMode: vi.fn(),
  setExecutionMode: vi.fn(),
}));
const dbSyncMocks = vi.hoisted(() => ({ flushDbSave: vi.fn() }));

vi.mock("@/lib/ai", () => aiMocks);
vi.mock("@/lib/conversation-db-sync", () => dbSyncMocks);

import { useChatModes } from "./useChatModes";

describe("useChatModes execution profile commit", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    globalThis.localStorage.clear();
    dbSyncMocks.flushDbSave.mockResolvedValue(undefined);
    useStore.setState({
      conversations: {},
      activeConversationId: null,
      conversationOrder: [],
      conversationTerminals: {},
      sessions: {},
    });
    useStore.getState().addConversation({
      id: "conv-mode",
      title: "Mode",
      messages: [],
      createdAt: 0,
      aiSessionId: "ai-mode",
      aiInitialized: true,
      isStreaming: false,
    });
    useStore.getState().setActiveConversation("conv-mode");
  });

  it("keeps UI and persistence unchanged when backend profile update fails", async () => {
    aiMocks.setExecutionMode.mockRejectedValueOnce(new Error("backend profile rejected"));
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    const { result } = renderHook(() => useChatModes());

    let changed: unknown;
    await act(async () => {
      changed = await result.current.handleExecutionModeChange("pentest");
    });

    expect(changed).toBe(false);
    expect(result.current.chatExecutionMode).toBe("chat");
    expect(result.current.chatExecutionModeRef.current).toBe("chat");
    expect(globalThis.localStorage.getItem(LAST_MODE_STORAGE_KEY)).toBeNull();
    const messages = useStore.getState().conversations["conv-mode"].messages;
    expect(messages[messages.length - 1]?.error).toContain("backend profile rejected");
    consoleError.mockRestore();
  });

  it("commits UI and persistence only after backend accepts the profile", async () => {
    aiMocks.setExecutionMode.mockResolvedValueOnce(undefined);
    const { result } = renderHook(() => useChatModes());

    let changed: unknown;
    await act(async () => {
      changed = await result.current.handleExecutionModeChange("pentest");
    });

    expect(changed).toBe(true);
    expect(aiMocks.setExecutionMode).toHaveBeenCalledWith("ai-mode", "pentest");
    expect(result.current.chatExecutionMode).toBe("pentest");
    expect(result.current.chatExecutionModeRef.current).toBe("pentest");
    expect(globalThis.localStorage.getItem(LAST_MODE_STORAGE_KEY)).toBe("pentest");
  });
});
