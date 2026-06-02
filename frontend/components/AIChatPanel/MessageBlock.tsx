import { AlertCircle, Loader2 } from "lucide-react";
import React, { memo } from "react";
import { Markdown } from "@/components/Markdown";
import { cn } from "@/lib/utils";
import type { ChatMessage } from "@/store";
import type { ChatToolCall, ThinkingSegment } from "@/store/slices/conversation";

import { AgentStatusIndicator, type AgentStatusPhase } from "./AgentStatusIndicator";
import {
  CollapsibleToolCall,
  type TaskPlanViewModel,
  ThinkingBlock,
  ToolCallSummary,
  usePlanNestedRequestIds,
} from "./ChatSubComponents";
import { InlinePlanCard } from "./InlinePlanCard";
import { SubAgentInlineCard } from "./SubAgentInlineCard";

/**
 * Strip XML-formatted tool call tags that some models (e.g. Mistral)
 * emit as raw text instead of structured tool_calls.
 * Handles both complete and incomplete (streaming) XML fragments.
 */
function stripToolCallXml(text: string): string {
  let cleaned = text
    .replace(/<tool_call\b[^>]*>[\s\S]*?<\/tool_call>/g, "")
    .replace(/<execute>[\s\S]*?<\/execute>/g, "")
    .replace(/<function=[^>]*>[\s\S]*?<\/function>/g, "")
    .replace(/<\/?tool_call\b[^>]*>/g, "")
    .replace(/<\/function>/g, "");

  const incompleteIdx = cleaned.search(/<(?:tool_call\b|execute|function[=\s]|parameter[=\s])/);
  if (incompleteIdx !== -1) {
    cleaned = cleaned.slice(0, incompleteIdx);
  }

  return cleaned;
}

export const MessageBlock = memo(function MessageBlock({
  message,
  pendingApproval,
  onApprove,
  onDeny,
  approvalMode,
  onApprovalModeChange,
  taskPlan,
  planTextOffset,
  terminalId,
}: {
  message: ChatMessage;
  pendingApproval?: { requestId: string; toolName: string } | null;
  onApprove?: (requestId: string) => void;
  onDeny?: (requestId: string) => void;
  approvalMode?: string;
  onApprovalModeChange?: (mode: "ask" | "allowlist" | "run-all") => void;
  taskPlan?: TaskPlanViewModel | null;
  planTextOffset?: number | null;
  terminalId?: string | null;
}) {
  const isUser = message.role === "user";
  const nestedIds = usePlanNestedRequestIds(taskPlan ? (terminalId ?? null) : null);

  const thinkingSegments = message.thinkingSegments;
  const hasThinkingSegments = !!thinkingSegments && thinkingSegments.length > 0;

  // A segment is "active" (owns the live spinner) while streaming and nothing
  // newer (answer text or a tool call) has landed since it started.
  const thinkingActive = (ts: ThinkingSegment) =>
    !!message.isStreaming &&
    ts.contentOffset === (message.content?.length ?? 0) &&
    ts.toolIndex === (message.toolCalls?.length ?? 0);

  return (
    <div className={cn("px-4 py-3", !isUser && "bg-[var(--bg-hover)]")}>
      <div className="text-[11px] text-muted-foreground mb-1.5 font-medium">
        {isUser ? "You" : "Golish AI"}
      </div>

      {/* Fallback for restored history (no per-segment data): one top block. */}
      {!isUser && message.thinking && !hasThinkingSegments && (
        <ThinkingBlock
          content={message.thinking}
          isActive={!!message.isStreaming && !message.content && !(message.toolCalls?.length ?? 0)}
          startedAt={message.thinkingStartedAt}
          endedAt={message.thinkingEndedAt}
        />
      )}

      {(() => {
        const hasContent =
          !!message.content?.trim() || (message.toolCalls && message.toolCalls.length > 0);

        if (message.error && !hasContent) {
          return (
            <div className="flex items-start gap-2 text-[13px] text-destructive">
              <AlertCircle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
              <span>{message.error}</span>
            </div>
          );
        }

        const hasToolCalls = !isUser && message.toolCalls && message.toolCalls.length > 0;

        const isSubAgentCall = (tc: ChatToolCall) => tc.name.startsWith("sub_agent_");

        const isPendingApprovalCall = (tc: ChatToolCall) =>
          pendingApproval != null &&
          (tc.requestId
            ? tc.requestId === pendingApproval.requestId
            : tc.name === pendingApproval.toolName);

        const isVisibleCall = (tc: ChatToolCall) =>
          tc.name !== "update_plan" &&
          !isSubAgentCall(tc) &&
          !nestedIds.has(tc.requestId ?? "") &&
          (tc.success !== undefined || !isPendingApprovalCall(tc));

        const pendingCalls =
          (hasToolCalls && pendingApproval
            ? message.toolCalls?.filter(
                (tc) => isPendingApprovalCall(tc) && tc.success === undefined
              )
            : []) ?? [];

        const renderPendingApprovalCards = () =>
          pendingCalls.length > 0 ? (
            <div className="mt-2 space-y-1.5">
              {pendingCalls.map((tc, i) => (
                <CollapsibleToolCall
                  key={tc.requestId ?? `${tc.name}-${i}`}
                  tc={tc}
                  approval={pendingApproval}
                  onApprove={onApprove}
                  onDeny={onDeny}
                  approvalMode={approvalMode}
                  onApprovalModeChange={onApprovalModeChange}
                />
              ))}
            </div>
          ) : null;

        // Build interleaved segments: text chunks, tool-call groups, sub-agent
        // cards, and reasoning bursts in time order.
        type Segment =
          | { kind: "text"; content: string }
          | { kind: "tools"; calls: ChatToolCall[]; requestIds: string[] }
          | { kind: "sub_agent"; requestId: string; toolCall: ChatToolCall }
          | { kind: "plan_marker" }
          | { kind: "thinking"; seg: ThinkingSegment };

        const segments: Segment[] = [];

        const flushToolBatch = (toolBatch: ChatToolCall[], toolBatchIds: string[]) => {
          if (toolBatch.length > 0) {
            segments.push({ kind: "tools", calls: [...toolBatch], requestIds: [...toolBatchIds] });
          }
        };

        // Group reasoning bursts by the tool index they preceded so they can be
        // spliced into the timeline right before that tool batch / answer text.
        const thinkingByTool = new Map<number, ThinkingSegment[]>();
        if (hasThinkingSegments) {
          for (const ts of thinkingSegments) {
            const arr = thinkingByTool.get(ts.toolIndex);
            if (arr) arr.push(ts);
            else thinkingByTool.set(ts.toolIndex, [ts]);
          }
        }
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
            // Reasoning that resumed after the previous tool, before this text/tool.
            if (thinkingByTool.has(i)) {
              flushToolBatch(toolBatch, toolBatchIds);
              toolBatch = [];
              toolBatchIds = [];
              pushThinkingBeforeTool(i);
            }

            const offset = offsets[i] ?? message.content.length;

            if (offset > textCursor) {
              flushToolBatch(toolBatch, toolBatchIds);
              toolBatch = [];
              toolBatchIds = [];
              segments.push({ kind: "text", content: message.content.slice(textCursor, offset) });
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
          pushThinkingBeforeTool(allCalls.length);
          if (textCursor < message.content.length) {
            segments.push({ kind: "text", content: message.content.slice(textCursor) });
          }
        } else if (hasToolCalls) {
          const tcOffset = message.toolCallsContentOffset ?? 0;
          pushThinkingBeforeTool(0);
          if (tcOffset > 0) {
            segments.push({ kind: "text", content: message.content.slice(0, tcOffset) });
          }

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
          pushThinkingBeforeTool(allCalls.length);

          if (tcOffset < message.content.length) {
            segments.push({ kind: "text", content: message.content.slice(tcOffset) });
          }
        } else {
          pushThinkingBeforeTool(0);
          // Suppress the "..." placeholder while ThinkingBlock owns the
          // streaming spinner — otherwise the user sees a second loader.
          const showStreamingPlaceholder = message.isStreaming && !message.thinking;
          segments.push({
            kind: "text",
            content: message.content || (showStreamingPlaceholder ? "..." : ""),
          });
        }

        // Determine where to insert the task plan card
        let planInserted = false;
        const shouldShowPlan = !isUser && taskPlan;
        // First text segment may not be index 0 once thinking bursts are spliced in.
        const firstTextIdx = segments.findIndex((s) => s.kind === "text");

        return (
          <div className="flex flex-col gap-2">
            {segments.map((seg, idx) => {
              if (seg.kind === "thinking") {
                return (
                  <ThinkingBlock
                    key={`seg-${idx}`}
                    content={seg.seg.content}
                    isActive={thinkingActive(seg.seg)}
                    startedAt={seg.seg.startedAt}
                    endedAt={seg.seg.endedAt}
                  />
                );
              }

              if (seg.kind === "text") {
                const displayContent = stripToolCallXml(seg.content);
                const text = displayContent.trim();
                if (!text && segments.length > 1) return null;

                const showPlanBefore =
                  shouldShowPlan &&
                  !planInserted &&
                  planTextOffset != null &&
                  planTextOffset > 0 &&
                  idx === firstTextIdx;

                if (showPlanBefore) {
                  const before = stripToolCallXml(seg.content.slice(0, planTextOffset));
                  const after = stripToolCallXml(seg.content.slice(planTextOffset));
                  planInserted = true;
                  return (
                    <React.Fragment key={`seg-${idx}`}>
                      {before.trim() && (
                        <div className="text-[12px] text-foreground leading-[1.55]">
                          <Markdown content={before} />
                        </div>
                      )}
                      <InlinePlanCard plan={taskPlan!} />
                      {after.trim() && (
                        <div className="text-[12px] text-foreground leading-[1.55]">
                          <Markdown content={after} />
                        </div>
                      )}
                    </React.Fragment>
                  );
                }

                // Same suppression rule as above: don't print the
                // "..." placeholder while ThinkingBlock is the active
                // streaming indicator.
                const placeholder = message.isStreaming && !message.thinking ? "..." : "";
                return (
                  <div key={`seg-${idx}`} className="text-[12px] text-foreground leading-[1.55]">
                    <Markdown content={displayContent || placeholder} />
                  </div>
                );
              }

              if (seg.kind === "plan_marker") {
                if (!planInserted && shouldShowPlan) {
                  planInserted = true;
                  return <InlinePlanCard key={`seg-${idx}`} plan={taskPlan!} />;
                }
                return null;
              }

              if (seg.kind === "sub_agent") {
                return (
                  <SubAgentInlineCard
                    key={`seg-${idx}`}
                    requestId={seg.requestId}
                    toolCall={seg.toolCall}
                    sessionId={terminalId}
                  />
                );
              }

              // Tool segment
              const messageComplete = !message.isStreaming;
              return (
                <ToolCallSummary
                  key={`seg-${idx}`}
                  toolCalls={seg.calls}
                  requestIds={seg.requestIds}
                  isMessageComplete={messageComplete}
                />
              );
            })}
            {shouldShowPlan && !planInserted && <InlinePlanCard plan={taskPlan!} />}
            {!taskPlan && !isUser && message.toolCalls?.some((tc) => tc.name === "update_plan") && (
              <div className="mx-0 my-1.5 flex items-center gap-2 px-3 py-1.5 rounded-lg border border-[var(--border-subtle)] bg-background/60 text-[11.5px] text-muted-foreground/50">
                <Loader2 className="w-3 h-3 animate-spin text-accent flex-shrink-0" />
                <span>Planning…</span>
              </div>
            )}
            {pendingCalls.length > 0 && renderPendingApprovalCards()}
            {message.error && (
              <div className="flex items-start gap-2 text-[13px] text-destructive mt-2">
                <AlertCircle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
                <span>{message.error}</span>
              </div>
            )}
          </div>
        );
      })()}

      {message.isStreaming &&
        (() => {
          const lastPendingTool = message.toolCalls
            ?.slice()
            .reverse()
            .find((tc) => tc.success === undefined);

          // While a ThinkingBlock owns the active spinner — either the
          // reasoning-only state or a reasoning burst that resumed after
          // content/tools — suppress the footer to avoid two spinners.
          const reasoningOwnsSpinner =
            (!lastPendingTool && message.thinking && !message.content) ||
            (hasThinkingSegments && thinkingSegments.some(thinkingActive));
          if (reasoningOwnsSpinner) {
            return null;
          }

          let phase: AgentStatusPhase;
          let detail: string | undefined;

          if (lastPendingTool) {
            const name = lastPendingTool.name;
            if (name.startsWith("sub_agent_")) {
              phase = "delegating";
              detail = name.replace("sub_agent_", "");
            } else if (name === "update_plan") {
              phase = "planning";
            } else if (name === "run_pty_cmd" || name === "run_command") {
              phase = "tool";
              try {
                const args = JSON.parse(lastPendingTool.args || "{}");
                const cmd = args.command as string | undefined;
                detail = cmd ? (cmd.length > 40 ? `${cmd.slice(0, 40)}…` : cmd) : "command";
              } catch {
                detail = "command";
              }
            } else if (name === "pentest_run") {
              phase = "tool";
              try {
                const args = JSON.parse(lastPendingTool.args || "{}");
                detail = (args.tool_name as string) || "pentest tool";
              } catch {
                detail = "pentest tool";
              }
            } else if (name === "read_file") {
              phase = "tool";
              try {
                const args = JSON.parse(lastPendingTool.args || "{}");
                const path = (args.path as string) || "file";
                detail = path.split("/").pop() || path;
              } catch {
                detail = "file";
              }
            } else {
              phase = "tool";
              detail = name.replace(/_/g, " ");
            }
          } else if (message.content) {
            phase = "writing";
          } else {
            phase = "starting";
          }

          return <AgentStatusIndicator phase={phase} detail={detail} />;
        })()}
    </div>
  );
});
