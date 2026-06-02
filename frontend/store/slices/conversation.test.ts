import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "../index";
import type { ChatConversation } from "./conversation";

/**
 * Covers the interleaved-thinking segmentation in `appendMessageThinking`:
 * a new segment must open whenever reasoning resumes after answer text or a
 * tool call, so the chat can render reasoning bursts in time order instead of
 * collapsing everything into one block.
 */
describe("Conversation Slice — thinking segments", () => {
  const CONV = "conv-thinking-test";

  const lastMsg = () => {
    const c = useStore.getState().conversations[CONV];
    return c.messages[c.messages.length - 1];
  };

  beforeEach(() => {
    useStore.setState({
      conversations: {},
      activeConversationId: null,
      conversationOrder: [],
      conversationTerminals: {},
    });

    const conv: ChatConversation = {
      id: CONV,
      title: "t",
      messages: [],
      createdAt: 0,
      aiSessionId: CONV,
      aiInitialized: false,
      isStreaming: true,
    };
    useStore.getState().addConversation(conv);
    useStore.getState().addConversationMessage(CONV, {
      id: "m1",
      role: "assistant",
      content: "",
      timestamp: 0,
      isStreaming: true,
    });
  });

  it("merges consecutive reasoning chunks into a single segment", () => {
    useStore.getState().appendMessageThinking(CONV, "think ");
    useStore.getState().appendMessageThinking(CONV, "more");

    const segs = lastMsg().thinkingSegments;
    expect(segs).toHaveLength(1);
    expect(segs?.[0].content).toBe("think more");
    expect(segs?.[0].contentOffset).toBe(0);
    expect(segs?.[0].toolIndex).toBe(0);
  });

  it("opens a new segment when reasoning resumes after answer text", () => {
    useStore.getState().appendMessageThinking(CONV, "first");
    useStore.getState().appendMessageDelta(CONV, "hello");
    useStore.getState().appendMessageThinking(CONV, "second");

    const segs = lastMsg().thinkingSegments;
    expect(segs).toHaveLength(2);
    expect(segs?.[0].content).toBe("first");
    expect(segs?.[1].content).toBe("second");
    expect(segs?.[1].contentOffset).toBe("hello".length);
    expect(segs?.[1].toolIndex).toBe(0);
  });

  it("opens a new segment when reasoning resumes after a tool call", () => {
    useStore.getState().appendMessageThinking(CONV, "first");
    useStore.getState().addMessageToolCall(CONV, { name: "grep", args: "{}", requestId: "r1" });
    useStore.getState().appendMessageThinking(CONV, "second");

    const segs = lastMsg().thinkingSegments;
    expect(segs).toHaveLength(2);
    expect(segs?.[1].toolIndex).toBe(1);
    expect(segs?.[1].content).toBe("second");
  });

  it("keeps the merged thinking string in sync for history fallback", () => {
    useStore.getState().appendMessageThinking(CONV, "a");
    useStore.getState().appendMessageDelta(CONV, "x");
    useStore.getState().appendMessageThinking(CONV, "b");

    expect(lastMsg().thinking).toBe("ab");
    expect(lastMsg().thinkingSegments).toHaveLength(2);
  });
});

describe("Conversation Slice — stage markers", () => {
  const CONV = "conv-stage-test";

  const messages = () => useStore.getState().conversations[CONV].messages;

  beforeEach(() => {
    useStore.setState({
      conversations: {},
      activeConversationId: null,
      conversationOrder: [],
      conversationTerminals: {},
    });
    const conv: ChatConversation = {
      id: CONV,
      title: "t",
      messages: [],
      createdAt: 0,
      aiSessionId: CONV,
      aiInitialized: false,
      isStreaming: false,
    };
    useStore.getState().addConversation(conv);
  });

  it("appends a system divider message carrying the stage event", () => {
    useStore.getState().addConversationStageMarker(CONV, {
      kind: "subtask_completed",
      label: "Stage complete: Scoping",
      title: "Scoping",
      detail: "scope defined",
    });

    const msgs = messages();
    expect(msgs).toHaveLength(1);
    expect(msgs[0].role).toBe("system");
    expect(msgs[0].content).toBe("Stage complete: Scoping");
    expect(msgs[0].stageEvent?.kind).toBe("subtask_completed");
    expect(msgs[0].stageEvent?.detail).toBe("scope defined");
  });

  it("de-dupes an identical consecutive marker", () => {
    const marker = { kind: "task_progress" as const, label: "Task complete", status: "finished" };
    useStore.getState().addConversationStageMarker(CONV, marker);
    useStore.getState().addConversationStageMarker(CONV, marker);
    expect(messages()).toHaveLength(1);
  });

  it("keeps distinct consecutive markers", () => {
    useStore.getState().addConversationStageMarker(CONV, {
      kind: "subtask_completed",
      label: "Stage complete: Scoping",
    });
    useStore.getState().addConversationStageMarker(CONV, {
      kind: "subtask_completed",
      label: "Stage complete: Reconnaissance",
    });
    expect(messages()).toHaveLength(2);
  });
});
