import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef, useState } from "react";
import type { InvestigationHypothesisListView } from "@/lib/api/investigation";
import { HypothesisDetail } from "./HypothesisDetail";
import { WorkspaceAsyncState } from "./WorkspaceAsyncState";
import type { ProjectionResource } from "./useInvestigationProjection";

type HypothesisItem = InvestigationHypothesisListView["hypotheses"][number];

export function HypothesesTab({
  resource,
  detail,
  selectedRevisionId,
  onSelect,
  onLoadMore,
  onRetry,
}: {
  resource: ProjectionResource<InvestigationHypothesisListView>;
  detail: ProjectionResource<import("@/lib/api/investigation").InvestigationHypothesisDetailView>;
  selectedRevisionId: string | null;
  onSelect: (revisionId: string) => void;
  onLoadMore: () => void;
  onRetry: () => void;
}) {
  const [epistemicFilter, setEpistemicFilter] = useState("all");
  const [readinessFilter, setReadinessFilter] = useState("all");
  const [activeIndex, setActiveIndex] = useState(0);
  const railRef = useRef<HTMLDivElement>(null);
  const items = useMemo(
    () =>
      (resource.data?.hypotheses ?? []).filter(
        (item) =>
          (epistemicFilter === "all" || item.epistemicState === epistemicFilter) &&
          (readinessFilter === "all" || item.planningReadiness === readinessFilter)
      ),
    [epistemicFilter, readinessFilter, resource.data?.hypotheses]
  );
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => railRef.current,
    estimateSize: () => 82,
    overscan: 8,
  });
  const virtualItems = virtualizer.getVirtualItems();

  useEffect(() => {
    const last = virtualItems[virtualItems.length - 1]?.index ?? -1;
    if (last >= items.length - 10 && resource.nextCursor) onLoadMore();
  }, [items.length, onLoadMore, resource.nextCursor, virtualItems]);

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (items.length === 0) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const delta = event.key === "ArrowDown" ? 1 : -1;
      const next = Math.max(0, Math.min(items.length - 1, activeIndex + delta));
      setActiveIndex(next);
      virtualizer.scrollToIndex(next, { align: "auto" });
    } else if (event.key === "Enter") {
      event.preventDefault();
      const item = items[activeIndex];
      if (item) onSelect(item.revisionId);
    }
  };

  const state = (
    <WorkspaceAsyncState
      resource={resource}
      label="hypotheses"
      empty={resource.data !== undefined && items.length === 0}
      onRetry={onRetry}
    />
  );
  if (!resource.data || items.length === 0) return state;

  return (
    <div className="grid h-full min-h-0 grid-cols-[minmax(240px,34%)_1fr]">
      <aside className="flex min-h-0 flex-col border-r border-border/25">
        <div className="grid grid-cols-2 gap-2 border-b border-border/25 p-2">
          <label className="text-[10px] text-muted-foreground">
            Epistemic
            <select
              className="mt-1 w-full rounded border border-border/30 bg-background px-1.5 py-1 text-[11px]"
              value={epistemicFilter}
              onChange={(event) => setEpistemicFilter(event.target.value)}
            >
              <option value="all">All</option>
              {[...new Set(resource.data.hypotheses.map((item) => item.epistemicState))].map((value) => (
                <option key={value} value={value}>{value}</option>
              ))}
            </select>
          </label>
          <label className="text-[10px] text-muted-foreground">
            Readiness
            <select
              className="mt-1 w-full rounded border border-border/30 bg-background px-1.5 py-1 text-[11px]"
              value={readinessFilter}
              onChange={(event) => setReadinessFilter(event.target.value)}
            >
              <option value="all">All</option>
              {[...new Set(resource.data.hypotheses.map((item) => item.planningReadiness))].map((value) => (
                <option key={value} value={value}>{value}</option>
              ))}
            </select>
          </label>
        </div>
        <div
          ref={railRef}
          role="listbox"
          aria-label="Hypotheses"
          tabIndex={0}
          className="min-h-0 flex-1 overflow-y-auto outline-none"
          onKeyDown={onKeyDown}
        >
          <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
            {virtualItems.map((virtualRow) => {
              const item: HypothesisItem = items[virtualRow.index];
              return (
                <button
                  key={item.revisionId}
                  type="button"
                  role="option"
                  aria-selected={selectedRevisionId === item.revisionId}
                  className={`absolute left-0 top-0 w-full border-b border-border/20 px-3 py-2 text-left hover:bg-muted/20 ${selectedRevisionId === item.revisionId ? "bg-cyan-500/[0.07]" : ""}`}
                  style={{ height: virtualRow.size, transform: `translateY(${virtualRow.start}px)` }}
                  onClick={() => {
                    setActiveIndex(virtualRow.index);
                    onSelect(item.revisionId);
                  }}
                >
                  <div className="line-clamp-2 text-xs font-medium">{item.predicateSummary}</div>
                  <div className="mt-1 flex flex-wrap gap-1 text-[9px] text-muted-foreground">
                    <span>{item.epistemicState}</span><span>·</span><span>{item.planningReadiness}</span>
                    <span className="ml-auto tabular-nums">gaps {item.gapCount}</span>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      </aside>
      <div className="min-h-0 overflow-y-auto">
        <HypothesisDetail resource={detail} />
      </div>
    </div>
  );
}
