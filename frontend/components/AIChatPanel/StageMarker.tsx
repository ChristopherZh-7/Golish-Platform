import { CheckCircle2, ChevronDown, CircleDot, Flag, PauseCircle, RotateCcw } from "lucide-react";
import { useState } from "react";
import { Markdown } from "@/components/Markdown";
import { cn } from "@/lib/utils";
import type { ChatMessage } from "@/store";

function iconFor(kind?: string, status?: string) {
  if (status === "waiting_approval") return PauseCircle;
  if (status === "finished") return Flag;
  if (kind === "task_resumed") return RotateCcw;
  if (kind === "subtask_completed") return CheckCircle2;
  return CircleDot;
}

/**
 * Inline divider for a task-mode stage boundary (subtask completed, task
 * progress transition, task resumed). Keeps consecutive stage narrations from
 * reading as one continuous monologue and makes the runtime's stage advance
 * visible. Rendered by `AIChatPanel` for `role: "system"` messages.
 */
export function StageMarker({ message }: { message: ChatMessage }) {
  const ev = message.stageEvent;
  const label = ev?.label ?? message.content;
  const detail = ev?.detail;
  const isWaiting = ev?.status === "waiting_approval";
  const [expanded, setExpanded] = useState(false);
  const Icon = iconFor(ev?.kind, ev?.status);

  if (!label) return null;

  return (
    <div className="px-4 py-2">
      <div className="flex items-center gap-2">
        <div className="h-px flex-1 bg-[var(--border-subtle)]" />
        <div
          className={cn(
            "flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-medium max-w-[80%]",
            isWaiting
              ? "border-[#e0af68]/40 text-[#e0af68] bg-[#e0af68]/5"
              : "border-[var(--border-subtle)] text-muted-foreground/80 bg-background/60"
          )}
        >
          <Icon
            className={cn(
              "w-3 h-3 flex-shrink-0",
              isWaiting ? "text-[#e0af68]" : "text-[var(--ansi-green)]"
            )}
          />
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
        <div className="h-px flex-1 bg-[var(--border-subtle)]" />
      </div>

      {detail && expanded && (
        <div className="mt-1.5 mx-auto max-w-[92%] max-h-[240px] overflow-auto rounded-md border border-[var(--border-subtle)] bg-[var(--bg-hover)] px-3 py-2 text-[11.5px] text-foreground/75 leading-[1.6] [&_p]:mb-1.5 [&_p:last-child]:mb-0">
          <Markdown content={detail} />
        </div>
      )}
    </div>
  );
}
