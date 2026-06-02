import type { ChatMessage, ChatToolCall, ThinkingSegment } from "@/store/slices/conversation";

/**
 * A renderable chunk of an assistant message, in true time order: answer-text
 * runs, grouped tool-call batches, sub-agent cards, the task-plan anchor, and
 * reasoning ("thinking") bursts.
 */
export type MessageSegment =
  | { kind: "text"; content: string }
  | { kind: "tools"; calls: ChatToolCall[]; requestIds: string[] }
  | { kind: "sub_agent"; requestId: string; toolCall: ChatToolCall }
  | { kind: "plan_marker" }
  | { kind: "thinking"; seg: ThinkingSegment };

/**
 * Turn a streamed message into an ordered segment list the chat can render
 * top-to-bottom.
 *
 * Reasoning bursts are anchored by BOTH the tool index they preceded and their
 * `contentOffset`, so that thinking which resumed *between* two answer-text
 * chunks (with no tool call in between) is spliced into the prose at the right
 * spot instead of being stacked at the top of the message. This is the core of
 * the "interleave thinking with content" behaviour.
 */
export function buildMessageSegments(
  message: ChatMessage,
  nestedRequestIds: ReadonlySet<string> = new Set(),
  pendingApproval?: { requestId: string; toolName: string } | null
): MessageSegment[] {
  const isUser = message.role === "user";
  const thinkingSegments = message.thinkingSegments;
  const hasThinkingSegments = !!thinkingSegments && thinkingSegments.length > 0;
  const hasToolCalls = !isUser && !!message.toolCalls && message.toolCalls.length > 0;

  const isSubAgentCall = (tc: ChatToolCall) => tc.name.startsWith("sub_agent_");

  const isPendingApprovalCall = (tc: ChatToolCall) =>
    pendingApproval != null &&
    (tc.requestId
      ? tc.requestId === pendingApproval.requestId
      : tc.name === pendingApproval.toolName);

  const isVisibleCall = (tc: ChatToolCall) =>
    tc.name !== "update_plan" &&
    !isSubAgentCall(tc) &&
    !nestedRequestIds.has(tc.requestId ?? "") &&
    (tc.success !== undefined || !isPendingApprovalCall(tc));

  const segments: MessageSegment[] = [];

  const flushToolBatch = (toolBatch: ChatToolCall[], toolBatchIds: string[]) => {
    if (toolBatch.length > 0) {
      segments.push({ kind: "tools", calls: [...toolBatch], requestIds: [...toolBatchIds] });
    }
  };

  // Group reasoning bursts by the tool index they preceded. Within a tool window
  // bursts are also anchored by `contentOffset` (see emitTextWithThinking) so
  // reasoning that resumed between answer-text chunks — with no tool call in
  // between — splices into the text instead of stacking at the top.
  const thinkingByTool = new Map<number, ThinkingSegment[]>();
  if (hasThinkingSegments) {
    for (const ts of thinkingSegments) {
      const arr = thinkingByTool.get(ts.toolIndex);
      if (arr) arr.push(ts);
      else thinkingByTool.set(ts.toolIndex, [ts]);
    }
  }

  // Emit the answer text in [start, end) for one tool window, splicing in that
  // window's reasoning bursts at their content offsets so thinking and prose
  // interleave in true time order.
  const emitTextWithThinking = (start: number, end: number, toolIdx: number) => {
    const arr = thinkingByTool.get(toolIdx);
    let cursor = start;
    if (arr) {
      for (const ts of arr) {
        const anchor = Math.min(Math.max(ts.contentOffset, start), end);
        if (anchor > cursor) {
          segments.push({ kind: "text", content: message.content.slice(cursor, anchor) });
          cursor = anchor;
        }
        segments.push({ kind: "thinking", seg: ts });
      }
    }
    if (end > cursor) {
      segments.push({ kind: "text", content: message.content.slice(cursor, end) });
    }
  };
  const pushThinkingBeforeTool = (toolIdx: number) => {
    const arr = thinkingByTool.get(toolIdx);
    if (arr) for (const ts of arr) segments.push({ kind: "thinking", seg: ts });
  };

  if (hasToolCalls && message.toolCallOffsets && message.toolCallOffsets.length > 0) {
    const offsets = message.toolCallOffsets;
    const allCalls = message.toolCalls!;
    let textCursor = 0;

    let toolBatch: ChatToolCall[] = [];
    let toolBatchIds: string[] = [];

    for (let i = 0; i < allCalls.length; i++) {
      const offset = Math.max(offsets[i] ?? message.content.length, textCursor);

      // Answer text (with any reasoning bursts spliced in at their content
      // offsets) that preceded this tool.
      if (offset > textCursor || thinkingByTool.has(i)) {
        flushToolBatch(toolBatch, toolBatchIds);
        toolBatch = [];
        toolBatchIds = [];
        emitTextWithThinking(textCursor, offset, i);
        textCursor = offset;
      }

      if (allCalls[i].name === "update_plan") {
        flushToolBatch(toolBatch, toolBatchIds);
        toolBatch = [];
        toolBatchIds = [];
        segments.push({ kind: "plan_marker" });
      } else if (isSubAgentCall(allCalls[i])) {
        flushToolBatch(toolBatch, toolBatchIds);
        toolBatch = [];
        toolBatchIds = [];
        segments.push({
          kind: "sub_agent",
          requestId: allCalls[i].requestId ?? allCalls[i].name,
          toolCall: allCalls[i],
        });
      } else if (isVisibleCall(allCalls[i])) {
        toolBatch.push(allCalls[i]);
        if (allCalls[i].requestId) toolBatchIds.push(allCalls[i].requestId!);
      }
    }

    flushToolBatch(toolBatch, toolBatchIds);
    emitTextWithThinking(textCursor, message.content.length, allCalls.length);
  } else if (hasToolCalls) {
    const tcOffset = message.toolCallsContentOffset ?? 0;
    // Pre-tool answer text with interleaved reasoning.
    emitTextWithThinking(0, tcOffset, 0);

    const allCalls = message.toolCalls!;
    let toolBatch: ChatToolCall[] = [];
    let toolBatchIds: string[] = [];

    for (let i = 0; i < allCalls.length; i++) {
      const tc = allCalls[i];
      // Reasoning that resumed after the previous tool (index 0 handled above).
      if (i > 0 && thinkingByTool.has(i)) {
        flushToolBatch(toolBatch, toolBatchIds);
        toolBatch = [];
        toolBatchIds = [];
        pushThinkingBeforeTool(i);
      }
      if (tc.name === "update_plan") {
        flushToolBatch(toolBatch, toolBatchIds);
        toolBatch = [];
        toolBatchIds = [];
        segments.push({ kind: "plan_marker" });
      } else if (isSubAgentCall(tc)) {
        flushToolBatch(toolBatch, toolBatchIds);
        toolBatch = [];
        toolBatchIds = [];
        segments.push({
          kind: "sub_agent",
          requestId: tc.requestId ?? tc.name,
          toolCall: tc,
        });
      } else if (isVisibleCall(tc)) {
        toolBatch.push(tc);
        if (tc.requestId) toolBatchIds.push(tc.requestId);
      }
    }
    flushToolBatch(toolBatch, toolBatchIds);
    // Post-tool answer text with interleaved reasoning.
    emitTextWithThinking(tcOffset, message.content.length, allCalls.length);
  } else {
    // No tool calls: split the answer text by reasoning offsets so multiple
    // "Thought for …" bursts land between the prose they preceded instead of all
    // stacking at the top of the message.
    emitTextWithThinking(0, message.content.length, 0);
    // Nothing has landed yet (no content, no reasoning): show a streaming
    // placeholder so the bubble isn't empty. Suppressed once a ThinkingBlock owns
    // the spinner to avoid a duplicate loader.
    if (segments.length === 0) {
      const showStreamingPlaceholder = message.isStreaming && !message.thinking;
      segments.push({ kind: "text", content: showStreamingPlaceholder ? "..." : "" });
    }
  }

  return segments;
}
