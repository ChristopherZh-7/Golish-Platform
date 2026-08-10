import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, useState } from "react";
import type {
  InvestigationCampaignDetailResponse,
  InvestigationCampaignPageResponse,
} from "@/lib/api/investigation";
import { CampaignDetail } from "./CampaignDetail";
import { WorkspaceAsyncState } from "./WorkspaceAsyncState";
import type { ProjectionResource } from "./useInvestigationProjection";

type CampaignItem = InvestigationCampaignPageResponse["campaigns"][number];

export function CampaignsTab({
  operationId,
  refreshVersion,
  resource,
  detail,
  selectedCampaignId,
  onSelect,
  onLoadMore,
  onRetry,
}: {
  operationId: string;
  refreshVersion: number;
  resource: ProjectionResource<InvestigationCampaignPageResponse>;
  detail: ProjectionResource<InvestigationCampaignDetailResponse>;
  selectedCampaignId: string | null;
  onSelect: (campaignId: string) => void;
  onLoadMore: () => void;
  onRetry: () => void;
}) {
  const [activeIndex, setActiveIndex] = useState(0);
  const railRef = useRef<HTMLDivElement>(null);
  const items = resource.data?.campaigns ?? [];
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => railRef.current,
    estimateSize: () => 70,
    overscan: 8,
  });
  const virtualItems = virtualizer.getVirtualItems();

  useEffect(() => {
    const last = virtualItems[virtualItems.length - 1]?.index ?? -1;
    if (last >= items.length - 10 && resource.nextCursor) onLoadMore();
  }, [items.length, onLoadMore, resource.nextCursor, virtualItems]);

  const state = (
    <WorkspaceAsyncState
      resource={resource}
      label="Campaigns"
      empty={resource.data !== undefined && items.length === 0}
      onRetry={onRetry}
    />
  );
  if (!resource.data || items.length === 0) return state;

  return (
    <div className="grid h-full min-h-0 grid-cols-[minmax(230px,32%)_1fr]">
      <div
        ref={railRef}
        role="listbox"
        aria-label="Verification Campaigns"
        tabIndex={0}
        className="min-h-0 overflow-y-auto border-r border-border/25 outline-none"
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            const delta = event.key === "ArrowDown" ? 1 : -1;
            const next = Math.max(0, Math.min(items.length - 1, activeIndex + delta));
            setActiveIndex(next);
            virtualizer.scrollToIndex(next, { align: "auto" });
          } else if (event.key === "Enter") {
            const item = items[activeIndex];
            if (item) onSelect(item.campaignId);
          }
        }}
      >
        <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
          {virtualItems.map((virtualRow) => {
            const item: CampaignItem = items[virtualRow.index];
            return (
              <button
                key={item.campaignId}
                type="button"
                role="option"
                aria-selected={selectedCampaignId === item.campaignId}
                className={`absolute left-0 top-0 w-full border-b border-border/20 px-3 py-2 text-left hover:bg-muted/20 ${selectedCampaignId === item.campaignId ? "bg-violet-500/[0.07]" : ""}`}
                style={{ height: virtualRow.size, transform: `translateY(${virtualRow.start}px)` }}
                onClick={() => {
                  setActiveIndex(virtualRow.index);
                  onSelect(item.campaignId);
                }}
              >
                <div className="truncate text-xs font-medium">{item.label}</div>
                <div className="mt-1 flex flex-wrap gap-1 text-[9px] text-muted-foreground">
                  <span>Wave {item.waveOrdinal}</span><span>·</span><span>{item.state}</span>
                  <span className="ml-auto">{item.coverageStatus}</span>
                </div>
              </button>
            );
          })}
        </div>
      </div>
      <div className="min-h-0 overflow-y-auto">
        <CampaignDetail
          operationId={operationId}
          refreshVersion={refreshVersion}
          resource={detail}
        />
      </div>
    </div>
  );
}
