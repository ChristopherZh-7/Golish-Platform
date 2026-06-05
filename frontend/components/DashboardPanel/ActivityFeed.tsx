import { Zap } from "lucide-react";
import type { AgentUsage, ToolCallStat } from "@/lib/dashboard";
import { formatDurationShort } from "@/lib/time";
import { cn } from "@/lib/utils";
import { fmtNum } from "./StatCards";

export function eventLabel(type: string): string {
  const map: Record<string, string> = {
    target_added: "Target Added",
    target_updated: "Target Updated",
    finding_created: "Finding Reported",
    scan_completed: "Scan Complete",
    credential_added: "Credential Added",
  };
  return map[type] || type.replace(/_/g, " ");
}

export function ActivityDot({ type }: { type: string }) {
  const color = type.includes("finding")
    ? "bg-red-400"
    : type.includes("scan")
      ? "bg-orange-400"
      : type.includes("target")
        ? "bg-blue-400"
        : "bg-muted-foreground/40";

  return <div className={cn("w-1.5 h-1.5 rounded-full mt-1 flex-shrink-0", color)} />;
}

export function AgentUsageChart({ agents }: { agents: AgentUsage[] }) {
  const maxTokens = Math.max(...agents.map((a) => a.total_tokens_in + a.total_tokens_out), 1);

  return (
    <div className="space-y-2">
      {agents.map((a) => {
        const total = a.total_tokens_in + a.total_tokens_out;
        const pct = (total / maxTokens) * 100;
        const inPct = total > 0 ? (a.total_tokens_in / total) * 100 : 0;
        return (
          <div key={a.agent} className="space-y-1">
            <div className="flex items-center justify-between text-[10px]">
              <span className="text-foreground/60 truncate">{a.agent}</span>
              <div className="flex items-center gap-2 text-muted-foreground/40 tabular-nums">
                <span>{fmtNum(total)} tok</span>
                {a.total_cost > 0 && <span>${a.total_cost.toFixed(4)}</span>}
              </div>
            </div>
            <div className="h-1.5 rounded-full bg-muted/15 overflow-hidden">
              <div
                className="h-full rounded-full flex overflow-hidden transition-all duration-500"
                style={{ width: `${pct}%` }}
              >
                <div className="h-full bg-purple-500/50" style={{ width: `${inPct}%` }} />
                <div className="h-full bg-violet-400/40" style={{ width: `${100 - inPct}%` }} />
              </div>
            </div>
          </div>
        );
      })}
      <div className="flex items-center gap-3 pt-0.5">
        <div className="flex items-center gap-1 text-[9px]">
          <div className="w-1.5 h-1.5 rounded-full bg-purple-500/50" />
          <span className="text-muted-foreground/40">Input</span>
        </div>
        <div className="flex items-center gap-1 text-[9px]">
          <div className="w-1.5 h-1.5 rounded-full bg-violet-400/40" />
          <span className="text-muted-foreground/40">Output</span>
        </div>
      </div>
    </div>
  );
}

export function ToolCallChart({ tools, maxCount }: { tools: ToolCallStat[]; maxCount: number }) {
  return (
    <div className="space-y-1.5">
      {tools.map((t) => {
        const pct = (t.total_count / maxCount) * 100;
        return (
          <div key={t.name} className="flex items-center gap-2 text-[10px]">
            <span className="text-foreground/60 w-28 truncate flex-shrink-0" title={t.name}>
              {t.name}
            </span>
            <div className="flex-1 h-1.5 rounded-full bg-muted/15 overflow-hidden">
              <div
                className="h-full rounded-full bg-blue-500/40 transition-all duration-500"
                style={{ width: `${pct}%` }}
              />
            </div>
            <div className="flex items-center gap-1.5 text-muted-foreground/40 tabular-nums flex-shrink-0">
              <span className="font-medium text-foreground/60">{t.total_count}</span>
              {t.avg_duration_ms > 0 && (
                <span className="text-[9px]">
                  <Zap className="w-2 h-2 inline-block mr-0.5 text-amber-400/50" />
                  {formatDurationShort(Math.round(t.avg_duration_ms))}
                </span>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
