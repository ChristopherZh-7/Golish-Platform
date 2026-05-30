/**
 * Sub-agent dispatch inflight monitor.
 *
 * Read-only diagnostic card that lists every sub-agent dispatch the
 * runtime currently considers "in flight" for the active conversation.
 * After a crash + restart, the row that never got a `record_finish`
 * shows up here for ≤24h (after which the backend reaps it on the
 * next read).
 *
 * Companion to KnowledgeGraphSection — same drop-in design, same
 * three-state UX, same refresh affordance.
 */

import { Activity, Loader2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { listRunningSubAgentDispatches, type RunningSubAgentDispatch } from "@/lib/ai";
import { formatRelativeAgo } from "@/lib/time";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";

interface DispatchInflightSectionProps {
  /** Force a particular session; defaults to the active conversation's aiSessionId. */
  sessionId?: string | null;
  /** Optional extra className for the outer wrapper. */
  className?: string;
}

export function DispatchInflightSection({ sessionId, className }: DispatchInflightSectionProps) {
  const activeAiSessionId = useStore((s) => {
    if (sessionId !== undefined) return sessionId;
    if (!s.activeConversationId) return null;
    return s.conversations[s.activeConversationId]?.aiSessionId ?? null;
  });

  const [rows, setRows] = useState<RunningSubAgentDispatch[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastRefreshed, setLastRefreshed] = useState<number>(0);

  const fetchData = useCallback(async () => {
    if (!activeAiSessionId) {
      setRows([]);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const data = await listRunningSubAgentDispatches(activeAiSessionId);
      setRows(data);
      setLastRefreshed(Date.now());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [activeAiSessionId]);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  return (
    <section
      className={cn(
        "space-y-3 rounded-lg border border-[var(--border-subtle)] bg-background/40 p-4",
        className
      )}
    >
      <header className="flex items-center gap-2">
        <Activity className="h-4 w-4 text-accent" />
        <h3 className="text-sm font-medium text-foreground">Sub-Agent Dispatches</h3>
        <span className="text-xs text-muted-foreground">
          {loading ? "loading…" : `${rows.length} in-flight`}
        </span>
        <button
          type="button"
          onClick={fetchData}
          disabled={loading || !activeAiSessionId}
          className={cn(
            "ml-auto inline-flex items-center gap-1 rounded px-2 py-1 text-xs",
            "border border-[var(--border-subtle)] hover:bg-[var(--bg-hover)]",
            (loading || !activeAiSessionId) && "opacity-50 cursor-not-allowed"
          )}
          title="Refresh"
        >
          {loading ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <RefreshCw className="h-3 w-3" />
          )}
          Refresh
        </button>
      </header>

      {!activeAiSessionId ? (
        <p className="text-xs text-muted-foreground/70">
          No active AI session — start a conversation to see dispatch state.
        </p>
      ) : error ? (
        <p className="text-xs text-red-400/80" role="alert">
          {error}
        </p>
      ) : rows.length === 0 && !loading ? (
        <p className="text-xs text-muted-foreground/70">
          No mid-flight sub-agent dispatches. Anything stuck &gt; 24h gets auto-cancelled by the
          backend on the next read.
        </p>
      ) : (
        <ul className="space-y-1.5">
          {rows.map((row) => (
            <li key={row.id} className="flex items-baseline gap-2 text-xs text-foreground/80">
              <span className="font-mono text-[11px] uppercase text-accent">{row.agent_id}</span>
              {row.depth > 0 && (
                <span className="text-[10px] text-muted-foreground/60">depth={row.depth}</span>
              )}
              <span className="text-[10px] text-muted-foreground/60 ml-auto">
                started {formatRelativeAgo(row.started_at, { minUnit: "second", maxUnit: "hour" })}
              </span>
            </li>
          ))}
        </ul>
      )}

      {lastRefreshed > 0 && !loading && !error && activeAiSessionId && (
        <footer className="pt-1 text-[10px] text-muted-foreground/50">
          Last refresh: {new Date(lastRefreshed).toLocaleTimeString()}
        </footer>
      )}
    </section>
  );
}
