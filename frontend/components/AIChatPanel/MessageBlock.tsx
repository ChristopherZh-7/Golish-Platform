import React, { memo } from "react";
import { AlertCircle, Loader2 } from "lucide-react";
import { Markdown } from "@/components/Markdown";
import { cn } from "@/lib/utils";
import type { ChatMessage } from "@/store";
import type { ChatToolCall } from "@/store/slices/conversation";

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
  taskPlan?: TaskPlanViewModel | null;
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
        const hasContent = !!(message.content?.trim()) ||
          (message.toolCalls && message.toolCalls.length > 0);

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
                      <InlinePlanCard plan={taskPlan!} />
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
                <ToolCallSummary key={`seg-${idx}`} toolCalls={seg.calls} requestIds={seg.requestIds} isMessageComplete={messageComplete} />
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

      {message.isStreaming && (() => {
        const lastPendingTool = message.toolCalls
          ?.slice()
          .reverse()
          .find((tc) => tc.success === undefined);
        let statusLabel = "Thinking";
        if (lastPendingTool) {
          const name = lastPendingTool.name;
          if (name.startsWith("sub_agent_")) {
            const agentName = name.replace("sub_agent_", "");
            statusLabel = `Delegating to ${agentName}`;
          } else if (name === "update_plan") {
            statusLabel = "Planning";
          } else if (name === "run_pty_cmd" || name === "run_command") {
            try {
              const args = JSON.parse(lastPendingTool.args || "{}");
              const cmd = args.command as string | undefined;
              statusLabel = cmd ? `Running ${cmd.length > 40 ? `${cmd.slice(0, 40)}…` : cmd}` : "Running command";
            } catch { statusLabel = "Running command"; }
          } else if (name === "pentest_run") {
            try {
              const args = JSON.parse(lastPendingTool.args || "{}");
              statusLabel = args.tool_name ? `Running ${args.tool_name}` : "Running pentest tool";
            } catch { statusLabel = "Running pentest tool"; }
          } else if (name === "run_pipeline") {
            statusLabel = "Running pipeline";
          } else if (name === "read_file") {
            try {
              const args = JSON.parse(lastPendingTool.args || "{}");
              const path = (args.path as string) || "";
              const filename = path.split("/").pop() || path;
              statusLabel = `Reading ${filename}`;
            } catch { statusLabel = "Reading file"; }
          } else if (name === "write_file" || name === "create_file" || name === "edit_file") {
            statusLabel = "Writing file";
          } else if (name === "search_memories") {
            statusLabel = "Searching memories";
          } else if (name === "manage_targets") {
            statusLabel = "Managing targets";
          } else if (name === "web_search" || name.startsWith("tavily_")) {
            statusLabel = "Searching web";
          } else {
            statusLabel = `Running ${name.replace(/_/g, " ")}`;
          }
        } else if (message.thinking && !message.content) {
          statusLabel = "Reasoning";
        } else if (!message.content) {
          statusLabel = "Thinking";
        } else {
          statusLabel = "Writing";
        }

        return (
          <div className="flex items-center gap-2 mt-2 agent-loading-shimmer rounded py-1">
            <Loader2 className="w-3 h-3 animate-spin text-accent flex-shrink-0" />
            <span className="text-[11px] text-muted-foreground/70 truncate">{statusLabel}</span>
          </div>
        );
      })()}
    </div>
  );
});

