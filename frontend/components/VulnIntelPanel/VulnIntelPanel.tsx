import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Bell, Crosshair, History, Loader2, Plus, RefreshCw,
  Search, Trash2, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { CustomSelect } from "@/components/ui/custom-select";
import { getProjectPath } from "@/lib/projects";
import { useTranslation } from "react-i18next";

import {
  type VulnFeed, type VulnEntry, type VulnLink,
  type ViewMode, type DetailTab, type FilterMode,
  type SeverityFilter, type SourceFilter, type TopTab, type DbVulnLinkFull,
  SEV_COLORS, SEV_DOT, dbToVulnLink, getOrCreateLink, WikiPanelEmbed,
} from "./types";
import { VulnDetailView } from "./VulnDetailView";
import { VulnKbTopBar } from "./VulnKbTopBar";
import { PocLibraryView } from "./PocLibraryView";

export function VulnIntelPanel() {
  const { t } = useTranslation();
  const [topTab, setTopTab] = useState<TopTab>("intel");
  const [wikiOpenPath] = useState<string | null>(null);
  const [vulnLinksForPocLib, setVulnLinksForPocLib] = useState<Record<string, VulnLink>>({});
  const [entries, setEntries] = useState<VulnEntry[]>([]);
  const [matchedEntries, setMatchedEntries] = useState<VulnEntry[]>([]);
  const [feeds, setFeeds] = useState<VulnFeed[]>([]);
  const [loading, setLoading] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>("feed");
  const [searchQuery, setSearchQuery] = useState("");
  const [expandedCve, setExpandedCve] = useState<string | null>(null);
  const [detailTab, setDetailTab] = useState<DetailTab>("intel");
  const [showAddFeed, setShowAddFeed] = useState(false);
  const [newFeed, setNewFeed] = useState({ name: "", feed_type: "rss", url: "" });
  const [vulnLinks, setVulnLinks] = useState<Record<string, VulnLink>>({});
  const [filterMode, setFilterMode] = useState<FilterMode>("all");
  const [severityFilter, setSeverityFilter] = useState<SeverityFilter>("all");
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>("all");

  // Load all vuln links from PostgreSQL on mount
  useEffect(() => {
    invoke<Record<string, DbVulnLinkFull>>("vuln_link_get_all")
      .then((dbLinks) => {
        const mapped: Record<string, VulnLink> = {};
        for (const [cveId, dbLink] of Object.entries(dbLinks)) {
          mapped[cveId] = dbToVulnLink(dbLink);
        }
        setVulnLinks(mapped);
        setVulnLinksForPocLib(mapped);
      })
      .catch((e) => console.error("Failed to load vuln links from DB:", e));
  }, []);

  const updateLinks = useCallback((cveId: string, updater: (link: VulnLink) => VulnLink) => {
    setVulnLinks((prev) => {
      const link = getOrCreateLink(prev, cveId);
      const updated = updater(link);
      return { ...prev, [cveId]: updated };
    });
  }, []);

  const loadCached = useCallback(async () => {
    try {
      const cached = await invoke<VulnEntry[]>("intel_get_cached");
      setEntries(Array.isArray(cached) ? cached : []);
    } catch { /* ignore */ }
  }, []);

  const loadFeeds = useCallback(async () => {
    try {
      const f = await invoke<VulnFeed[]>("intel_list_feeds");
      setFeeds(Array.isArray(f) ? f : []);
    } catch { /* ignore */ }
  }, []);

  useEffect(() => {
    loadCached();
    loadFeeds();
  }, [loadCached, loadFeeds]);

  const handleFetch = useCallback(async () => {
    setLoading(true);
    try {
      const result = await invoke<VulnEntry[]>("intel_fetch");
      setEntries(Array.isArray(result) ? result : []);
      loadFeeds();
    } catch (e) {
      console.error("Fetch failed:", e);
    }
    setLoading(false);
  }, [loadFeeds]);

  const [loadMorePage, setLoadMorePage] = useState(1);
  const [searchOffset, setSearchOffset] = useState(0);

  const SEARCH_HISTORY_KEY = "golish-vuln-search-history";
  const MAX_SEARCH_HISTORY = 20;
  const [searchHistory, setSearchHistory] = useState<string[]>(() => {
    try { return JSON.parse(localStorage.getItem(SEARCH_HISTORY_KEY) || "[]"); } catch { return []; }
  });
  const [showSearchHistory, setShowSearchHistory] = useState(false);
  const searchHistoryRef = useRef<HTMLDivElement>(null);

  const addToSearchHistory = useCallback((query: string) => {
    setSearchHistory((prev) => {
      const next = [query, ...prev.filter((q) => q !== query)].slice(0, MAX_SEARCH_HISTORY);
      try { localStorage.setItem(SEARCH_HISTORY_KEY, JSON.stringify(next)); } catch { /* ignore */ }
      return next;
    });
  }, []);

  const removeFromSearchHistory = useCallback((query: string) => {
    setSearchHistory((prev) => {
      const next = prev.filter((q) => q !== query);
      try { localStorage.setItem(SEARCH_HISTORY_KEY, JSON.stringify(next)); } catch { /* ignore */ }
      return next;
    });
  }, []);

  const clearSearchHistory = useCallback(() => {
    setSearchHistory([]);
    localStorage.removeItem(SEARCH_HISTORY_KEY);
  }, []);

  useEffect(() => {
    if (!showSearchHistory) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (searchHistoryRef.current && !searchHistoryRef.current.contains(e.target as Node)) {
        setShowSearchHistory(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [showSearchHistory]);

  const handleLoadMore = useCallback(async () => {
    setLoading(true);
    try {
      if (searchQuery.trim()) {
        const nextOffset = searchOffset + 50;
        const moreResults = await invoke<VulnEntry[]>("intel_search_remote_page", {
          query: searchQuery.trim(),
          startIndex: nextOffset,
        });
        setEntries((prev) => {
          const ids = new Set(prev.map((e) => e.cve_id));
          return [...prev, ...moreResults.filter((r) => !ids.has(r.cve_id))];
        });
        setSearchOffset(nextOffset);
      } else {
        const result = await invoke<VulnEntry[]>("intel_fetch_page", { page: loadMorePage });
        setEntries(Array.isArray(result) ? result : []);
        setLoadMorePage((p) => p + 1);
      }
    } catch (e) {
      console.error("Load more failed:", e);
    }
    setLoading(false);
  }, [loadMorePage, searchQuery, searchOffset]);

  const handleSearch = useCallback(async () => {
    if (!searchQuery.trim()) {
      loadCached();
      setSearchOffset(0);
      return;
    }
    const q = searchQuery.trim();
    addToSearchHistory(q);
    setShowSearchHistory(false);
    setLoading(true);
    setSearchOffset(0);
    try {
      const localResults = await invoke<VulnEntry[]>("intel_search", { query: q });
      const safeLocalResults = Array.isArray(localResults) ? localResults : [];
      if (safeLocalResults.length > 0) {
        setEntries(safeLocalResults);
      } else {
        const remoteResults = await invoke<VulnEntry[]>("intel_search_remote", { query: q });
        setEntries(Array.isArray(remoteResults) ? remoteResults : []);
      }
    } catch { /* ignore */ }
    setLoading(false);
  }, [searchQuery, loadCached]);

  const handleMatchTargets = useCallback(async () => {
    setLoading(true);
    try {
      const matched = await invoke<VulnEntry[]>("intel_match_targets", {
        projectPath: getProjectPath(),
      });
      setMatchedEntries(matched);
      setViewMode("matched");
    } catch (e) {
      console.error("Match failed:", e);
    }
    setLoading(false);
  }, []);

  const handleAddFeed = useCallback(async () => {
    if (!newFeed.name.trim() || !newFeed.url.trim()) return;
    try {
      await invoke("intel_add_feed", {
        name: newFeed.name.trim(),
        feedType: newFeed.feed_type,
        url: newFeed.url.trim(),
      });
      setNewFeed({ name: "", feed_type: "rss", url: "" });
      setShowAddFeed(false);
      loadFeeds();
    } catch { /* ignore */ }
  }, [newFeed, loadFeeds]);

  const handleToggleFeed = useCallback(async (id: string, enabled: boolean) => {
    await invoke("intel_toggle_feed", { id, enabled });
    loadFeeds();
  }, [loadFeeds]);

  const handleDeleteFeed = useCallback(async (id: string) => {
    await invoke("intel_delete_feed", { id });
    loadFeeds();
  }, [loadFeeds]);

  const baseEntries = viewMode === "matched" ? (matchedEntries ?? []) : (entries ?? []);

  const displayEntries = useMemo(() => {
    let filtered = baseEntries;
    if (sourceFilter !== "all") {
      filtered = filtered.filter((e) => {
        const id = (e.cve_id || "").toUpperCase();
        switch (sourceFilter) {
          case "cve": return id.startsWith("CVE-");
          case "cnvd": return id.startsWith("CNVD-") || id.startsWith("CNNVD-");
          case "other": return !id.startsWith("CVE-") && !id.startsWith("CNVD-") && !id.startsWith("CNNVD-");
          default: return true;
        }
      });
    }
    if (severityFilter !== "all") {
      filtered = filtered.filter((e) => (e.severity || "").toLowerCase() === severityFilter);
    }
    if (filterMode !== "all") {
      filtered = filtered.filter((e) => {
        const link = vulnLinks[e.cve_id];
        switch (filterMode) {
          case "has-poc": return link && link.pocTemplates.length > 0;
          case "has-wiki": return link && link.wikiPaths.length > 0;
          case "no-poc": return !link || link.pocTemplates.length === 0;
          default: return true;
        }
      });
    }
    return filtered;
  }, [baseEntries, filterMode, severityFilter, sourceFilter, vulnLinks]);

  const INTEL_PAGE = 200;
  const [intelDisplayCount, setIntelDisplayCount] = useState(INTEL_PAGE);
  useEffect(() => { setIntelDisplayCount(INTEL_PAGE); }, [displayEntries]);
  const displayedEntries = useMemo(() => displayEntries.slice(0, intelDisplayCount), [displayEntries, intelDisplayCount]);

  const pocCount = useMemo(() => Object.values(vulnLinks).reduce((acc, l) => acc + l.pocTemplates.length, 0), [vulnLinks]);
  const wikiCount = useMemo(() => Object.values(vulnLinks).reduce((acc, l) => acc + l.wikiPaths.length, 0), [vulnLinks]);

  const handleJumpToCve = useCallback((cveId: string) => {
    setTopTab("intel");
    setExpandedCve(cveId);
    setDetailTab("intel");
  }, []);

  const sevStats = useMemo(() => {
    const counts: Record<string, number> = { critical: 0, high: 0, medium: 0, low: 0, info: 0 };
    for (const e of displayEntries) counts[e.severity] = (counts[e.severity] || 0) + 1;
    return counts;
  }, [displayEntries]);

  const sourceStats = useMemo(() => {
    let cve = 0, cnvd = 0, other = 0;
    for (const e of baseEntries) {
      const id = (e.cve_id || "").toUpperCase();
      if (id.startsWith("CVE-")) cve++;
      else if (id.startsWith("CNVD-") || id.startsWith("CNNVD-")) cnvd++;
      else other++;
    }
    return { cve, cnvd, other };
  }, [baseEntries]);

  const selectedEntry = useMemo(
    () => displayEntries.find((e) => e.cve_id === expandedCve) || null,
    [displayEntries, expandedCve]
  );

  const [visitedTabs] = useState(() => new Set<TopTab>(["intel"]));
  if (!visitedTabs.has(topTab)) visitedTabs.add(topTab);

  return (
    <div className="h-full flex flex-col">
      <VulnKbTopBar activeTab={topTab} onTabChange={setTopTab} />

      {/* Wiki view - lazy mounted, hidden when not active */}
      {visitedTabs.has("wiki") && (
        <div className={cn("flex-1 overflow-hidden min-h-0", topTab !== "wiki" && "hidden")}>
          <Suspense fallback={<div className="h-full flex items-center justify-center"><Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" /></div>}>
            <WikiPanelEmbed initialPath={wikiOpenPath} />
          </Suspense>
        </div>
      )}

      {/* PocLibraryView - lazy mounted, hidden when not active */}
      {visitedTabs.has("poc-library") && (
        <div className={cn("flex-1 overflow-hidden min-h-0", topTab !== "poc-library" && "hidden")}>
          <PocLibraryView vulnLinks={vulnLinksForPocLib} onLinksChange={setVulnLinksForPocLib} onJumpToCve={handleJumpToCve} />
        </div>
      )}

      {/* Intel content - hidden when wiki or poc-library active */}
      <div className={cn(topTab !== "intel" && "hidden")}>
        <>

      {/* Stats overview bar */}
      <div className="flex items-center gap-3 px-3 py-1.5 border-b border-border/15 flex-shrink-0">
        <span className="text-[9px] text-muted-foreground/40">
          {displayEntries.length} {viewMode === "matched" ? "matched" : "entries"}
        </span>
        <div className="flex items-center gap-1.5">
          {(["critical", "high", "medium", "low", "info"] as const).map((sev) =>
            sevStats[sev] > 0 ? (
              <span key={sev} className={cn("text-[8px] px-1.5 py-0.5 rounded-full border capitalize", SEV_COLORS[sev])}>
                {sevStats[sev]} {sev}
              </span>
            ) : null
          )}
        </div>
        <div className="w-px h-3 bg-border/15" />
        <span className="text-[8px] text-muted-foreground/30">
          {pocCount} PoC · {wikiCount} Wiki
        </span>
        <div className="w-px h-3 bg-border/15" />
        <span className="text-[8px] text-muted-foreground/30">
          {sourceStats.cve} CVE · {sourceStats.cnvd} CNVD{sourceStats.other > 0 ? ` · ${sourceStats.other} other` : ""}
        </span>
        {displayEntries.length > 0 && (
          <>
            <div className="w-px h-3 bg-border/15" />
            <span className="text-[8px] text-emerald-400/50">
              {Math.round((pocCount / Math.max(displayEntries.length, 1)) * 100)}% PoC coverage
            </span>
          </>
        )}
      </div>

      {/* Tab bar + actions */}
      <div className="flex items-center gap-1 px-3 py-1.5 border-b border-border/10 flex-shrink-0">
        <button
          onClick={() => setViewMode("feed")}
          className={cn("text-[9px] px-2 py-0.5 rounded transition-colors", viewMode === "feed" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground")}
        >
          <Bell className="w-2.5 h-2.5 inline mr-1" />
          Feed
        </button>
        <button
          onClick={handleMatchTargets}
          className={cn("text-[9px] px-2 py-0.5 rounded transition-colors", viewMode === "matched" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground")}
        >
          <Crosshair className="w-2.5 h-2.5 inline mr-1" />
          Match Targets
        </button>
        <button
          onClick={() => setViewMode("feeds-config")}
          className={cn("text-[9px] px-2 py-0.5 rounded transition-colors", viewMode === "feeds-config" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground")}
        >
          Feeds
        </button>

        <div className="w-px h-3.5 bg-border/15 mx-1" />

        <CustomSelect
          value={severityFilter}
          onChange={(v) => setSeverityFilter(v as SeverityFilter)}
          options={[
            { value: "all", label: t("vulnIntel.severityAll", "All Severity") },
            { value: "critical", label: t("vulnIntel.severityCritical", "Critical") },
            { value: "high", label: t("vulnIntel.severityHigh", "High") },
            { value: "medium", label: t("vulnIntel.severityMedium", "Medium") },
            { value: "low", label: t("vulnIntel.severityLow", "Low") },
            { value: "info", label: t("vulnIntel.severityInfo", "Info") },
          ]}
          size="xs"
          className="min-w-[80px]"
        />

        <CustomSelect
          value={filterMode}
          onChange={(v) => setFilterMode(v as FilterMode)}
          options={[
            { value: "all", label: t("vulnIntel.filterAll", "All") },
            { value: "has-poc", label: t("vulnIntel.filterHasPoc", "Has PoC") },
            { value: "has-wiki", label: t("vulnIntel.filterHasWiki", "Has Wiki") },
            { value: "no-poc", label: t("vulnIntel.filterNoPoc", "No PoC") },
          ]}
          size="xs"
          className="min-w-[60px]"
        />

        <CustomSelect
          value={sourceFilter}
          onChange={(v) => setSourceFilter(v as SourceFilter)}
          options={[
            { value: "all", label: t("vulnIntel.sourceAll", "All Sources") },
            { value: "cve", label: "CVE" },
            { value: "cnvd", label: "CNVD / CNNVD" },
            { value: "other", label: t("vulnIntel.sourceOther", "Other") },
          ]}
          size="xs"
          className="min-w-[80px]"
        />

        <div className="flex-1" />
        <div className="relative" ref={searchHistoryRef}>
          <div className="flex items-center gap-1 bg-background border border-border/20 rounded px-1.5">
            <Search className="w-2.5 h-2.5 text-muted-foreground/30" />
            <input
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleSearch()}
              onFocus={() => searchHistory.length > 0 && setShowSearchHistory(true)}
              placeholder={t("vulnIntel.searchPlaceholder", "Search CVEs...")}
              className="text-[10px] py-0.5 bg-transparent outline-none w-28"
            />
            {searchQuery && (
              <button onClick={() => { setSearchQuery(""); loadCached(); setShowSearchHistory(false); }} className="text-muted-foreground/30 hover:text-foreground">
                <X className="w-2.5 h-2.5" />
              </button>
            )}
            <button
              onClick={() => setShowSearchHistory((v) => !v)}
              className={cn(
                "p-0.5 rounded transition-colors",
                showSearchHistory ? "text-accent" : "text-muted-foreground/30 hover:text-muted-foreground/60"
              )}
              title={t("vulnIntel.searchHistory", "Search history")}
            >
              <History className="w-2.5 h-2.5" />
            </button>
          </div>

          {showSearchHistory && searchHistory.length > 0 && (
            <div className="absolute right-0 top-full mt-1 z-50 w-64 max-h-60 overflow-y-auto rounded-lg border border-border/20 bg-background shadow-xl">
              <div className="flex items-center justify-between px-3 py-1.5 border-b border-border/10">
                <span className="text-[10px] text-muted-foreground/50">{t("vulnIntel.recentSearches", "Recent searches")}</span>
                <button
                  onClick={clearSearchHistory}
                  className="text-[9px] text-muted-foreground/30 hover:text-destructive transition-colors"
                >
                  {t("vulnIntel.clearHistory", "Clear")}
                </button>
              </div>
              {searchHistory.map((q) => (
                <div
                  key={q}
                  className="group flex items-center gap-2 px-3 py-1.5 hover:bg-accent/5 cursor-pointer transition-colors"
                  onClick={() => { setSearchQuery(q); setShowSearchHistory(false); }}
                >
                  <History className="w-2.5 h-2.5 text-muted-foreground/25 flex-shrink-0" />
                  <span className="text-[10px] text-foreground/70 truncate flex-1">{q}</span>
                  <button
                    onClick={(e) => { e.stopPropagation(); removeFromSearchHistory(q); }}
                    className="p-0.5 rounded text-muted-foreground/0 group-hover:text-muted-foreground/30 hover:!text-destructive transition-all"
                  >
                    <X className="w-2.5 h-2.5" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
        <button
          onClick={handleFetch}
          disabled={loading}
          className="p-1 text-muted-foreground/30 hover:text-accent transition-colors"
        >
          <RefreshCw className={cn("w-3 h-3", loading && "animate-spin")} />
        </button>
      </div>

      {/* Feeds config view */}
      {viewMode === "feeds-config" ? (
        <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
          {(feeds ?? []).map((feed) => (
            <div key={feed.id} className="flex items-center gap-2 py-1.5 px-2 rounded hover:bg-muted/5 group">
              <input
                type="checkbox"
                checked={feed.enabled}
                onChange={() => handleToggleFeed(feed.id, !feed.enabled)}
                className="w-3 h-3 accent-accent"
              />
              <div className="flex-1 min-w-0">
                <div className="text-[10px] font-medium truncate">{feed.name}</div>
                <div className="text-[9px] text-muted-foreground/30 truncate">{feed.url}</div>
                {feed.last_fetched && (
                  <div className="text-[8px] text-muted-foreground/20">
                    Last fetched: {new Date(feed.last_fetched * 1000).toLocaleString()}
                  </div>
                )}
              </div>
              <span className="text-[8px] text-muted-foreground/25 px-1.5 py-0.5 bg-muted/10 rounded">
                {feed.feed_type}
              </span>
              <button
                onClick={() => handleDeleteFeed(feed.id)}
                className="p-1 text-muted-foreground/20 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-all"
              >
                <Trash2 className="w-3 h-3" />
              </button>
            </div>
          ))}

          {showAddFeed ? (
            <div className="space-y-1.5 p-2 border border-border/20 rounded">
              <input
                value={newFeed.name}
                onChange={(e) => setNewFeed((f) => ({ ...f, name: e.target.value }))}
                placeholder="Feed name..."
                className="w-full text-[10px] px-2 py-1 bg-background border border-border/30 rounded outline-none"
              />
              <CustomSelect
                value={newFeed.feed_type}
                onChange={(v) => setNewFeed((f) => ({ ...f, feed_type: v }))}
                options={[
                  { value: "rss", label: "RSS / Atom Feed" },
                  { value: "nvd", label: "NVD API" },
                  { value: "cisa_kev", label: "CISA KEV" },
                  { value: "custom", label: "Custom JSON" },
                ]}
                size="sm"
              />
              <input
                value={newFeed.url}
                onChange={(e) => setNewFeed((f) => ({ ...f, url: e.target.value }))}
                placeholder="Feed URL..."
                className="w-full text-[10px] px-2 py-1 bg-background border border-border/30 rounded outline-none"
              />
              <div className="flex gap-1.5">
                <button onClick={handleAddFeed} disabled={!newFeed.name.trim() || !newFeed.url.trim()}
                  className="text-[9px] text-accent hover:text-accent/80 font-medium disabled:opacity-30">
                  Add
                </button>
                <button onClick={() => setShowAddFeed(false)} className="text-[9px] text-muted-foreground/30">Cancel</button>
              </div>
            </div>
          ) : (
            <button
              onClick={() => setShowAddFeed(true)}
              className="flex items-center gap-1 text-[9px] text-muted-foreground/30 hover:text-accent transition-colors"
            >
              <Plus className="w-3 h-3" />
              Add feed
            </button>
          )}
        </div>
      ) : (
        /* Master-detail layout */
        <div className="flex-1 flex overflow-hidden min-h-0">
          {/* Left: CVE list */}
          <div className={cn(
            "flex-shrink-0 flex flex-col border-r border-border/10 overflow-hidden",
            selectedEntry ? "w-[320px]" : "flex-1"
          )}>
            <div className="flex-1 overflow-y-auto py-1 px-1.5">
              {displayEntries.length === 0 ? (
                <div className="text-center text-[11px] text-muted-foreground/30 py-12">
                  {loading ? t("vulnIntel.fetching", "Fetching vulnerability data...") : viewMode === "matched" ? t("vulnIntel.noMatched", "No matched vulnerabilities") : t("vulnIntel.clickRefresh", "Click refresh to fetch latest CVEs")}
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
                        isSelected ? "bg-accent/10 border border-accent/20" : "hover:bg-muted/5 border border-transparent"
                      )}
                      onClick={() => { setExpandedCve(isSelected ? null : entry.cve_id); setDetailTab("intel"); }}
                    >
                      <span className={cn("w-1.5 h-1.5 rounded-full mt-1.5 flex-shrink-0", SEV_DOT[entry.severity] || "bg-slate-500")} />
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-1.5 flex-wrap">
                          <span className="text-[10px] font-mono font-medium text-accent/80">{entry.cve_id}</span>
                          <span className={cn("text-[8px] px-1.5 py-0.5 rounded-full border capitalize",
                            SEV_COLORS[entry.severity] || SEV_COLORS.info
                          )}>
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
                        <div className="text-[10px] text-foreground/70 truncate mt-0.5">{entry.title}</div>
                        <div className="flex items-center gap-2 mt-0.5">
                          <span className="text-[8px] text-muted-foreground/20">{entry.source}</span>
                          <span className="text-[8px] text-muted-foreground/20">{entry.published.slice(0, 10)}</span>
                        </div>
                      </div>
                    </div>
                  );
                })}
                {intelDisplayCount < displayEntries.length && (
                  <button
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
          </div>

          {/* Right: Detail panel */}
          {selectedEntry && (
            <div className="flex-1 flex flex-col overflow-hidden min-w-0">
              <VulnDetailView
                entry={selectedEntry}
                link={vulnLinks[selectedEntry.cve_id] || { wikiPaths: [], pocTemplates: [], scanHistory: [] }}
                detailTab={detailTab}
                onTabChange={setDetailTab}
                onUpdateLink={(updater) => updateLinks(selectedEntry.cve_id, updater)}
              />
            </div>
          )}
        </div>
      )}
        </>
      </div>
    </div>
  );
}
