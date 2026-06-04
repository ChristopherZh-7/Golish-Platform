import { AlertCircle, AlertTriangle } from "lucide-react";
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
import { buildMessageSegments } from "./messageSegments";
import { StagePlanStack } from "./StagePlanStack";
import { SubAgentInlineCard } from "./SubAgentInlineCard";
import type { StagePlansViewModel } from "./TaskPlan";

/**
 * Strip XML-formatted tool call tags that some models (e.g. Mistral)
 * emit as raw text instead of structured tool_calls.
 * Handles both complete and incomplete (streaming) XML fragments.
 */
/**
 * One error/warning line under a message. `warning` reads as amber + triangle
 * (soft/recoverable, e.g. the planner asking for a task), matching the design
 * system's "interrupted" treatment; everything else stays red + alert circle.
 */
function MessageErrorLine({
  error,
  severity,
  className,
}: {
  error: string;
  severity?: "error" | "warning";
  className?: string;
}) {
  const isWarning = severity === "warning";
  const Icon = isWarning ? AlertTriangle : AlertCircle;
  return (
    <div
      className={cn(
        "flex items-start gap-2 text-[13px]",
        isWarning ? "text-amber-400" : "text-destructive",
        className
      )}
    >
      <Icon className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
      <span>{error}</span>
    </div>
  );
}

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
  stagePlans,
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
  stagePlans?: StagePlansViewModel | null;
  planTextOffset?: number | null;
  terminalId?: string | null;
}) {
  const isUser = message.role === "user";
  const hasStagePlans = !!stagePlans && stagePlans.stageOrder.length > 0;
  const nestedIds = usePlanNestedRequestIds(
    taskPlan || hasStagePlans ? (terminalId ?? null) : null
  );

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
          return <MessageErrorLine error={message.error} severity={message.errorSeverity} />;
        }

        const hasToolCalls = !isUser && message.toolCalls && message.toolCalls.length > 0;

        const isPendingApprovalCall = (tc: ChatToolCall) =>
          pendingApproval != null &&
          (tc.requestId
            ? tc.requestId === pendingApproval.requestId
            : tc.name === pendingApproval.toolName);

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

        // Build interleaved segments (text chunks, tool-call groups, sub-agent
        // cards, plan anchor, and reasoning bursts) in true time order. See
        // `buildMessageSegments` for the interleaving rules.
        const segments = buildMessageSegments(message, nestedIds, pendingApproval);

        // Determine where to insert the task plan card. Event-driven plans
        // (PlanUpdated, no `update_plan` tool call) have no plan_marker and often
        // no usable text offset; anchoring such a card at the TOP of the block
        // keeps streaming text / tool cards flowing BELOW it instead of pushing
        // it to the bottom as the message grows.
        let planInserted = false;
        // An empty (lazy) plan with zero steps carries no useful "N / M tasks
        // done" info and used to render a broken "Infinity more" card, so only
        // surface the plan card once it actually has steps.
        const shouldShowPlan =
          !isUser && (hasStagePlans || (!!taskPlan && taskPlan.steps.length > 0));
        // Per-stage stack (task mode) vs single card (chat mode). The stack
        // always anchors at the top of the plan-target block; the inline
        // offset / plan-marker placement only applies to the single card.
        const planNode = hasStagePlans ? (
          <StagePlanStack stagePlans={stagePlans!} />
        ) : (
          <InlinePlanCard plan={taskPlan!} />
        );
        const hasPlanMarker = segments.some((s) => s.kind === "plan_marker");
        // First text segment may not be index 0 once thinking bursts are spliced in.
        const firstTextIdx = segments.findIndex((s) => s.kind === "text");
        const willInsertInline =
          !hasStagePlans &&
          !!shouldShowPlan &&
          ((planTextOffset != null && planTextOffset > 0 && firstTextIdx !== -1) || hasPlanMarker);
        const showPlanAtTop = !!shouldShowPlan && !willInsertInline;
        if (showPlanAtTop) planInserted = true;

        return (
          <div className="flex flex-col gap-2">
            {showPlanAtTop && planNode}
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
                // Empty text renders nothing. While streaming, the
                // AgentStatusIndicator footer is the single "working" indicator
                // — no "..." placeholder that would double up with it.
                if (!text) return null;

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
                      {planNode}
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
                    <Markdown content={displayContent} />
                  </div>
                );
              }

              if (seg.kind === "plan_marker") {
                if (!planInserted && shouldShowPlan) {
                  planInserted = true;
                  return <React.Fragment key={`seg-${idx}`}>{planNode}</React.Fragment>;
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
            {shouldShowPlan && !planInserted && planNode}
            {pendingCalls.length > 0 && renderPendingApprovalCards()}
            {message.error && (
              <MessageErrorLine
                error={message.error}
                severity={message.errorSeverity}
                className="mt-2"
              />
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

          // A delegated sub-agent renders its own inline card with a live status
          // (spinner + current-activity line), so this footer would just be a
          // second, less specific "working" indicator for the same wait. Let the
          // card own that state instead of showing "Delegating to X" for the
          // whole sub-agent run.
          if (lastPendingTool?.name.startsWith("sub_agent_")) {
            return null;
          }

          let phase: AgentStatusPhase;
          let detail: string | undefined;

          if (lastPendingTool) {
            const name = lastPendingTool.name;
            if (name === "update_plan") {
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
