import { act, render, waitFor } from "@testing-library/react";
import type { MutableRefObject } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Capture the `onAiEvent` listener so the test can drive events synchronously.
// `vi.hoisted` keeps the holder available inside the hoisted `vi.mock` factory.
const aiMock = vi.hoisted(() => ({
  cb: null as ((e: Record<string, unknown>) => void) | null,
}));

vi.mock("@/lib/ai", () => ({
  onAiEvent: (cb: (e: Record<string, unknown>) => void) => {
    aiMock.cb = cb;
    return Promise.resolve(() => {});
  },
  isTitleGenSessionId: () => false,
  isGenerationSuppressedForAiSession: () => false,
  respondToToolApproval: vi.fn(() => Promise.resolve()),
}));

import { useStore } from "@/store";
import { useAiChatEvents } from "./useAiChatEvents";

const CONV = "conv-prep-lifecycle";

function ref<T>(v: T): MutableRefObject<T> {
  return { current: v };
}

function Harness({ taskInProgressRef }: { taskInProgressRef: MutableRefObject<boolean> }) {
  useAiChatEvents({
    activeConvId: CONV,
    streamingMsgRef: ref<string | null>(null),
    taskInProgressRef,
    modes: { setPendingApproval: vi.fn(), pendingApprovalRef: ref(null) },
    generateTitleRef: ref(null),
  });
  return null;
}

const isStreaming = () => useStore.getState().conversations[CONV].isStreaming;

function setup() {
  useStore.setState({
    conversations: {},
    activeConversationId: CONV,
    conversationOrder: [],
    conversationTerminals: {},
  });
  useStore.getState().addConversation({
    id: CONV,
    title: "t",
    messages: [],
    createdAt: 0,
    aiSessionId: CONV,
    aiInitialized: true,
    isStreaming: false,
  });
}

describe("useAiChatEvents — task preparing lifecycle", () => {
  beforeEach(() => {
    aiMock.cb = null;
    setup();
  });

  async function mount(taskInProgressRef: MutableRefObject<boolean>) {
    render(<Harness taskInProgressRef={taskInProgressRef} />);
    await waitFor(() => expect(aiMock.cb).toBeTruthy());
  }

  const fire = (e: Record<string, unknown>) =>
    act(() => {
      aiMock.cb?.({ session_id: CONV, ...e });
    });

  it("clears streaming on task_progress 'finished' so the terminal completed can't re-arm it", async () => {
    // Regression: a harness "hold for rework" Interrupt suspends the send invoke,
    // so taskInProgressRef never reset and the terminal `completed` re-armed the
    // preparing spinner forever. `task_progress: finished` must break that.
    const taskInProgressRef = ref(true);
    await mount(taskInProgressRef);
    act(() => useStore.getState().setConversationStreaming(CONV, true));

    fire({ type: "task_progress", task_id: "t", status: "finished", message: "" });
    expect(taskInProgressRef.current).toBe(false);
    expect(isStreaming()).toBe(false);

    // The final report burst that follows the end-of-run signal must NOT re-arm it.
    fire({ type: "started", turn_id: "x" });
    fire({ type: "text_delta", delta: "Report.", accumulated: "Report." });
    fire({ type: "completed", response: "Report.", reasoning: null });
    expect(isStreaming()).toBe(false);
  });

  it("still bridges streaming across a subtask completed while the task is in progress", async () => {
    const taskInProgressRef = ref(true);
    await mount(taskInProgressRef);
    act(() => useStore.getState().setConversationStreaming(CONV, true));

    // A subtask (not the whole task) completing keeps the spinner armed.
    fire({ type: "completed", response: "subtask done", reasoning: null });
    expect(isStreaming()).toBe(true);
  });
});
