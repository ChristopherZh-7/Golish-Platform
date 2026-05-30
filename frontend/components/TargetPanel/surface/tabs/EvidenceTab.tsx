import { Activity, CheckCircle2, Database } from "lucide-react";
import type { Target } from "@/lib/pentest/types";
import type { TimelineEntry } from "@/lib/security-analysis";
import { EmptyInline, Section } from "../SurfaceParts";
import { formatTime } from "../surfaceModel";

export function EvidenceTab({
  target,
  timeline,
  logs,
  loading,
}: {
  target: Target;
  timeline: TimelineEntry[];
  logs: Array<{
    id: number;
    action: string;
    status: string;
    toolName: string | null;
    createdAt: number;
  }>;
  loading: boolean;
}) {
  return (
    <div className="space-y-2.5">
      <Section title="Source Evidence" subtitle="why this target exists">
        <div className="rounded border border-border/20 bg-muted/5 px-2 py-1.5 text-[11px]">
          <div className="flex items-center gap-2">
            <CheckCircle2 className="w-3.5 h-3.5 text-green-300" />
            <span className="text-foreground">{target.source || "manual"}</span>
            <span className="text-muted-foreground">· scope={target.scope}</span>
          </div>
        </div>
      </Section>
      <Section title="Operation Logs" subtitle={`${logs.length} latest event(s)`}>
        {timeline.length === 0 && logs.length === 0 ? (
          <EmptyInline loading={loading} label="No operation logs for this target yet" />
        ) : (
          <div className="space-y-1">
            {timeline.slice(0, 30).map((entry, index) => (
              <div
                key={`${entry.source}:${entry.event}:${entry.createdAt}:${index}`}
                className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
              >
                <div className="flex items-center gap-2 text-[11px]">
                  <Activity className="w-3.5 h-3.5 text-accent/70" />
                  <span className="rounded bg-muted/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                    {entry.source}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-foreground/80">
                    {entry.details || entry.event}
                  </span>
                  {entry.toolName && (
                    <span className="rounded bg-accent/10 px-1.5 py-0.5 text-[9px] text-accent">
                      {entry.toolName}
                    </span>
                  )}
                  <span className="text-[9px] text-muted-foreground">
                    {formatTime(entry.createdAt)}
                  </span>
                </div>
              </div>
            ))}
            {timeline.length === 0 &&
              logs.map((log) => (
                <div
                  key={log.id}
                  className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
                >
                  <div className="flex items-center gap-2 text-[11px]">
                    <Database className="w-3.5 h-3.5 text-accent/70" />
                    <span className="min-w-0 flex-1 truncate text-foreground/80">{log.action}</span>
                    {log.toolName && (
                      <span className="rounded bg-accent/10 px-1.5 py-0.5 text-[9px] text-accent">
                        {log.toolName}
                      </span>
                    )}
                    <span className="text-[9px] text-muted-foreground">
                      {new Date(log.createdAt).toLocaleTimeString()}
                    </span>
                  </div>
                </div>
              ))}
          </div>
        )}
      </Section>
    </div>
  );
}
