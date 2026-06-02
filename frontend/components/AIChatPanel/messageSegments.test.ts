import { describe, expect, it } from "vitest";
import type { ChatMessage, ChatToolCall, ThinkingSegment } from "@/store/slices/conversation";
import { buildMessageSegments, type MessageSegment } from "./messageSegments";

function think(content: string, contentOffset: number, toolIndex: number): ThinkingSegment {
  return { content, startedAt: 0, endedAt: 1000, contentOffset, toolIndex };
}

function tool(name: string, requestId: string, success = true): ChatToolCall {
  return { name, args: "{}", requestId, success };
}

function msg(partial: Partial<ChatMessage>): ChatMessage {
  return { id: "m1", role: "assistant", content: "", timestamp: 0, ...partial };
}

/** Compact, order-preserving view of the segment list for readable assertions. */
function shape(segments: MessageSegment[]): string[] {
  return segments.map((s): string => {
    if (s.kind === "text") return `text:${s.content}`;
    if (s.kind === "thinking") return `think:${s.seg.content}`;
    if (s.kind === "tools") return `tools:${s.calls.map((c) => c.name).join(",")}`;
    if (s.kind === "sub_agent") return `sub:${s.toolCall.name}`;
    return "plan";
  });
}

describe("buildMessageSegments", () => {
  it("interleaves multiple reasoning bursts between answer-text chunks when there are no tool calls", () => {
    // Regression: previously every burst with the same toolIndex was stacked at
    // the top, so the user saw all "Thought for …" blocks first, then all prose.
    const message = msg({
      content: "ALPHABRAVO",
      thinking: "R1R2",
      thinkingSegments: [think("R1", 0, 0), think("R2", 5, 0)],
    });
    expect(shape(buildMessageSegments(message))).toEqual([
      "think:R1",
      "text:ALPHA",
      "think:R2",
      "text:BRAVO",
    ]);
  });

  it("keeps a single leading reasoning burst above the answer text", () => {
    const message = msg({
      content: "hello",
      thinking: "R1",
      thinkingSegments: [think("R1", 0, 0)],
    });
    expect(shape(buildMessageSegments(message))).toEqual(["think:R1", "text:hello"]);
  });

  it("interleaves reasoning, text, and a tool call using per-tool offsets", () => {
    const message = msg({
      content: "helloworld",
      thinkingSegments: [think("R1", 0, 0)],
      toolCalls: [tool("read_file", "r1")],
      toolCallOffsets: [5],
    });
    expect(shape(buildMessageSegments(message))).toEqual([
      "think:R1",
      "text:hello",
      "tools:read_file",
      "text:world",
    ]);
  });

  it("splices multiple reasoning bursts inside a single pre-tool text window", () => {
    const message = msg({
      content: "AAABBB",
      thinkingSegments: [think("R1", 0, 0), think("R2", 3, 0)],
      toolCalls: [tool("read_file", "r1")],
      toolCallOffsets: [6],
    });
    expect(shape(buildMessageSegments(message))).toEqual([
      "think:R1",
      "text:AAA",
      "think:R2",
      "text:BBB",
      "tools:read_file",
    ]);
  });

  it("places reasoning that resumed after a tool call below that tool", () => {
    const message = msg({
      content: "XY",
      thinkingSegments: [think("R1", 0, 0), think("R2", 1, 1)],
      toolCalls: [tool("read_file", "r1")],
      toolCallOffsets: [1],
    });
    expect(shape(buildMessageSegments(message))).toEqual([
      "think:R1",
      "text:X",
      "tools:read_file",
      "think:R2",
      "text:Y",
    ]);
  });

  it("groups consecutive visible tool calls into one batch", () => {
    const message = msg({
      content: "",
      toolCalls: [tool("read_file", "r1"), tool("grep_file", "r2")],
      toolCallOffsets: [0, 0],
    });
    expect(shape(buildMessageSegments(message))).toEqual(["tools:read_file,grep_file"]);
  });

  it("renders sub-agent calls as their own card and breaks the tool batch", () => {
    const message = msg({
      content: "",
      toolCalls: [tool("read_file", "r1"), tool("sub_agent_pentester", "r2")],
      toolCallOffsets: [0, 0],
    });
    expect(shape(buildMessageSegments(message))).toEqual([
      "tools:read_file",
      "sub:sub_agent_pentester",
    ]);
  });

  it("emits a plan marker for update_plan tool calls", () => {
    const message = msg({
      content: "",
      toolCalls: [tool("update_plan", "r1")],
      toolCallOffsets: [0],
    });
    expect(shape(buildMessageSegments(message))).toEqual(["plan"]);
  });

  it("falls back to a single text segment when there are no reasoning segments", () => {
    const message = msg({ content: "restored answer" });
    expect(shape(buildMessageSegments(message))).toEqual(["text:restored answer"]);
  });

  it("shows a streaming placeholder when nothing has arrived yet", () => {
    const message = msg({ content: "", isStreaming: true });
    expect(shape(buildMessageSegments(message))).toEqual(["text:..."]);
  });

  it("treats a user message as plain text", () => {
    const message = msg({ role: "user", content: "what is my IP?" });
    expect(shape(buildMessageSegments(message))).toEqual(["text:what is my IP?"]);
  });
});
