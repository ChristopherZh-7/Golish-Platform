import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  Loader2,
} from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/utils";
import { ItemRow } from "./ItemRow";
import { STEP_DESCRIPTIONS, type StepGroup } from "./types";
import { fmtDur, fmtTime, friendly } from "./utils";

/** Collapsible group row representing one pipeline step. Children are
 *  expanded by default while the step is running. */
export function StepRow({
  step,
  defaultOpen,
  dynamicDesc,
}: {
  step: StepGroup;
  defaultOpen: boolean;
  dynamicDesc?: string;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const { status, stepName, children, output, durationMs, startTs } = step;
  const desc = dynamicDesc || STEP_DESCRIPTIONS[stepName];
  const hasChildren = children.length > 0;
  const hasExpandable = hasChildren || (!!output && status === "completed");
  const isPending = status === "pending";

  return (
    <div className={cn(isPending && "opacity-40")}>
      <button
        type="button"
        onClick={() => hasExpandable && setOpen((v) => !v)}
        className={cn(
          "w-full flex items-start gap-2 py-2 px-3 text-left transition-colors",
          hasExpandable && "hover:bg-muted/10",
          status === "running" && "bg-blue-500/[0.04]"
        )}
      >
        <div className="mt-0.5 flex-shrink-0">
          {status === "pending" && (
            <div className="w-3.5 h-3.5 rounded-full border border-muted-foreground/15" />
          )}
          {status === "running" && <Loader2 className="w-3.5 h-3.5 text-blue-400 animate-spin" />}
          {status === "completed" && <CheckCircle2 className="w-3.5 h-3.5 text-green-400" />}
          {status === "failed" && <AlertTriangle className="w-3.5 h-3.5 text-red-400" />}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span
              className={cn(
                "text-xs font-medium truncate",
                status === "pending" && "text-muted-foreground/30",
                status === "running" && "text-blue-300",
                status === "completed" && "text-foreground/60",
                status === "failed" && "text-red-300"
              )}
            >
              {friendly(stepName)}
            </span>
            {hasChildren && (
              <span className="text-[9px] text-muted-foreground/25">
                {children.length} op{children.length !== 1 ? "s" : ""}
              </span>
            )}
            {durationMs != null && (
              <span className="text-[9px] text-muted-foreground/25 flex items-center gap-0.5">
                <Clock className="w-2 h-2" />
                {fmtDur(durationMs)}
              </span>
            )}
          </div>
          {status === "pending" && desc && (
            <p className="mt-0.5 text-[10px] text-muted-foreground/20">{desc}</p>
          )}
          {status === "running" && desc && (
            <p className="mt-0.5 text-[10px] text-muted-foreground/40">{desc}</p>
          )}
          {!open && status === "completed" && output && (
            <p className="mt-0.5 text-[10px] text-muted-foreground/30 font-mono truncate">
              {output.slice(0, 120)}
            </p>
          )}
        </div>
        <div className="flex items-center gap-1.5 mt-0.5 flex-shrink-0">
          {!isPending && (
            <span className="text-[9px] text-muted-foreground/15">{fmtTime(startTs)}</span>
          )}
          {hasExpandable &&
            (open ? (
              <ChevronDown className="w-3 h-3 text-muted-foreground/25" />
            ) : (
              <ChevronRight className="w-3 h-3 text-muted-foreground/25" />
            ))}
        </div>
      </button>
      {open && hasChildren && (
        <div className="border-l border-border/10 ml-5">
          {children.map((child) => (
            <ItemRow key={child.id} item={child} indent />
          ))}
        </div>
      )}
      {open && output && status === "completed" && (
        <pre className="ml-8 mr-3 mb-1 text-[10px] text-muted-foreground/30 font-mono leading-relaxed whitespace-pre-wrap break-all max-h-40 overflow-auto">
          {output.length > 800 ? `${output.slice(0, 800)}...` : output}
        </pre>
      )}
    </div>
  );
}
