import type { TFunction } from "i18next";
import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  type DetailTab,
  SEV_COLORS,
  SEV_DOT,
  type ViewMode,
  type VulnEntry,
  type VulnLink,
} from "./types";

interface VulnIntelCveListProps {
  displayedEntries: VulnEntry[];
  displayEntries: VulnEntry[];
  intelDisplayCount: number;
  loading: boolean;
  viewMode: ViewMode;
  searchQuery: string;
  expandedCve: string | null;
  vulnLinks: Record<string, VulnLink>;
  loadMorePage: number;
  setExpandedCve: (id: string | null) => void;
  setDetailTab: (tab: DetailTab) => void;
  setIntelDisplayCount: (updater: (c: number) => number) => void;
  handleLoadMore: () => void;
  t: TFunction;
}

const INTEL_PAGE = 200;

export function VulnIntelCveList({
  displayedEntries,
  displayEntries,
  intelDisplayCount,
  loading,
  viewMode,
  searchQuery,
  expandedCve,
  vulnLinks,
  loadMorePage,
  setExpandedCve,
  setDetailTab,
  setIntelDisplayCount,
  handleLoadMore,
  t,
}: VulnIntelCveListProps) {
  return (
    <div className="flex-1 overflow-y-auto py-1 px-1.5">
      {displayEntries.length === 0 ? (
        <div className="text-center text-[11px] text-muted-foreground/30 py-12">
          {loading
            ? t("vulnIntel.fetching", "Fetching vulnerability data...")
            : viewMode === "matched"
              ? t("vulnIntel.noMatched", "No matched vulnerabilities")
              : t("vulnIntel.clickRefresh", "Click refresh to fetch latest CVEs")}
        </div>
      ) : (
        <>
          {displayedEntries.map((entry) => {
            const isSelected = expandedCve === entry.cve_id;
            const link = vulnLinks[entry.cve_id];
            const hasPoc = link && link.pocTemplates.length > 0;
            const hasWiki = link && link.wikiPaths.length > 0;

            return (
              <div
                key={entry.cve_id}
                className={cn(
                  "flex items-start gap-2 py-1.5 px-2 rounded cursor-pointer transition-colors",
                  isSelected
                    ? "bg-accent/10 border border-accent/20"
                    : "hover:bg-muted/5 border border-transparent"
                )}
                onClick={() => {
                  setExpandedCve(isSelected ? null : entry.cve_id);
                  setDetailTab("intel");
                }}
              >
                <span
                  className={cn(
                    "w-1.5 h-1.5 rounded-full mt-1.5 flex-shrink-0",
                    SEV_DOT[entry.severity] || "bg-slate-500"
                  )}
                />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5 flex-wrap">
                    <span className="text-[10px] font-mono font-medium text-accent/80">
                      {entry.cve_id}
                    </span>
                    <span
                      className={cn(
                        "text-[8px] px-1.5 py-0.5 rounded-full border capitalize",
                        SEV_COLORS[entry.severity] || SEV_COLORS.info
                      )}
                    >
                      {entry.severity}
                      {entry.cvss_score != null && ` ${entry.cvss_score}`}
                    </span>
                    {hasPoc && (
                      <span className="text-[7px] px-1 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                        PoC
                      </span>
                    )}
                    {hasWiki && (
                      <span className="text-[7px] px-1 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20">
                        Wiki
                      </span>
                    )}
                  </div>
                  <div className="text-[10px] text-foreground/70 truncate mt-0.5">
                    {entry.title}
                  </div>
                  <div className="flex items-center gap-2 mt-0.5">
                    <span className="text-[8px] text-muted-foreground/20">{entry.source}</span>
                    <span className="text-[8px] text-muted-foreground/20">
                      {entry.published.slice(0, 10)}
                    </span>
                  </div>
                </div>
              </div>
            );
          })}
          {intelDisplayCount < displayEntries.length && (
            <button
              type="button"
              onClick={() => setIntelDisplayCount((c) => c + INTEL_PAGE)}
              className="w-full py-2 mt-1 text-[10px] text-accent/60 hover:text-accent hover:bg-accent/5 rounded transition-colors"
            >
              Show more ({displayEntries.length - intelDisplayCount} remaining)
            </button>
          )}
        </>
      )}
      {displayEntries.length >= 10 && !loading && (
        <button
          type="button"
          onClick={handleLoadMore}
          className="w-full py-2 mt-1 text-[10px] text-accent/60 hover:text-accent hover:bg-accent/5 rounded transition-colors"
        >
          {searchQuery.trim()
            ? `Load more results for "${searchQuery.trim()}"...`
            : `Load older CVEs (${loadMorePage * 120}-${(loadMorePage + 1) * 120} days ago)...`}
        </button>
      )}
      {loading && (
        <div className="flex items-center justify-center py-3">
          <Loader2 className="w-3.5 h-3.5 animate-spin text-muted-foreground/30" />
        </div>
      )}
    </div>
  );
}
