import { act, render, screen, waitFor } from "@testing-library/react";
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

  it("labels target-scope review separately from generic approval", async () => {
    await mount(ref(true));

    fire({
      type: "task_progress",
      task_id: "t",
      status: "waiting_target_scope",
      message: "ACTIVE_RECON_TRUSTED_TARGET_REQUIRED: review exact targets.",
    });

    const marker = useStore
      .getState()
      .conversations[CONV].messages.find(
        (message) => message.stageEvent?.status === "waiting_target_scope"
      );
    expect(marker?.stageEvent?.label).toBe("Review scan targets");
    expect(marker?.stageEvent?.detail).toContain("review exact targets");
  });
});

function AskHumanHarness() {
  const { askHumanRequest, lastDiscoverOrgId, lastDiscoverThreshold } = useAiChatEvents({
    activeConvId: CONV,
    streamingMsgRef: ref<string | null>(null),
    taskInProgressRef: ref(false),
    modes: { setPendingApproval: vi.fn(), pendingApprovalRef: ref(null) },
    generateTitleRef: ref(null),
  });
  return (
    <div>
      <div data-testid="ask-human">{askHumanRequest ? "present" : "absent"}</div>
      <div data-testid="ask-human-raw-type">{askHumanRequest?.rawInputType ?? "none"}</div>
      <div data-testid="ask-human-effective-type">{askHumanRequest?.inputType ?? "none"}</div>
      <div data-testid="discover-org">{lastDiscoverOrgId ?? "none"}</div>
      <div data-testid="discover-threshold">{lastDiscoverThreshold ?? "none"}</div>
    </div>
  );
}

describe("useAiChatEvents — ask_human lifecycle", () => {
  beforeEach(() => {
    aiMock.cb = null;
    setup();
  });

  const fire = (e: Record<string, unknown>) =>
    act(() => {
      aiMock.cb?.({ session_id: CONV, ...e });
    });

  it("clears a pending ask_human box when the run errors out (no dangling box)", async () => {
    // Regression: the backend's ask_human timeout sends no event and the box was
    // only cleared on submit/skip, so an errored run left it stuck forever.
    render(<AskHumanHarness />);
    await waitFor(() => expect(aiMock.cb).toBeTruthy());

    fire({
      type: "ask_human_request",
      request_id: "r-err",
      question: "Which scope?",
      input_type: "freetext",
      options: [],
      context: "",
    });
    expect(screen.getByTestId("ask-human")).toHaveTextContent("present");

    fire({ type: "error", message: "model exploded" });
    expect(screen.getByTestId("ask-human")).toHaveTextContent("absent");
  });

  it("clears a pending ask_human box when the response event arrives", async () => {
    render(<AskHumanHarness />);
    await waitFor(() => expect(aiMock.cb).toBeTruthy());

    fire({
      type: "ask_human_request",
      request_id: "r-done",
      question: "Confirm units?",
      input_type: "unit_review",
      options: [],
      context: "",
    });
    expect(screen.getByTestId("ask-human")).toHaveTextContent("present");

    fire({
      type: "ask_human_response",
      request_id: "r-done",
      response: "ok",
      skipped: false,
    });
    expect(screen.getByTestId("ask-human")).toHaveTextContent("absent");
  });

  it("preserves an unknown raw input type when options require choice rendering", async () => {
    render(<AskHumanHarness />);
    await waitFor(() => expect(aiMock.cb).toBeTruthy());

    fire({
      type: "ask_human_request",
      request_id: "r-unknown-choice",
      question: "Pick one",
      input_type: "future_security_decision",
      options: ["Approve", "Decline"],
      context: "",
    });

    expect(screen.getByTestId("ask-human-raw-type")).toHaveTextContent(
      "future_security_decision"
    );
    expect(screen.getByTestId("ask-human-effective-type")).toHaveTextContent("choice");
  });

  it("captures the org id from a recon_discover_subsidiaries tool call (unit_review fallback)", async () => {
    render(<AskHumanHarness />);
    await waitFor(() => expect(aiMock.cb).toBeTruthy());

    fire({
      type: "tool_auto_approved",
      tool_name: "recon_discover_subsidiaries",
      request_id: "d1",
      args: { organization_id: "140315f6-990e-4c5c-a04b-73b14310bf22", min_ownership_percent: 51 },
    });
    expect(screen.getByTestId("discover-org")).toHaveTextContent(
      "140315f6-990e-4c5c-a04b-73b14310bf22"
    );
    expect(screen.getByTestId("discover-threshold")).toHaveTextContent("51");
  });

  it("captures a string min_ownership_percent from discover (e.g. \"51%\")", async () => {
    render(<AskHumanHarness />);
    await waitFor(() => expect(aiMock.cb).toBeTruthy());

    fire({
      type: "tool_auto_approved",
      tool_name: "recon_discover_subsidiaries",
      request_id: "d3",
      args: JSON.stringify({
        organization_id: "140315f6-990e-4c5c-a04b-73b14310bf22",
        min_ownership_percent: "51%",
      }),
    });
    expect(screen.getByTestId("discover-threshold")).toHaveTextContent("51");
  });

  it("ignores a non-uuid organization_id from discover so it never poisons the fallback", async () => {
    render(<AskHumanHarness />);
    await waitFor(() => expect(aiMock.cb).toBeTruthy());

    fire({
      type: "tool_auto_approved",
      tool_name: "recon_discover_subsidiaries",
      request_id: "d2",
      args: { organization_id: "None" },
    });
    expect(screen.getByTestId("discover-org")).toHaveTextContent("none");
  });
});
