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

describe("Conversation Slice — error severity & de-dupe", () => {
  const CONV = "conv-error-test";

  const messages = () => useStore.getState().conversations[CONV].messages;

  beforeEach(() => {
    useStore.setState({
      conversations: {},
      activeConversationId: null,
      conversationOrder: [],
      conversationTerminals: {},
    });
    useStore.getState().addConversation({
      id: CONV,
      title: "t",
      messages: [],
      createdAt: 0,
      aiSessionId: CONV,
      aiInitialized: false,
      isStreaming: true,
    });
    useStore.getState().addConversationMessage(CONV, {
      id: "m1",
      role: "assistant",
      content: "",
      timestamp: 0,
      isStreaming: true,
    });
  });

  it("records the severity on the streaming message and stops streaming", () => {
    useStore.getState().setMessageError(CONV, "planner declined", "warning");
    const last = messages()[messages().length - 1];
    expect(last.error).toBe("planner declined");
    expect(last.errorSeverity).toBe("warning");
    expect(last.isStreaming).toBe(false);
  });

  it("defaults severity to error when omitted", () => {
    useStore.getState().setMessageError(CONV, "boom");
    expect(messages()[0].errorSeverity).toBe("error");
  });

  it("collapses the same failure surfaced twice into one message (keeps the shorter text)", () => {
    const clean = "Generator failed: declined to produce a plan";
    const wrapped = `[API trace=abc] send_ai_prompt_session: ${clean}`;
    // First surfacing (backend error event) lands on the streaming message.
    useStore.getState().setMessageError(CONV, clean, "warning");
    // Second surfacing (invoke rejection) must NOT push a duplicate bubble.
    useStore.getState().setMessageError(CONV, wrapped, "warning");

    expect(messages()).toHaveLength(1);
    expect(messages()[0].error).toBe(clean);
    expect(messages()[0].errorSeverity).toBe("warning");
  });

  it("escalates to a hard error if the duplicate is more severe", () => {
    useStore.getState().setMessageError(CONV, "declined to produce a plan", "warning");
    useStore.getState().setMessageError(CONV, "x declined to produce a plan x", "error");
    expect(messages()).toHaveLength(1);
    expect(messages()[0].errorSeverity).toBe("error");
  });

  it("still pushes a separate bubble for an unrelated error", () => {
    useStore.getState().setMessageError(CONV, "first failure", "error");
    useStore.getState().setMessageError(CONV, "totally different failure", "error");
    expect(messages()).toHaveLength(2);
  });
});

describe("Conversation Slice — tool result correlation", () => {
  const CONV = "conv-tool-result-test";

  const toolCalls = () => {
    const conv = useStore.getState().conversations[CONV];
    return conv.messages.flatMap((m) => m.toolCalls ?? []);
  };

  beforeEach(() => {
    useStore.setState({
      conversations: {},
      activeConversationId: null,
      conversationOrder: [],
      conversationTerminals: {},
    });
    useStore.getState().addConversation({
      id: CONV,
      title: "t",
      messages: [],
      createdAt: 0,
      aiSessionId: CONV,
      aiInitialized: false,
      isStreaming: true,
    });
    useStore.getState().addConversationMessage(CONV, {
      id: "m1",
      role: "assistant",
      content: "",
      timestamp: 0,
      isStreaming: true,
    });
  });

  it("updates same-name tool calls by request id instead of the newest name match", () => {
    const store = useStore.getState();
    store.addMessageToolCall(CONV, { name: "pentest_run", args: "{}", requestId: "r1" });
    store.addMessageToolCall(CONV, { name: "pentest_run", args: "{}", requestId: "r2" });

    store.updateMessageToolResult(CONV, "pentest_run", '{"status":"backgrounded"}', true, "r1");

    expect(toolCalls().find((tc) => tc.requestId === "r1")?.result).toBe(
      '{"status":"backgrounded"}'
    );
    expect(toolCalls().find((tc) => tc.requestId === "r2")?.result).toBeUndefined();
  });

  it("updates a backgrounded tool call when completion arrives by job id", () => {
    const store = useStore.getState();
    store.addMessageToolCall(CONV, { name: "pentest_run", args: "{}", requestId: "r1" });
    store.updateMessageToolResult(
      CONV,
      "pentest_run",
      '{"status":"backgrounded","job_id":"job_42"}',
      true,
      "r1"
    );

    store.updateMessageToolResultByJobId(
      CONV,
      "job_42",
      '{"status":"done","job_id":"job_42","stdout":"ok"}',
      true
    );

    expect(toolCalls()[0].result).toBe('{"status":"done","job_id":"job_42","stdout":"ok"}');
    expect(toolCalls()[0].success).toBe(true);
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
      label: "Step complete: Scoping",
    });
    useStore.getState().addConversationStageMarker(CONV, {
      kind: "subtask_completed",
      label: "Step complete: Reconnaissance",
    });
    expect(messages()).toHaveLength(2);
  });

  it("carries a stage_completed milestone distinct from per-step markers", () => {
    useStore.getState().addConversationStageMarker(CONV, {
      kind: "subtask_completed",
      label: "Step complete: DNS recon",
    });
    useStore.getState().addConversationStageMarker(CONV, {
      kind: "stage_completed",
      label: "Stage complete: Scoping",
      status: "finished",
    });

    const msgs = messages();
    expect(msgs).toHaveLength(2);
    // The per-step marker and the whole-stage milestone are kept separate so the
    // UI can render them with different prominence.
    expect(msgs[0].stageEvent?.kind).toBe("subtask_completed");
    expect(msgs[1].stageEvent?.kind).toBe("stage_completed");
    expect(msgs[1].content).toBe("Stage complete: Scoping");
  });
});

/**
 * A multi-stage harness run opens a new assistant message per stage (often with no
 * `completed` between them, and with stage-divider system messages interleaved), so
 * the previous stage's "Writing response" footer must not linger after the run has
 * advanced. The footer renders whenever a message keeps `isStreaming: true`.
 */
describe("Conversation Slice — multi-stage streaming footer", () => {
  const CONV = "conv-multistage-test";
  const messages = () => useStore.getState().conversations[CONV].messages;
  const byId = (id: string) => messages().find((m) => m.id === id);

  beforeEach(() => {
    useStore.setState({
      conversations: {},
      activeConversationId: null,
      conversationOrder: [],
      conversationTerminals: {},
    });
    useStore.getState().addConversation({
      id: CONV,
      title: "t",
      messages: [],
      createdAt: 0,
      aiSessionId: CONV,
      aiInitialized: false,
      isStreaming: true,
    });
  });

  it("clears a prior stage's isStreaming when the next stage's message opens", () => {
    const s = useStore.getState();
    s.addConversationMessage(CONV, {
      id: "stage1",
      role: "assistant",
      content: "scoping done",
      timestamp: 0,
      isStreaming: true,
    });
    // Divider sits between the two assistant messages (so stage1 is no longer last).
    s.addConversationStageMarker(CONV, {
      kind: "stage_completed",
      label: "Stage complete: Scoping",
      status: "finished",
    });
    s.addConversationMessage(CONV, {
      id: "stage2",
      role: "assistant",
      content: "",
      timestamp: 1,
      isStreaming: true,
    });

    expect(byId("stage1")?.isStreaming).toBe(false);
    expect(byId("stage2")?.isStreaming).toBe(true);
  });

  it("finalizes the streaming assistant message even when a system marker is last", () => {
    const s = useStore.getState();
    s.addConversationMessage(CONV, {
      id: "stage1",
      role: "assistant",
      content: "scoping",
      timestamp: 0,
      isStreaming: true,
    });
    // `completed` arrives after the divider was appended (divider is now last).
    s.addConversationStageMarker(CONV, {
      kind: "stage_completed",
      label: "Stage complete: Scoping",
      status: "finished",
    });
    s.finalizeStreamingMessage(CONV, "scoping final");

    expect(byId("stage1")?.isStreaming).toBe(false);
    expect(byId("stage1")?.content).toBe("scoping final");
    expect(useStore.getState().conversations[CONV].isStreaming).toBe(false);
  });
});
