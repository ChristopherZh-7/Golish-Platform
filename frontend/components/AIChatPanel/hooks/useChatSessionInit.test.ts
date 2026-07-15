import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/store";

const aiMocks = vi.hoisted(() => ({
  cancelAiGeneration: vi.fn(),
  initAiSession: vi.fn(),
  restoreAiConversation: vi.fn(),
  sendPromptSession: vi.fn(),
  setAgentMode: vi.fn(),
  setExecutionMode: vi.fn(),
  shutdownAiSession: vi.fn(),
}));
const settingsMocks = vi.hoisted(() => ({ getSettings: vi.fn() }));
const providerMocks = vi.hoisted(() => ({ buildProviderConfig: vi.fn() }));

vi.mock("@/lib/ai", () => ({
  ...aiMocks,
  normalizeExecutionModeId: (mode: string) => (mode === "task" ? "assessment" : mode),
  titleGenSessionId: (id: string) => `title-${id}`,
}));
vi.mock("@/lib/settings", () => settingsMocks);
vi.mock("../providerConfig", () => providerMocks);

import { useChatSessionInit } from "./useChatSessionInit";

describe("useChatSessionInit execution profile authority", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    settingsMocks.getSettings.mockResolvedValue({});
    providerMocks.buildProviderConfig.mockReturnValue({
      provider: "openai",
      model: "gpt-test",
    });
    aiMocks.initAiSession.mockResolvedValue(undefined);
    aiMocks.setAgentMode.mockResolvedValue(undefined);
    aiMocks.shutdownAiSession.mockResolvedValue(undefined);
    useStore.setState({
      conversations: {},
      activeConversationId: null,
      conversationOrder: [],
      conversationTerminals: {},
      sessions: {},
      currentProjectPath: "/tmp/golish-profile-test",
      approvalMode: "ask",
    });
    useStore.getState().addConversation({
      id: "conv-init-profile",
      title: "Profile init",
      messages: [],
      createdAt: 0,
      aiSessionId: "ai-init-profile",
      aiInitialized: false,
      isStreaming: false,
    });
  });

  it("does not mark a session initialized when backend profile restore fails", async () => {
    aiMocks.setExecutionMode.mockRejectedValueOnce(new Error("profile restore failed"));
    const updateConv = vi.fn();
    const setChatExecutionMode = vi.fn();
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    const { result } = renderHook(() =>
      useChatSessionInit({
        selectedModel: { provider: "openai", model: "gpt-test" },
        chatExecutionModeRef: { current: "pentest" },
        setChatExecutionMode,
        updateConv,
      })
    );

    let initialized = true;
    await act(async () => {
      initialized = await result.current.initializeSession({
        id: "conv-init-profile",
        aiSessionId: "ai-init-profile",
        aiInitialized: false,
      });
    });

    expect(aiMocks.setExecutionMode).toHaveBeenCalledWith("ai-init-profile", "pentest");
    expect(initialized).toBe(false);
    expect(updateConv).not.toHaveBeenCalled();
    expect(setChatExecutionMode).not.toHaveBeenCalled();
    consoleError.mockRestore();
  });
});
