/**
 * Knowledge Graph snapshot card.
 *
 * Read-only debug surface that loads the most-recently-touched
 * entities from the backend KG and renders them grouped by type.
 * The agent writes via the `graph_*` LLM tools and the regex
 * auto-extractor; this card lets a human eyeball what landed.
 *
 * Designed to drop into a settings tab without any wiring:
 * fetches on mount, paginates implicitly via the backend's
 * default limit of 50, and degrades to an "empty" placeholder
 * when nothing has been recorded yet.
 */

import { Database, Loader2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { groupEntitiesByType, type KgEntity, kgListEntities } from "@/lib/ai";
import { cn } from "@/lib/utils";

interface KnowledgeGraphSectionProps {
  /**
   * Optional project filter forwarded to `kg_list_entities`. When the
   * settings tab is invoked from a known project context the parent
   * can pass it in; otherwise the backend returns entities across
   * all projects (default behaviour).
   */
  projectId?: string | null;
  /** Override the default fetch limit (1-500). */
  limit?: number;
  /** Optional extra className for the outer wrapper. */
  className?: string;
}

const TYPE_LABEL: Record<string, string> = {
  host: "Hosts",
  service: "Services",
  vulnerability: "Vulnerabilities",
  credential: "Credentials",
  technique: "Techniques",
  endpoint: "Endpoints",
};

const TYPE_ORDER = [
  "host",
  "service",
  "vulnerability",
  "credential",
  "technique",
  "endpoint",
];

export function KnowledgeGraphSection({
  projectId,
  limit,
  className,
}: KnowledgeGraphSectionProps) {
  const [entities, setEntities] = useState<KgEntity[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastRefreshed, setLastRefreshed] = useState<number>(0);

  const fetchData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await kgListEntities({ projectId, limit });
      setEntities(data);
      setLastRefreshed(Date.now());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [projectId, limit]);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  const grouped = useMemo(() => groupEntitiesByType(entities), [entities]);
  const total = entities.length;

  const sortedTypes = useMemo(() => {
    const known = TYPE_ORDER.filter((t) => grouped[t]?.length);
    const extras = Object.keys(grouped)
      .filter((t) => !TYPE_ORDER.includes(t))
      .sort();
    return [...known, ...extras];
  }, [grouped]);

  return (
    <section
      className={cn(
        "space-y-3 rounded-lg border border-[var(--border-subtle)] bg-background/40 p-4",
        className
      )}
    >
      <header className="flex items-center gap-2">
        <Database className="h-4 w-4 text-accent" />
        <h3 className="text-sm font-medium text-foreground">Knowledge Graph</h3>
        <span className="text-xs text-muted-foreground">
          {loading ? "loading…" : `${total} entities`}
        </span>
        <button
          type="button"
          onClick={fetchData}
          disabled={loading}
          className={cn(
            "ml-auto inline-flex items-center gap-1 rounded px-2 py-1 text-xs",
            "border border-[var(--border-subtle)] hover:bg-[var(--bg-hover)]",
            loading && "opacity-50 cursor-not-allowed"
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

      {error ? (
        <p className="text-xs text-red-400/80" role="alert">
          {error}
        </p>
      ) : total === 0 && !loading ? (
        <p className="text-xs text-muted-foreground/70">
          No entities recorded yet. The agent will populate this graph via the
          <code className="mx-1 rounded bg-[var(--bg-hover)] px-1">graph_*</code>
          tools or the regex auto-extractor as it works.
        </p>
      ) : (
        <ul className="space-y-2">
          {sortedTypes.map((type) => {
            const rows = grouped[type] ?? [];
            if (rows.length === 0) return null;
            return (
              <li key={type} className="space-y-1">
                <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
                  <span>{TYPE_LABEL[type] ?? type}</span>
                  <span className="text-[10px] text-muted-foreground/60">
                    ({rows.length})
                  </span>
                </div>
                <ul className="space-y-0.5 pl-3">
                  {rows.slice(0, 8).map((ent) => (
                    <li
                      key={ent.id}
                      className="flex items-baseline gap-2 text-xs text-foreground/80"
                    >
                      <span className="font-mono text-[11px] truncate">
                        {ent.name}
                      </span>
                      <span className="text-[10px] text-muted-foreground/60">
                        updated {new Date(ent.updated_at).toLocaleString()}
                      </span>
                    </li>
                  ))}
                  {rows.length > 8 && (
                    <li className="pl-3 text-[10px] text-muted-foreground/50">
                      +{rows.length - 8} more
                    </li>
                  )}
                </ul>
              </li>
            );
          })}
        </ul>
      )}

      {lastRefreshed > 0 && !loading && !error && (
        <footer className="pt-1 text-[10px] text-muted-foreground/50">
          Last refresh: {new Date(lastRefreshed).toLocaleTimeString()}
        </footer>
      )}
    </section>
  );
}
