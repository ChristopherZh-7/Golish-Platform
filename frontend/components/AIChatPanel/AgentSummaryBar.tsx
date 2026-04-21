import { useEffect, useRef, useState } from "react";
import {
  ArrowRight, Bot, CheckCircle2, ChevronDown, ChevronUp, Code2, Loader2, Search, XCircle,
} from "lucide-react";
import { useStore } from "@/store";

const EMPTY_SUB_AGENTS: never[] = [];

const AGENT_ICON_MAP: Record<string, typeof Bot> = {
  coder: Code2,
  researcher: Search,
  explorer: Search,
};

function getAgentIcon(name: string): typeof Bot {
  const lower = name.toLowerCase();
  for (const [key, icon] of Object.entries(AGENT_ICON_MAP)) {
    if (lower.includes(key)) return icon;
  }
  return Bot;
}

export function AgentSummaryBar() {
  const [isExpanded, setIsExpanded] = useState(false);
  const wasRunningRef = useRef(false);
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
  const errored = subAgents.filter((a) => a.status === "error").length;
  const totalDurationMs = subAgents.reduce((sum, a) => sum + (a.durationMs ?? 0), 0);

  const scrollToAgent = (parentRequestId: string) => {
    const el = document.querySelector(`[data-agent-block="sub-agent-${parentRequestId}"]`) as HTMLElement | null;
    if (!el) return;
    el.scrollIntoView({ behavior: "smooth", block: "center" });
    el.style.transition = "box-shadow 0.3s ease, background-color 0.3s ease";
    el.style.boxShadow = "0 0 0 2px var(--accent), 0 0 16px 2px rgba(var(--accent-rgb, 80 200 180), 0.3)";
    el.style.backgroundColor = "rgba(var(--accent-rgb, 80 200 180), 0.08)";
    setTimeout(() => {
      el.style.boxShadow = "";
      el.style.backgroundColor = "";
      setTimeout(() => { el.style.transition = ""; }, 300);
    }, 2000);
  };

  return (
    <div className="mx-4 my-1.5 rounded-md bg-muted/20 text-[11px] text-muted-foreground overflow-hidden">
      <button
        type="button"
        onClick={() => setIsExpanded((v) => !v)}
        className="w-full flex items-center gap-3 px-3 py-1.5 hover:bg-muted/30 transition-colors"
      >
        <div className="flex items-center gap-1">
          <Bot className="w-3 h-3" />
          <span className="font-medium">
            {subAgents.length} agent{subAgents.length > 1 ? "s" : ""}
          </span>
        </div>
        {running > 0 && (
          <div className="flex items-center gap-1">
            <Loader2 className="w-2.5 h-2.5 animate-spin text-accent" />
            <span>{running} active</span>
          </div>
        )}
        {completed > 0 && (
          <div className="flex items-center gap-1">
            <CheckCircle2 className="w-2.5 h-2.5 text-green-500" />
            <span>{completed} done</span>
          </div>
        )}
        {errored > 0 && (
          <div className="flex items-center gap-1">
            <XCircle className="w-2.5 h-2.5 text-destructive" />
            <span>{errored} failed</span>
          </div>
        )}
        <div className="ml-auto flex items-center gap-1.5">
          {totalDurationMs > 0 && (
            <span className="text-[10px] text-muted-foreground/60">
              {(totalDurationMs / 1000).toFixed(1)}s total
            </span>
          )}
          {isExpanded ? (
            <ChevronUp className="w-3 h-3 text-muted-foreground/40" />
          ) : (
            <ChevronDown className="w-3 h-3 text-muted-foreground/40" />
          )}
        </div>
      </button>

      <div
        className="grid transition-[grid-template-rows] duration-300 ease-in-out"
        style={{ gridTemplateRows: isExpanded ? "1fr" : "0fr" }}
      >
        <div className="overflow-hidden border-t border-border/30 px-3 py-1">
          {subAgents.map((agent) => {
            const AgentIcon = getAgentIcon(agent.agentName);
            return (
              <div
                key={agent.parentRequestId}
                className="flex items-center gap-2 py-1 text-[10px]"
              >
                <AgentIcon className="w-3 h-3 flex-shrink-0 text-muted-foreground/60" />
                <span className="flex-1 truncate">
                  {agent.agentName || agent.agentId}
                </span>
                {agent.status === "running" && (
                  <Loader2 className="w-2.5 h-2.5 animate-spin text-accent" />
                )}
                {agent.status === "completed" && (
                  <CheckCircle2 className="w-2.5 h-2.5 text-green-500" />
                )}
                {agent.status === "error" && (
                  <XCircle className="w-2.5 h-2.5 text-destructive" />
                )}
                {agent.durationMs !== undefined && (
                  <span className="text-muted-foreground/40">
                    {(agent.durationMs / 1000).toFixed(1)}s
                  </span>
                )}
                <button
                  type="button"
                  onClick={(e) => { e.stopPropagation(); scrollToAgent(agent.parentRequestId); }}
                  className="p-0.5 hover:bg-accent/30 rounded transition-colors"
                  title="Scroll to agent card"
                >
                  <ArrowRight className="w-2.5 h-2.5 text-muted-foreground/40 hover:text-accent" />
                </button>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

