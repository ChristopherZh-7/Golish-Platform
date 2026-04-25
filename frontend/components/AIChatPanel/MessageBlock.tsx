import React, { memo } from "react";
import { AlertCircle, Loader2 } from "lucide-react";
import { Markdown } from "@/components/Markdown";
import { cn } from "@/lib/utils";
import type { ChatMessage } from "@/store";
import type { ChatToolCall } from "@/store/slices/conversation";

import {
  CollapsibleToolCall,
  PlanUpdatedNotice,
  TaskPlanCard,
  type TaskPlanState,
  ThinkingBlock,
  ToolCallSummary,
  usePlanNestedRequestIds,
} from "./ChatSubComponents";

/**
 * Strip XML-formatted tool call tags that some models (e.g. Mistral)
 * emit as raw text instead of structured tool_calls.
 * Handles both complete and incomplete (streaming) XML fragments.
 */
function stripToolCallXml(text: string): string {
  let cleaned = text
    .replace(/<execute>[\s\S]*?<\/execute>/g, "")
    .replace(/<function=[^>]*>[\s\S]*?<\/function>/g, "")
    .replace(/<\/function>/g, "");

  const incompleteIdx = cleaned.search(/<(?:execute|function[=\s]|parameter[=\s])/);
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
  taskPlan?: TaskPlanState | null;
  planTextOffset?: number | null;
  terminalId?: string | null;
}) {
  const isUser = message.role === "user";
  const nestedIds = usePlanNestedRequestIds(taskPlan ? (terminalId ?? null) : null);

  return (
    <div className={cn("px-4 py-3", !isUser && "bg-[var(--bg-hover)]")}>
      <div className="text-[11px] text-muted-foreground mb-1.5 font-medium">
        {isUser ? "You" : "Golish AI"}
      </div>

      {!isUser && message.thinking && (
        <ThinkingBlock
          content={message.thinking}
          isActive={!!message.isStreaming && !message.content}
        />
      )}

      {(() => {
        if (message.error) {
          return (
            <div className="flex items-start gap-2 text-[13px] text-destructive">
              <AlertCircle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
              <span>{message.error}</span>
            </div>
          );
        }

        const hasToolCalls = !isUser && message.toolCalls && message.toolCalls.length > 0;

        const isSubAgentCall = (tc: ChatToolCall) => tc.name.startsWith("sub_agent_");

        const isVisibleCall = (tc: ChatToolCall) =>
          tc.name !== "update_plan" &&
          !isSubAgentCall(tc) &&
          !nestedIds.has(tc.requestId ?? "") &&
          (tc.success !== undefined ||
            (pendingApproval == null && tc.success === undefined));

        const pendingCalls = hasToolCalls && pendingApproval
          ? message.toolCalls!.filter(
              (tc) => tc.name === pendingApproval.toolName && tc.success === undefined
            )
          : [];

        const renderPendingApprovalCards = () =>
          pendingCalls.length > 0 ? (
            <div className="mt-2 space-y-1.5">
              {pendingCalls.map((tc, i) => (
                <CollapsibleToolCall
                  key={`${tc.name}-${i}`}
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

        // Build interleaved segments: text chunks, tool-call groups, and sub-agent cards in order.
        type Segment =
          | { kind: "text"; content: string }
          | { kind: "tools"; calls: ChatToolCall[]; requestIds: string[] }
          | { kind: "sub_agent"; requestId: string; toolCall: ChatToolCall }
          | { kind: "plan_marker" };

        const segments: Segment[] = [];

        const flushToolBatch = (
          toolBatch: ChatToolCall[],
          toolBatchIds: string[],
        ) => {
          if (toolBatch.length > 0) {
            segments.push({ kind: "tools", calls: [...toolBatch], requestIds: [...toolBatchIds] });
          }
        };

        if (hasToolCalls && message.toolCallOffsets && message.toolCallOffsets.length > 0) {
          const offsets = message.toolCallOffsets;
          const allCalls = message.toolCalls!;
          let textCursor = 0;

          let toolBatch: ChatToolCall[] = [];
          let toolBatchIds: string[] = [];

          for (let i = 0; i < allCalls.length; i++) {
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
          if (textCursor < message.content.length) {
            segments.push({ kind: "text", content: message.content.slice(textCursor) });
          }
        } else if (hasToolCalls) {
          const tcOffset = message.toolCallsContentOffset ?? 0;
          if (tcOffset > 0) {
            segments.push({ kind: "text", content: message.content.slice(0, tcOffset) });
          }

          const allCalls = message.toolCalls!;
          let toolBatch: ChatToolCall[] = [];
          let toolBatchIds: string[] = [];

          for (const tc of allCalls) {
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

          if (tcOffset < message.content.length) {
            segments.push({ kind: "text", content: message.content.slice(tcOffset) });
          }
        } else {
          segments.push({ kind: "text", content: message.content || (message.isStreaming ? "..." : "") });
        }

        // Determine where to insert the task plan card
        let planInserted = false;
        const shouldShowPlan = !isUser && taskPlan;

        return (
          <div className="flex flex-col gap-2">
            {segments.map((seg, idx) => {
              if (seg.kind === "text") {
                const displayContent = stripToolCallXml(seg.content);
                const text = displayContent.trim();
                if (!text && segments.length > 1) return null;

                const showPlanBefore =
                  shouldShowPlan &&
                  !planInserted &&
                  planTextOffset != null &&
                  planTextOffset > 0 &&
                  idx === 0;

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
                      <TaskPlanCard plan={taskPlan!} terminalId={terminalId} />
                      {after.trim() && (
                        <div className="text-[12px] text-foreground leading-[1.55]">
                          <Markdown content={after} />
                        </div>
                      )}
                    </React.Fragment>
                  );
                }

                return (
                  <div key={`seg-${idx}`} className="text-[12px] text-foreground leading-[1.55]">
                    <Markdown content={displayContent || (message.isStreaming ? "..." : "")} />
                  </div>
                );
              }

              if (seg.kind === "plan_marker") {
                if (!planInserted && shouldShowPlan) {
                  planInserted = true;
                  return <TaskPlanCard key={`seg-${idx}`} plan={taskPlan!} terminalId={terminalId} />;
                }
                return null;
              }

              if (seg.kind === "sub_agent") {
                return null;
              }

              // Tool segment
              const messageComplete = !message.isStreaming;
              return (
                <ToolCallSummary key={`seg-${idx}`} toolCalls={seg.calls} requestIds={seg.requestIds} isMessageComplete={messageComplete} />
              );
            })}
            {shouldShowPlan && !planInserted && <TaskPlanCard plan={taskPlan!} terminalId={terminalId} />}
            {!taskPlan && !isUser && message.toolCalls?.some((tc) => tc.name === "update_plan") && <PlanUpdatedNotice />}
            {pendingCalls.length > 0 && renderPendingApprovalCards()}
          </div>
        );
      })()}

      {message.isStreaming && (
        <div className="flex items-center gap-1 mt-1">
          <Loader2 className="w-3 h-3 animate-spin text-accent" />
        </div>
      )}
    </div>
  );
});

