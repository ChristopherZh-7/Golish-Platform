import { memo, useMemo } from "react";

import { cn } from "@/lib/utils";

/**
 * High-level phase the running agent turn is in.
 */
export type AgentStatusPhase =
  | "starting" // brand-new turn, no chunks yet
  | "thinking" // reasoning content streaming
  | "writing" // text content streaming
  | "tool" // a tool call is in flight (detail = tool name)
  | "delegating" // sub-agent dispatch (detail = agent name)
  | "compacting" // context compaction in progress
  | "planning"; // update_plan tool call

interface AgentStatusIndicatorProps {
  phase: AgentStatusPhase;
  /**
   * Optional contextual detail substituted into vocabulary templates
   * (e.g. tool name, file name, sub-agent name).
   */
  detail?: string;
  className?: string;
}

const PHASE_LABELS: Record<AgentStatusPhase, string> = {
  starting: "Preparing context",
  thinking: "Planning next step",
  writing: "Writing response",
  tool: "Running tool",
  delegating: "Delegating task",
  compacting: "Compacting context",
  planning: "Planning next step",
};

function formatDetail(detail: string | undefined): string | null {
  if (!detail) return null;
  const cleaned = detail.replace(/\s+/g, " ").trim();
  if (!cleaned) return null;
  return cleaned.length > 48 ? `${cleaned.slice(0, 48)}…` : cleaned;
}

function statusText(phase: AgentStatusPhase, detail: string | undefined): string {
  const formattedDetail = formatDetail(detail);
  if (phase === "tool" && formattedDetail) return `Running ${formattedDetail}`;
  if (phase === "delegating" && formattedDetail) return `Delegating to ${formattedDetail}`;
  return PHASE_LABELS[phase];
}

/**
 * Compact phase indicator for the streaming agent turn.
 *
 * It intentionally reads like product state, not terminal output: stable
 * enough to reassure the user, specific enough to explain what is happening.
 */
export const AgentStatusIndicator = memo(function AgentStatusIndicator({
  phase,
  detail,
  className,
}: AgentStatusIndicatorProps) {
  const text = useMemo(() => statusText(phase, detail), [phase, detail]);

  return (
    <div
      className={cn(
        "agent-status-line mt-2 inline-flex max-w-full items-center gap-2 rounded-md",
        "border border-[var(--border-subtle)] bg-background/55 px-2.5 py-1",
        "text-[11.5px] text-muted-foreground select-none",
        className
      )}
      aria-live="polite"
      aria-busy
    >
      <span className="relative flex h-2 w-2 flex-shrink-0" aria-hidden="true">
        <span className="agent-status-dot absolute inline-flex h-full w-full rounded-full bg-accent/55" />
        <span className="relative inline-flex h-2 w-2 rounded-full bg-accent/80" />
      </span>
      <span className="truncate text-foreground/75">{text}</span>
    </div>
  );
});
