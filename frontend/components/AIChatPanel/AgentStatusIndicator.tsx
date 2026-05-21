import { memo, useEffect, useMemo, useRef, useState } from "react";

import { cn } from "@/lib/utils";

/**
 * High-level phase the running agent turn is in. The component picks a
 * vocabulary that fits the phase and rotates through it while the phase
 * stays unchanged, so the user always sees motion + variety without us
 * relying on a generic spinner.
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
  /**
   * Override the rotation interval (ms). Defaults to 2200 — fast enough to
   * feel alive, slow enough to read each line.
   */
  rotationMs?: number;
  className?: string;
}

const VOCABULARY: Record<AgentStatusPhase, string[]> = {
  starting: [
    "warming the rig",
    "loading the toolkit",
    "calibrating sensors",
    "scoping the room",
    "tuning the antenna",
  ],
  thinking: [
    "tracing the lead",
    "triangulating context",
    "chasing the chain",
    "running the playbook",
    "cross-checking signals",
    "decoding the intent",
    "weighing the angles",
  ],
  writing: [
    "drafting the report",
    "compiling intel",
    "patching the brief",
    "composing answer",
    "laying down rounds",
  ],
  tool: ["running {detail}", "executing {detail}", "deploying {detail}", "spawning {detail}"],
  delegating: [
    "relaying to {detail}",
    "dispatching to {detail}",
    "tasking {detail}",
    "handing off to {detail}",
  ],
  compacting: [
    "condensing memory",
    "compressing scrolls",
    "trimming the journal",
    "summarising the run",
  ],
  planning: [
    "drafting the plan",
    "pinning the route",
    "mapping the surface",
    "staking the milestones",
  ],
};

function fill(template: string, detail: string | undefined): string {
  if (!detail) return template.replace(/\s*{detail}\s*/g, "").trim();
  return template.replace("{detail}", detail).trim();
}

/**
 * Cursor-style status line for the streaming agent. Visually:
 *   `> tracing the lead_`
 *
 * - Monospace + emerald hue (Golish brand cyber-recon vibe)
 * - Trailing block cursor blinks via the `caret-blink` keyframe defined
 *   alongside the component
 * - Phrase rotates every `rotationMs` while the phase is unchanged, so the
 *   user always perceives forward motion without a spinner
 */
export const AgentStatusIndicator = memo(function AgentStatusIndicator({
  phase,
  detail,
  rotationMs = 2200,
  className,
}: AgentStatusIndicatorProps) {
  const phrases = useMemo(
    () => VOCABULARY[phase].map((tpl) => fill(tpl, detail)).filter((p) => p.length > 0),
    [phase, detail]
  );

  const [index, setIndex] = useState(0);
  const phaseRef = useRef(phase);
  useEffect(() => {
    if (phaseRef.current !== phase) {
      phaseRef.current = phase;
      setIndex(0);
    }
  }, [phase]);

  useEffect(() => {
    if (phrases.length <= 1) return undefined;
    const id = setInterval(() => {
      setIndex((i) => (i + 1) % phrases.length);
    }, rotationMs);
    return () => clearInterval(id);
  }, [phrases, rotationMs]);

  const text = phrases[index] ?? phrases[0] ?? "working";

  return (
    <div
      className={cn(
        "agent-status-line flex items-center gap-1.5 mt-2 py-0.5 select-none",
        "font-mono text-[11.5px] text-emerald-400/70",
        className
      )}
      aria-live="polite"
      aria-busy
    >
      <span className="text-emerald-400/60">&gt;</span>
      <span className="agent-status-text truncate">{text}</span>
      <span className="agent-status-caret inline-block w-1.5 h-3 bg-emerald-400/70 align-middle" />
    </div>
  );
});
