import { memo } from "react";

import { cn } from "@/lib/utils";

/**
 * High-level phase the running agent turn is in. Each phase maps to a single,
 * truthful label — no fake vocabulary rotation. The phase is computed from
 * the live message state (tool calls in flight, thinking stream, content
 * stream) by the parent component, so what the user sees is what the agent
 * is actually doing.
 *
 * `writing` is intentionally *not* rendered here: the moment the model
 * starts streaming the answer text, the indicator vanishes — no exit
 * animation, no height-collapse, just gone. The answer text itself is the
 * "alive" signal from that point on.
 */
export type AgentStatusPhase =
  | "starting" // brand-new turn, no chunks yet
  | "thinking" // reasoning content streaming
  | "writing" // text content streaming → indicator hidden
  | "tool" // a tool call is in flight (detail = tool / command)
  | "delegating" // sub-agent dispatch (detail = sub-agent name)
  | "compacting" // context compaction in progress
  | "planning"; // update_plan tool call

interface AgentStatusIndicatorProps {
  phase: AgentStatusPhase;
  /**
   * Contextual detail substituted into the label (e.g. tool name, file
   * name, sub-agent name, command preview).
   */
  detail?: string;
  className?: string;
}

type VisiblePhase = Exclude<AgentStatusPhase, "writing">;

const LABELS: Record<VisiblePhase, (detail?: string) => string> = {
  starting: () => "Starting…",
  thinking: () => "Thinking…",
  planning: () => "Planning the task…",
  compacting: () => "Compacting context…",
  tool: (detail) => (detail ? `Running ${detail}…` : "Running tool…"),
  delegating: (detail) => (detail ? `Delegating to ${detail}…` : "Delegating…"),
};

/**
 * Windsurf-style status row for the streaming agent. Visually:
 *
 *   `  Running nmap -sV 10.0.0.1…`   (theme-tinted, soft shimmer sweep)
 *
 * - One truthful line per phase, no rotation, no vocabulary lottery.
 * - Soft horizontal shimmer (CSS `background-clip: text`) replaces the
 *   spinner — perceptible motion without flicker.
 * - On phase change the inner `<span>` is keyed on the label text, so the
 *   new text plays a one-shot fade-in while the shimmer keeps sweeping.
 * - On `phase === "writing"` we simply `return null`: the indicator
 *   disappears instantly with zero exit animation, no height collapse.
 */
export const AgentStatusIndicator = memo(function AgentStatusIndicator({
  phase,
  detail,
  className,
}: AgentStatusIndicatorProps) {
  if (phase === "writing") return null;

  const label = LABELS[phase as VisiblePhase](detail);

  return (
    <div
      className={cn(
        "agent-status-line flex items-center gap-1.5 mt-2 py-0.5 select-none",
        "font-mono text-[11.5px]",
        className
      )}
      aria-live="polite"
      aria-busy
    >
      <span key={label} className="agent-status-shimmer truncate">
        {label}
      </span>
    </div>
  );
});
