import { useEffect, useRef, useState } from "react";
import {
  Bot, CheckCircle2, ChevronDown, Loader2, XCircle,
} from "lucide-react";
import { AnchorChip } from "@/components/ui/AnchorChip";
import { getAgentIcon } from "@/lib/sub-agent-theme";
import { formatDurationShort } from "@/lib/time";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";

const EMPTY_SUB_AGENTS: never[] = [];

export function SubAgentSummaryBar() {
  const [isExpanded, setIsExpanded] = useState(false);
  const wasRunningRef = useRef(false);
  const barRef = useRef<HTMLDivElement>(null);
  const activeSessionId = useStore((s) => s.activeSessionId);
  const subAgents = useStore((s) =>
    activeSessionId ? (s.activeSubAgents[activeSessionId] ?? EMPTY_SUB_AGENTS) : EMPTY_SUB_AGENTS
  );

  const running = subAgents.filter((a) => a.status === "running").length;
  const allDone = running === 0 && subAgents.length > 0;

  useEffect(() => {
    if (running > 0) {
      wasRunningRef.current = true;
    } else if (wasRunningRef.current && allDone) {
      const timer = setTimeout(() => setIsExpanded(false), 800);
      return () => clearTimeout(timer);
    }
  }, [running, allDone]);

  if (subAgents.length === 0) return null;

  const completed = subAgents.filter((a) => a.status === "completed").length;
  const errored = subAgents.filter((a) => a.status === "error" || a.status === "interrupted").length;

  const handleToggle = () => {
    setIsExpanded((v) => {
      if (!v) {
        requestAnimationFrame(() => {
          barRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
        });
      }
      return !v;
    });
  };

  return (
    <div ref={barRef} className="mx-4 my-2 rounded-lg border border-border/30 bg-background/50 overflow-hidden">
      <button
        type="button"
        onClick={handleToggle}
        className="w-full flex items-center gap-2 px-3 py-2 text-left"
      >
        <Bot className="w-3.5 h-3.5 text-accent flex-shrink-0" />
        <span className="text-[12px] font-medium text-foreground flex-1">
          {subAgents.length} Agent{subAgents.length > 1 ? "s" : ""}
        </span>
        <div className="flex items-center gap-2">
          {running > 0 && (
            <span className="flex items-center gap-1 text-[11px] text-accent">
              <Loader2 className="w-3 h-3 animate-spin" />
              {running} active
            </span>
          )}
          {completed > 0 && (
            <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
              <CheckCircle2 className="w-3 h-3 text-[var(--ansi-green)]" />
              {completed}
            </span>
          )}
          {errored > 0 && (
            <span className="flex items-center gap-1 text-[11px] text-red-400">
              <XCircle className="w-3 h-3" />
              {errored}
            </span>
          )}
        </div>
        <ChevronDown
          className={cn(
            "w-3 h-3 text-muted-foreground transition-transform",
            isExpanded && "rotate-180",
          )}
        />
      </button>

      {isExpanded && (
        <div className="border-t border-border/20 px-3 py-1">
          {subAgents.map((agent) => {
            const AgentIcon = getAgentIcon(agent.agentName);
            const isDone = agent.status === "completed";
            const isErr = agent.status === "error";
            const isInt = agent.status === "interrupted";
            const isRun = agent.status === "running";
            return (
              <button
                key={agent.parentRequestId}
                type="button"
                onClick={() => {
                  if (activeSessionId) {
                    const store = useStore.getState();
                    store.setToolDetailRequestIds(activeSessionId, [agent.parentRequestId]);
                    store.setDetailViewMode(activeSessionId, "sub-agent-detail");
                  }
                }}
                className={cn(
                  "w-full flex items-center gap-2 py-1.5 text-left rounded px-1.5 transition-colors",
                  "hover:bg-accent/10 cursor-pointer group",
                )}
              >
                <AgentIcon className="w-3.5 h-3.5 flex-shrink-0 text-muted-foreground" />
                <span className={cn(
                  "truncate text-[11px] font-medium",
                  isErr && "text-red-400",
                  isInt && "text-amber-400",
                  isDone && "text-foreground/80",
                  isRun && "text-foreground",
                )}>
                  {agent.agentName || agent.agentId}
                </span>
                <AnchorChip sessionId={activeSessionId} requestId={agent.parentRequestId} />
                <div className="flex-1" />
                {isRun && (
                  <Loader2 className="w-3 h-3 animate-spin text-accent flex-shrink-0" />
                )}
                {isDone && (
                  <CheckCircle2 className="w-3 h-3 text-[var(--ansi-green)] flex-shrink-0" />
                )}
                {isErr && (
                  <XCircle className="w-3 h-3 text-red-400 flex-shrink-0" />
                )}
                {agent.durationMs != null && (
                  <span className="text-[10px] text-muted-foreground/60 tabular-nums flex-shrink-0">
                    {formatDurationShort(agent.durationMs)}
                  </span>
                )}
                <span className="text-[10px] text-muted-foreground/60 group-hover:text-accent/60 transition-colors flex-shrink-0">
                  Details →
                </span>
              </button>
            );
          })}
        </div>
      )}

    </div>
  );
}
