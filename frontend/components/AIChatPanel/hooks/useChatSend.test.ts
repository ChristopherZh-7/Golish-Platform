import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/store";
import { shouldInjectPentestSystemPrompt } from "./useChatSend";

const aiMocks = vi.hoisted(() => ({
  cancelAiGeneration: vi.fn(),
  clearGenerationSuppressForAiSession: vi.fn(),
  createTextPayload: vi.fn((text: string) => ({ parts: [{ type: "text", text }] })),
  discardPendingBatchedDeltasForAiSession: vi.fn(),
  sendPromptSession: vi.fn(),
  sendPromptWithAttachments: vi.fn(),
  setExecutionMode: vi.fn(),
  suppressGenerationForAiSession: vi.fn(),
}));

vi.mock("@/lib/ai", () => aiMocks);

import { type ChatInteractionLane, useChatSend } from "./useChatSend";

describe("useChatSend system prompt injection", () => {
  it("keeps full pentest context only in chat mode", () => {
    expect(shouldInjectPentestSystemPrompt("chat")).toBe(true);
    expect(shouldInjectPentestSystemPrompt("assessment")).toBe(false);
    expect(shouldInjectPentestSystemPrompt("red_team")).toBe(false);
    expect(shouldInjectPentestSystemPrompt("task")).toBe(false);
  });
});

describe("useChatSend execution profile authority", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useStore.setState({
      conversations: {},
      activeConversationId: null,
      conversationOrder: [],
      conversationTerminals: {},
      sessions: {},
      activeSessionId: null,
    });
    useStore.getState().addConversation({
      id: "conv-profile-sync",
      title: "New Chat",
      messages: [],
      createdAt: 0,
      aiSessionId: "ai-profile-sync",
      aiInitialized: true,
      isStreaming: false,
    });
  });

  it("does not send a prompt when the backend rejects the selected profile", async () => {
    aiMocks.setExecutionMode.mockRejectedValueOnce(new Error("profile sync failed"));
    aiMocks.sendPromptSession.mockRejectedValueOnce(new Error("prompt must not be sent"));
    const taskInProgressRef = { current: false };
    const { result } = renderHook(() =>
      useChatSend({
        input: "",
        setInput: vi.fn(),
        isStreaming: false,
        activeConvId: "conv-profile-sync",
        imageAttachments: [],
        setImageAttachments: vi.fn(),
        textareaRef: { current: null },
        userScrolledUpRef: { current: false },
        streamingMsgRef: { current: null },
        chatExecutionModeRef: { current: "pentest" },
        taskInProgressRef,
        initializeSession: vi.fn().mockResolvedValue(true),
        buildPentestSystemPrompt: vi.fn(() => ""),
        createTerminalTab: vi.fn().mockResolvedValue(null),
        t: (_key, fallback) => fallback ?? "",
      })
    );

    let sent: boolean | undefined;
    await act(async () => {
      sent = await result.current.handleSend("测试广州有创");
    });

    expect(aiMocks.setExecutionMode).toHaveBeenCalledWith("ai-profile-sync", "pentest");
    expect(aiMocks.sendPromptSession).not.toHaveBeenCalled();
    expect(aiMocks.sendPromptWithAttachments).not.toHaveBeenCalled();
    expect(taskInProgressRef.current).toBe(false);
    const conversation = useStore.getState().conversations["conv-profile-sync"];
    expect(conversation.isStreaming).toBe(false);
    expect(conversation.messages[conversation.messages.length - 1]?.error).toContain(
      "profile sync failed"
    );
    expect(sent).toBe(false);
  });

  it("refuses every prompt while a destructive stage reset owns the send lane", async () => {
    const { result } = renderHook(() =>
      useChatSend({
        input: "must not race reset",
        setInput: vi.fn(),
        isStreaming: false,
        activeConvId: "conv-profile-sync",
        imageAttachments: [],
        setImageAttachments: vi.fn(),
        textareaRef: { current: null },
        userScrolledUpRef: { current: false },
        streamingMsgRef: { current: null },
        chatExecutionModeRef: { current: "pentest" },
        taskInProgressRef: { current: false },
        interactionLaneRef: { current: "reset" },
        initializeSession: vi.fn().mockResolvedValue(true),
        buildPentestSystemPrompt: vi.fn(() => ""),
        createTerminalTab: vi.fn().mockResolvedValue(null),
        t: (_key, fallback) => fallback ?? "",
      })
    );

    let sent: boolean | undefined;
    await act(async () => {
      sent = await result.current.handleSend();
    });

    expect(sent).toBe(false);
    expect(aiMocks.setExecutionMode).not.toHaveBeenCalled();
    expect(aiMocks.sendPromptSession).not.toHaveBeenCalled();
    expect(useStore.getState().conversations["conv-profile-sync"].messages).toEqual([]);
  });

  it("claims the shared lane synchronously before asynchronous session initialization", async () => {
    let resolveInitialization: ((initialized: boolean) => void) | undefined;
    const initializeSession = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          resolveInitialization = resolve;
        })
    );
    const interactionLaneRef: { current: ChatInteractionLane } = { current: "idle" };
    const { result } = renderHook(() =>
      useChatSend({
        input: "claim before await",
        setInput: vi.fn(),
        isStreaming: false,
        activeConvId: "conv-profile-sync",
        imageAttachments: [],
        setImageAttachments: vi.fn(),
        textareaRef: { current: null },
        userScrolledUpRef: { current: false },
        streamingMsgRef: { current: null },
        chatExecutionModeRef: { current: "pentest" },
        taskInProgressRef: { current: false },
        interactionLaneRef,
        initializeSession,
        buildPentestSystemPrompt: vi.fn(() => ""),
        createTerminalTab: vi.fn().mockResolvedValue(null),
        t: (_key, fallback) => fallback ?? "",
      })
    );

    let send: Promise<boolean> | undefined;
    act(() => {
      send = result.current.handleSend();
    });
    expect(interactionLaneRef.current).toBe("send");

    await act(async () => {
      await vi.waitFor(() => expect(resolveInitialization).toBeTypeOf("function"));
      resolveInitialization?.(false);
      await send;
    });
    expect(interactionLaneRef.current).toBe("idle");
  });
});
