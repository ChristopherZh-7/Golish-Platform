import { Check, ChevronDown, CircleDot, Flag, PauseCircle, RotateCcw } from "lucide-react";
import { useState } from "react";
import { Markdown } from "@/components/Markdown";
import { cn } from "@/lib/utils";
import type { ChatMessage } from "@/store";

/** Human-friendly labels for the known harness stage ids. */
const STAGE_LABELS: Record<string, string> = {
  scoping: "Scoping",
  target_intel: "Target Intel",
  external_attack_surface: "External Attack Surface",
  enumeration: "Enumeration",
  vuln_triage: "Vulnerability Triage",
  verification: "Verification",
  objective_pathing: "Objective Pathing",
  exploitation: "Exploitation",
  post_exploitation: "Post-Exploitation",
  lateral_movement: "Lateral Movement",
  reporting: "Reporting",
  cleanup: "Cleanup",
};

/**
 * Turn a raw harness stage id (`external_attack_surface`) into a readable label
 * (`External Attack Surface`). Falls back to Title-casing unknown ids.
 */
export function prettyStageName(id: string): string {
  const known = STAGE_LABELS[id];
  if (known) return known;
  return id
    .split(/[_\s]+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

function iconFor(kind?: string, status?: string) {
  if (status === "waiting_approval") return PauseCircle;
  if (kind === "stage_completed" || status === "finished") return Flag;
  if (kind === "task_resumed") return RotateCcw;
  if (kind === "subtask_completed") return Check;
  return CircleDot;
}

/**
 * Inline divider for a task-mode stage boundary. Two completion granularities
 * are rendered with deliberately different prominence so the user can tell a
 * single *step* finishing apart from a whole *stage* passing its gate:
 *
 *  - `stage_completed` (a stage's `submit_stage_deliverable` was accepted) →
 *    a prominent green "Stage complete" milestone with a flag.
 *  - `subtask_completed` (one step inside a stage) → a subdued, smaller marker.
 *
 * Rendered by `AIChatPanel` for `role: "system"` messages.
 */
export function StageMarker({ message }: { message: ChatMessage }) {
  const ev = message.stageEvent;
  const label = ev?.label ?? message.content;
  const detail = ev?.detail;
  const isWaiting = ev?.status === "waiting_approval";
  const isStage = ev?.kind === "stage_completed";
  const isStep = ev?.kind === "subtask_completed";
  const [expanded, setExpanded] = useState(false);
  const Icon = iconFor(ev?.kind, ev?.status);

  if (!label) return null;

  // Prominence: stage milestones stand out (bold green flag); step completions
  // recede (muted, smaller); waiting-for-approval is amber; everything else
  // keeps the neutral default.
  const pillClass = isWaiting
    ? "border-[#e0af68]/40 text-[#e0af68] bg-[#e0af68]/5 px-2.5 py-1 text-[11px] font-medium"
    : isStage
      ? "border-[var(--ansi-green)]/50 text-[var(--ansi-green)] bg-[var(--ansi-green)]/10 px-2.5 py-1 text-[11px] font-semibold"
      : isStep
        ? "border-[var(--border-subtle)]/60 text-muted-foreground/55 bg-transparent px-2 py-0.5 text-[10.5px] font-medium"
        : "border-[var(--border-subtle)] text-muted-foreground/80 bg-background/60 px-2.5 py-1 text-[11px] font-medium";

  const iconClass = isWaiting
    ? "text-[#e0af68] w-3 h-3"
    : isStage
      ? "text-[var(--ansi-green)] w-3.5 h-3.5"
      : isStep
        ? "text-muted-foreground/45 w-2.5 h-2.5"
        : "text-[var(--ansi-green)] w-3 h-3";

  // Step dividers are quieter than stage/other dividers so a stage milestone
  // reads as the stronger boundary.
  const lineClass = cn(
    "h-px flex-1",
    isStep ? "bg-[var(--border-subtle)]/40" : "bg-[var(--border-subtle)]"
  );

  return (
    <div className={cn(isStep ? "px-4 py-1" : "px-4 py-2")}>
      <div className="flex items-center gap-2">
        <div className={lineClass} />
        <div className={cn("flex items-center gap-1.5 rounded-full border max-w-[80%]", pillClass)}>
          <Icon className={cn("flex-shrink-0", iconClass)} />
          <span className="truncate">{label}</span>
          {detail && (
            <button
              type="button"
              onClick={() => setExpanded((v) => !v)}
              className="ml-0.5 inline-flex items-center text-muted-foreground/50 hover:text-muted-foreground transition-colors"
              aria-expanded={expanded}
              aria-label={expanded ? "Hide details" : "Show details"}
            >
              <ChevronDown
                className={cn("w-3 h-3 transition-transform", !expanded && "-rotate-90")}
              />
            </button>
          )}
        </div>
        <div className={lineClass} />
      </div>

      {detail && expanded && (
        <div className="mt-1.5 mx-auto max-w-[92%] max-h-[240px] overflow-auto rounded-md border border-[var(--border-subtle)] bg-[var(--bg-hover)] px-3 py-2 text-[11.5px] text-foreground/75 leading-[1.6] [&_p]:mb-1.5 [&_p:last-child]:mb-0">
          <Markdown content={detail} />
        </div>
      )}
    </div>
  );
}
