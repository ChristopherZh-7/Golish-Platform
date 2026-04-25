import {
  Bot,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Loader2,
  XCircle,
} from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { formatDurationLong } from "@/lib/time";
import { cn } from "@/lib/utils";

export interface TaskGroupShellProps {
  title: string;
  /** Slot rendered after the title (e.g. target badge) */
  titleExtra?: ReactNode;
  running: number;
  completed: number;
  failed: number;
  total: number;
  totalDurationMs: number;
  /** True when the progress bar should appear with the "failed" colour */
  hasFailure?: boolean;
  children: ReactNode;
}

export const TaskGroupShell = memo(function TaskGroupShell({
  title,
  titleExtra,
  running,
  completed,
  failed,
  total,
  totalDurationMs,
  hasFailure,
  children,
}: TaskGroupShellProps) {
  const [isCollapsed, setIsCollapsed] = useState(false);
  const wasRunningRef = useRef(false);

  const doneCount = completed + failed;
  const allDone = running === 0 && total > 0 && doneCount === total;
  const progress = total > 0 ? (doneCount / total) * 100 : 0;

  const toggleCollapse = useCallback(() => setIsCollapsed((v) => !v), []);

  useEffect(() => {
    if (running > 0) {
      wasRunningRef.current = true;
    } else if (wasRunningRef.current && allDone) {
      const timer = setTimeout(() => setIsCollapsed(true), 800);
      return () => clearTimeout(timer);
    }
  }, [running, allDone]);

  return (
    <div className="mt-1 mb-1.5">
      {/* Group header */}
      <button
        type="button"
        onClick={toggleCollapse}
        className={cn(
          "w-full flex items-center gap-2 px-3 py-2 rounded-t-lg text-xs transition-all duration-300",
          "bg-card hover:bg-muted/50 border border-b-0 border-border",
          isCollapsed && "rounded-b-lg border-b",
        )}
      >
        <Bot className="w-3.5 h-3.5 text-muted-foreground/60" />
        <span className="font-medium text-sm text-foreground/80">{title}</span>
        <span className="text-muted-foreground/50 mx-1">·</span>

        {running > 0 && (
          <span className="flex items-center gap-1 text-[var(--ansi-blue)]">
            <Loader2 className="w-3 h-3 animate-spin" />
            {running} running
          </span>
        )}
        {completed > 0 && (
          <span className="flex items-center gap-1 text-[var(--ansi-green)]">
            <CheckCircle2 className="w-3 h-3" />
            {completed} done
          </span>
        )}
        {failed > 0 && (
          <span className="flex items-center gap-1 text-[var(--ansi-red)]">
            <XCircle className="w-3 h-3" />
            {failed} failed
          </span>
        )}

        <div className="ml-auto flex items-center gap-2">
          {titleExtra}
          {allDone && totalDurationMs > 0 && (
            <span className="text-muted-foreground/40">{formatDurationLong(totalDurationMs)}</span>
          )}
          {isCollapsed ? (
            <ChevronDown className="w-3.5 h-3.5 text-muted-foreground/40" />
          ) : (
            <ChevronUp className="w-3.5 h-3.5 text-muted-foreground/40" />
          )}
        </div>
      </button>

      {/* Collapsible content with smooth animation */}
      <div
        className="grid transition-[grid-template-rows] duration-300 ease-in-out"
        style={{ gridTemplateRows: isCollapsed ? "0fr" : "1fr" }}
      >
        <div className="overflow-hidden">
          {/* Progress bar */}
          {!allDone && (
            <div className="h-0.5 bg-muted/20 border-x border-border">
              <div
                className={cn(
                  "h-full transition-all duration-500 ease-out",
                  hasFailure ? "bg-red-400" : "bg-[var(--ansi-green)]",
                )}
                style={{ width: `${progress}%` }}
              />
            </div>
          )}

          {/* Content */}
          <div className="border border-t-0 border-border rounded-b-lg bg-card/50">
            {children}
          </div>
        </div>
      </div>
    </div>
  );
});
