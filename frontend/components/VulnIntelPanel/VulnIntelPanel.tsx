import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  Bell,
  BookOpen,
  Code,
  Copy,
  ExternalLink,
  FileCode2,
  FileText,
  Link2,
  Plus,
  RefreshCw,
  Search,
  Shield,
  Trash2,
  X,
  Crosshair,
  Zap,
  History,
  ChevronDown,
  ChevronRight,
  Loader2,
  Play,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import { useTranslation } from "react-i18next";

interface VulnFeed {
  id: string;
  name: string;
  feed_type: string;
  url: string;
  enabled: boolean;
  last_fetched: number | null;
}

interface VulnEntry {
  cve_id: string;
  title: string;
  description: string;
  severity: string;
  cvss_score: number | null;
  published: string;
  source: string;
  references: string[];
  affected_products: string[];
}

interface PocTemplate {
  id: string;
  name: string;
  type: "nuclei" | "script" | "manual";
  language: string;
  content: string;
  created: number;
}

interface VulnLink {
  wikiPaths: string[];
  pocTemplates: PocTemplate[];
  scanHistory: ScanHistoryEntry[];
}

interface ScanHistoryEntry {
  target: string;
  date: number;
  result: "vulnerable" | "not_vulnerable" | "error" | "pending";
  details?: string;
}

const SEV_COLORS: Record<string, string> = {
  critical: "text-red-400 bg-red-500/10 border-red-500/20",
  high: "text-orange-400 bg-orange-500/10 border-orange-500/20",
  medium: "text-yellow-400 bg-yellow-500/10 border-yellow-500/20",
  low: "text-blue-400 bg-blue-500/10 border-blue-500/20",
  info: "text-slate-400 bg-slate-500/10 border-slate-500/20",
};

const SEV_DOT: Record<string, string> = {
  critical: "bg-red-500",
  high: "bg-orange-500",
  medium: "bg-yellow-500",
  low: "bg-blue-500",
  info: "bg-slate-500",
};

const VULN_LINKS_KEY = "golish-vuln-links";

function loadVulnLinks(): Record<string, VulnLink> {
  try { return JSON.parse(localStorage.getItem(VULN_LINKS_KEY) || "{}"); } catch { return {}; }
}

function saveVulnLinks(links: Record<string, VulnLink>) {
  try { localStorage.setItem(VULN_LINKS_KEY, JSON.stringify(links)); } catch { /* ignore */ }
}

function getOrCreateLink(links: Record<string, VulnLink>, cveId: string): VulnLink {
  return links[cveId] || { wikiPaths: [], pocTemplates: [], scanHistory: [] };
}

type ViewMode = "feed" | "matched" | "feeds-config";
type DetailTab = "intel" | "wiki" | "poc" | "history";
type FilterMode = "all" | "has-poc" | "has-wiki" | "no-poc";
type SeverityFilter = "all" | "critical" | "high" | "medium" | "low" | "info";
type TopTab = "intel" | "wiki" | "poc-library";

import { lazy, Suspense } from "react";
const WikiPanelEmbed = lazy(() =>
  import("@/components/WikiPanel/WikiPanel").then((m) => ({ default: m.WikiPanel }))
);

export function VulnIntelPanel() {
  const { t } = useTranslation();
  const [topTab, setTopTab] = useState<TopTab>("intel");
  const [wikiOpenPath, setWikiOpenPath] = useState<string | null>(null);
  const [vulnLinksForPocLib, setVulnLinksForPocLib] = useState<Record<string, VulnLink>>(loadVulnLinks);
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
  const [vulnLinks, setVulnLinks] = useState<Record<string, VulnLink>>(loadVulnLinks);
  const [filterMode, setFilterMode] = useState<FilterMode>("all");
  const [severityFilter, setSeverityFilter] = useState<SeverityFilter>("all");

  const updateLinks = useCallback((cveId: string, updater: (link: VulnLink) => VulnLink) => {
    setVulnLinks((prev) => {
      const link = getOrCreateLink(prev, cveId);
      const next = { ...prev, [cveId]: updater(link) };
      saveVulnLinks(next);
      return next;
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
  }, [baseEntries, filterMode, severityFilter, vulnLinks]);

  const pocCount = useMemo(() => Object.values(vulnLinks).reduce((acc, l) => acc + l.pocTemplates.length, 0), [vulnLinks]);
  const wikiCount = useMemo(() => Object.values(vulnLinks).reduce((acc, l) => acc + l.wikiPaths.length, 0), [vulnLinks]);

  const handleJumpToCve = useCallback((cveId: string) => {
    setTopTab("intel");
    setExpandedCve(cveId);
    setDetailTab("intel");
  }, []);

  const handleOpenWikiFile = useCallback((path: string) => {
    setWikiOpenPath(path);
    setTopTab("wiki");
  }, []);

  const sevStats = useMemo(() => {
    const counts: Record<string, number> = { critical: 0, high: 0, medium: 0, low: 0, info: 0 };
    for (const e of displayEntries) counts[e.severity] = (counts[e.severity] || 0) + 1;
    return counts;
  }, [displayEntries]);

  const selectedEntry = useMemo(
    () => displayEntries.find((e) => e.cve_id === expandedCve) || null,
    [displayEntries, expandedCve]
  );

  if (topTab === "wiki") {
    return (
      <div className="h-full flex flex-col bg-background/95">
        <VulnKbTopBar activeTab={topTab} onTabChange={setTopTab} />
        <div className="flex-1 overflow-hidden min-h-0">
          <Suspense fallback={<div className="h-full flex items-center justify-center"><Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" /></div>}>
            <WikiPanelEmbed initialPath={wikiOpenPath} />
          </Suspense>
        </div>
      </div>
    );
  }

  if (topTab === "poc-library") {
    return (
      <div className="h-full flex flex-col bg-background/95">
        <VulnKbTopBar activeTab={topTab} onTabChange={setTopTab} />
        <div className="flex-1 overflow-hidden min-h-0">
          <PocLibraryView vulnLinks={vulnLinksForPocLib} onLinksChange={setVulnLinksForPocLib} onJumpToCve={handleJumpToCve} />
        </div>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col bg-background/95">
      <VulnKbTopBar activeTab={topTab} onTabChange={setTopTab} />

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

        <select
          value={severityFilter}
          onChange={(e) => setSeverityFilter(e.target.value as SeverityFilter)}
          className="text-[9px] px-1.5 py-0.5 rounded bg-transparent border border-border/15 text-muted-foreground/50 outline-none cursor-pointer"
        >
          <option value="all">{t("vulnIntel.severityAll", "All Severity")}</option>
          <option value="critical">{t("vulnIntel.severityCritical", "Critical")}</option>
          <option value="high">{t("vulnIntel.severityHigh", "High")}</option>
          <option value="medium">{t("vulnIntel.severityMedium", "Medium")}</option>
          <option value="low">{t("vulnIntel.severityLow", "Low")}</option>
          <option value="info">{t("vulnIntel.severityInfo", "Info")}</option>
        </select>

        <select
          value={filterMode}
          onChange={(e) => setFilterMode(e.target.value as FilterMode)}
          className="text-[9px] px-1.5 py-0.5 rounded bg-transparent border border-border/15 text-muted-foreground/50 outline-none cursor-pointer"
        >
          <option value="all">{t("vulnIntel.filterAll", "All")}</option>
          <option value="has-poc">{t("vulnIntel.filterHasPoc", "Has PoC")}</option>
          <option value="has-wiki">{t("vulnIntel.filterHasWiki", "Has Wiki")}</option>
          <option value="no-poc">{t("vulnIntel.filterNoPoc", "No PoC")}</option>
        </select>

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
              <select
                value={newFeed.feed_type}
                onChange={(e) => setNewFeed((f) => ({ ...f, feed_type: e.target.value }))}
                className="w-full text-[10px] px-2 py-1 bg-background border border-border/30 rounded outline-none"
              >
                <option value="rss">RSS / Atom Feed</option>
                <option value="nvd">NVD API</option>
                <option value="cisa_kev">CISA KEV</option>
                <option value="custom">Custom JSON</option>
              </select>
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
                displayEntries.map((entry) => {
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
                })
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
                onOpenWikiFile={handleOpenWikiFile}
              />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function VulnDetailView({
  entry,
  link,
  detailTab,
  onTabChange,
  onUpdateLink,
  onOpenWikiFile,
}: {
  entry: VulnEntry;
  link: VulnLink;
  detailTab: DetailTab;
  onTabChange: (tab: DetailTab) => void;
  onUpdateLink: (updater: (link: VulnLink) => VulnLink) => void;
  onOpenWikiFile?: (path: string) => void;
}) {
  return (
    <>
      {/* Detail header */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border/10 flex-shrink-0">
        <span className={cn("w-2 h-2 rounded-full flex-shrink-0", SEV_DOT[entry.severity] || "bg-slate-500")} />
        <span className="text-[12px] font-mono font-semibold text-accent">{entry.cve_id}</span>
        <span className={cn("text-[9px] px-2 py-0.5 rounded-full border capitalize", SEV_COLORS[entry.severity] || SEV_COLORS.info)}>
          {entry.severity}
          {entry.cvss_score != null && ` ${entry.cvss_score}`}
        </span>
        <span className="text-[9px] text-muted-foreground/25">{entry.source}</span>
      </div>

      {/* Detail tabs */}
      <div className="flex items-center gap-0.5 px-3 py-1.5 border-b border-border/10 bg-muted/3 flex-shrink-0">
        {([
          { id: "intel" as const, icon: Shield, label: "Intel" },
          { id: "wiki" as const, icon: BookOpen, label: `Wiki${link.wikiPaths.length > 0 ? ` (${link.wikiPaths.length})` : ""}` },
          { id: "poc" as const, icon: Code, label: `PoC${link.pocTemplates.length > 0 ? ` (${link.pocTemplates.length})` : ""}` },
          { id: "history" as const, icon: History, label: `History${link.scanHistory.length > 0 ? ` (${link.scanHistory.length})` : ""}` },
        ]).map((tab) => (
          <button
            key={tab.id}
            onClick={() => onTabChange(tab.id)}
            className={cn(
              "flex items-center gap-1 px-2.5 py-1 rounded text-[10px] transition-colors",
              detailTab === tab.id ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground"
            )}
          >
            <tab.icon className="w-3 h-3" />
            {tab.label}
          </button>
        ))}
      </div>

      {/* Detail content */}
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {detailTab === "intel" && <IntelTab entry={entry} />}
        {detailTab === "wiki" && <WikiTab link={link} cveId={entry.cve_id} onUpdateLink={onUpdateLink} onOpenFile={onOpenWikiFile} />}
        {detailTab === "poc" && <PocTab link={link} cveId={entry.cve_id} onUpdateLink={onUpdateLink} />}
        {detailTab === "history" && <HistoryTab link={link} />}
      </div>
    </>
  );
}

function IntelTab({ entry }: { entry: VulnEntry }) {
  return (
    <div className="space-y-2.5">
      {/* Meta grid */}
      <div className="grid grid-cols-2 gap-x-4 gap-y-1.5">
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">CVE ID</span>
          <div className="text-[10px] font-mono text-accent">{entry.cve_id}</div>
        </div>
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">CVSS Score</span>
          <div className="flex items-center gap-1.5 mt-0.5">
            {entry.cvss_score != null ? (
              <span className={cn(
                "text-[11px] font-bold",
                entry.cvss_score >= 9 ? "text-red-400" :
                entry.cvss_score >= 7 ? "text-orange-400" :
                entry.cvss_score >= 4 ? "text-yellow-400" : "text-blue-400"
              )}>
                {entry.cvss_score.toFixed(1)}
              </span>
            ) : null}
            <span className={cn("text-[8px] px-1.5 py-0.5 rounded border capitalize",
              SEV_COLORS[entry.severity] || SEV_COLORS.info
            )}>{entry.severity}</span>
          </div>
        </div>
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Published</span>
          <div className="text-[10px] text-foreground/60">{entry.published ? new Date(entry.published).toLocaleDateString("zh-CN", { year: "numeric", month: "long", day: "numeric" }) : "Unknown"}</div>
        </div>
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Source</span>
          <div className="text-[10px] text-foreground/60">{entry.source || "N/A"}</div>
        </div>
      </div>

      {/* Description */}
      <div>
        <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Description</span>
        <p className="text-[10px] text-foreground/60 leading-relaxed mt-0.5">{entry.description}</p>
      </div>

      {/* Affected Products */}
      {entry.affected_products.length > 0 && (
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Affected Products</span>
          <div className="flex flex-wrap gap-1 mt-0.5">
            {entry.affected_products.map((prod, i) => (
              <span key={i} className="text-[9px] px-1.5 py-0.5 bg-orange-500/10 text-orange-400 border border-orange-500/20 rounded">
                {prod}
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Quick links */}
      <div className="flex items-center gap-3 pt-1 border-t border-border/10">
        <a href={`https://nvd.nist.gov/vuln/detail/${entry.cve_id}`} target="_blank" rel="noopener noreferrer"
          className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors">
          <ExternalLink className="w-2.5 h-2.5" /> NVD
        </a>
        <a href={`https://cve.mitre.org/cgi-bin/cvename.cgi?name=${entry.cve_id}`} target="_blank" rel="noopener noreferrer"
          className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors">
          <ExternalLink className="w-2.5 h-2.5" /> MITRE
        </a>
        <a href={`https://www.google.com/search?q=${entry.cve_id}+exploit+poc`} target="_blank" rel="noopener noreferrer"
          className="flex items-center gap-1 text-[9px] text-red-400/60 hover:text-red-400 transition-colors">
          <AlertTriangle className="w-2.5 h-2.5" /> Search PoC
        </a>
        <a href={`https://github.com/search?q=${entry.cve_id}&type=repositories`} target="_blank" rel="noopener noreferrer"
          className="flex items-center gap-1 text-[9px] text-muted-foreground/40 hover:text-foreground transition-colors">
          <ExternalLink className="w-2.5 h-2.5" /> GitHub
        </a>
      </div>

      {/* References */}
      {entry.references.length > 0 && (
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">References ({entry.references.length})</span>
          <div className="space-y-0.5 mt-0.5 max-h-24 overflow-y-auto">
            {entry.references.map((ref_, i) => (
              <a key={i} href={ref_} target="_blank" rel="noopener noreferrer"
                className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors truncate">
                <ExternalLink className="w-2.5 h-2.5 flex-shrink-0" />
                {ref_}
              </a>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function WikiTab({ link, cveId, onUpdateLink, onOpenFile }: { link: VulnLink; cveId: string; onUpdateLink: (updater: (l: VulnLink) => VulnLink) => void; onOpenFile?: (path: string) => void }) {
  const { t } = useTranslation();
  const [adding, setAdding] = useState(false);
  const [newPath, setNewPath] = useState("");
  const [wikiTree, setWikiTree] = useState<{ path: string; name: string; is_dir: boolean }[]>([]);
  const [loadingTree, setLoadingTree] = useState(false);

  const loadWikiFiles = useCallback(async () => {
    setLoadingTree(true);
    try {
      const flatFiles: { path: string; name: string; is_dir: boolean }[] = [];
      const flatten = (entries: { path: string; name: string; is_dir: boolean; children?: { path: string; name: string; is_dir: boolean }[] }[]) => {
        for (const e of entries) {
          if (e.is_dir && e.children) flatten(e.children);
          else if (!e.is_dir) flatFiles.push(e);
        }
      };
      const tree = await invoke<{ path: string; name: string; is_dir: boolean; children?: { path: string; name: string; is_dir: boolean }[] }[]>("wiki_list");
      if (Array.isArray(tree)) flatten(tree);
      setWikiTree(flatFiles);
    } catch { /* ignore */ }
    setLoadingTree(false);
  }, []);

  useEffect(() => { if (adding) loadWikiFiles(); }, [adding, loadWikiFiles]);

  const handleAddWiki = useCallback((path: string) => {
    onUpdateLink((l) => ({
      ...l,
      wikiPaths: l.wikiPaths.includes(path) ? l.wikiPaths : [...l.wikiPaths, path],
    }));
    setAdding(false);
    setNewPath("");
  }, [onUpdateLink]);

  const handleRemoveWiki = useCallback((path: string) => {
    onUpdateLink((l) => ({ ...l, wikiPaths: l.wikiPaths.filter((p) => p !== path) }));
  }, [onUpdateLink]);

  const handleCreateWikiArticle = useCallback(async () => {
    const path = `${cveId}/README.md`;
    try {
      await invoke("wiki_create_cve", { cveId, title: cveId, pocLang: null });
      handleAddWiki(path);
    } catch (err) {
      console.error("Failed to create wiki article:", err);
    }
  }, [cveId, handleAddWiki]);

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">
          {t("vulnIntel.linkedWiki", "Linked Wiki Articles")}
        </span>
        <div className="flex items-center gap-1">
          <button onClick={() => setAdding(!adding)}
            className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors">
            <Link2 className="w-2.5 h-2.5" /> {t("vulnIntel.linkArticle", "Link")}
          </button>
          <button onClick={handleCreateWikiArticle}
            className="flex items-center gap-1 text-[9px] text-emerald-400/60 hover:text-emerald-400 transition-colors">
            <Plus className="w-2.5 h-2.5" /> {t("vulnIntel.createArticle", "Create")}
          </button>
        </div>
      </div>

      {adding && (
        <div className="space-y-1.5 p-2 border border-border/15 rounded bg-[var(--bg-hover)]/20">
          <div className="flex items-center gap-1.5">
            <input
              value={newPath}
              onChange={(e) => setNewPath(e.target.value)}
              placeholder={t("vulnIntel.wikiPathPlaceholder", "Wiki file path...")}
              className="flex-1 h-6 px-2 text-[10px] font-mono bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
            />
            <button onClick={() => handleAddWiki(newPath.trim())} disabled={!newPath.trim()}
              className="text-[9px] text-accent hover:text-accent/80 font-medium disabled:opacity-30">
              {t("vulnIntel.link", "Link")}
            </button>
            <button onClick={() => { setAdding(false); setNewPath(""); }}
              className="text-[9px] text-muted-foreground/30">{t("common.cancel")}</button>
          </div>
          {loadingTree ? (
            <div className="text-[9px] text-muted-foreground/30 py-1"><Loader2 className="w-3 h-3 animate-spin inline mr-1" />Loading...</div>
          ) : wikiTree.length > 0 ? (
            <div className="max-h-32 overflow-y-auto space-y-0.5">
              {wikiTree.filter((f) => !link.wikiPaths.includes(f.path)).map((f) => (
                <div key={f.path} onClick={() => handleAddWiki(f.path)}
                  className="flex items-center gap-1.5 px-1.5 py-1 rounded cursor-pointer hover:bg-muted/10 transition-colors">
                  <FileText className="w-3 h-3 text-blue-400/50 flex-shrink-0" />
                  <span className="text-[9px] text-foreground/60 truncate">{f.path}</span>
                </div>
              ))}
            </div>
          ) : (
            <div className="text-[9px] text-muted-foreground/30 py-1">{t("vulnIntel.noWikiFiles", "No wiki files found")}</div>
          )}
        </div>
      )}

      {link.wikiPaths.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-6 gap-2 text-muted-foreground/20">
          <BookOpen className="w-8 h-8" />
          <p className="text-[10px]">{t("vulnIntel.noLinkedWiki", "No wiki articles linked")}</p>
          <p className="text-[9px] text-muted-foreground/15">{t("vulnIntel.linkWikiHint", "Link existing wiki articles or create a new one for this vulnerability")}</p>
        </div>
      ) : (
        <div className="space-y-1">
          {link.wikiPaths.map((path) => (
            <div key={path} className="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-muted/5 transition-colors group cursor-pointer"
              onClick={() => onOpenFile?.(path)}>
              <FileText className="w-3.5 h-3.5 text-blue-400/60 flex-shrink-0" />
              <span className="text-[10px] text-accent/70 hover:text-accent truncate flex-1 font-mono">{path}</span>
              <button onClick={(e) => { e.stopPropagation(); handleRemoveWiki(path); }}
                className="p-0.5 rounded text-muted-foreground/0 group-hover:text-muted-foreground/30 hover:!text-destructive transition-all">
                <X className="w-3 h-3" />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function PocTab({ link, cveId, onUpdateLink }: { link: VulnLink; cveId: string; onUpdateLink: (updater: (l: VulnLink) => VulnLink) => void }) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<PocTemplate | null>(null);
  const [formName, setFormName] = useState("");
  const [formType, setFormType] = useState<PocTemplate["type"]>("nuclei");
  const [formLang, setFormLang] = useState("yaml");
  const [formContent, setFormContent] = useState("");
  const [expandedPoc, setExpandedPoc] = useState<string | null>(null);

  const generateTemplate = useCallback((type: PocTemplate["type"], lang = "python"): { content: string; language: string } => {
    const slug = cveId.toLowerCase().replace(/[^a-z0-9-]/g, "-");
    if (type === "nuclei") {
      return { language: "yaml", content: `id: ${slug}

info:
  name: ${cveId}
  author: golish
  severity: medium
  description: |
    Detection for ${cveId}

http:
  - method: GET
    path:
      - "{{BaseURL}}/"
    matchers:
      - type: status
        status:
          - 200
` };
    }
    if (type === "script") {
      const templates: Record<string, string> = {
        python: `#!/usr/bin/env python3
"""${cveId} PoC - Proof of Concept"""
import requests
import sys

def exploit(target: str):
    """Test target for ${cveId}"""
    url = f"{target.rstrip('/')}/"
    try:
        resp = requests.get(url, timeout=10, verify=False)
        if resp.status_code == 200:
            print(f"[+] {target} may be vulnerable to ${cveId}")
            return True
    except requests.RequestException as e:
        print(f"[-] Error: {e}")
    return False

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <target_url>")
        sys.exit(1)
    exploit(sys.argv[1])
`,
        bash: `#!/bin/bash
# ${cveId} PoC - Proof of Concept

TARGET="\${1:?Usage: $0 <target_url>}"

echo "[*] Testing $TARGET for ${cveId}..."

RESP=$(curl -sk -o /dev/null -w "%{http_code}" "$TARGET/")

if [ "$RESP" = "200" ]; then
    echo "[+] $TARGET may be vulnerable to ${cveId}"
else
    echo "[-] $TARGET does not appear vulnerable (HTTP $RESP)"
fi
`,
        go: `package main

import (
\t"fmt"
\t"net/http"
\t"os"
\t"time"
)

// ${cveId} PoC
func main() {
\tif len(os.Args) < 2 {
\t\tfmt.Fprintf(os.Stderr, "Usage: %s <target_url>\\n", os.Args[0])
\t\tos.Exit(1)
\t}
\ttarget := os.Args[1]
\tclient := &http.Client{Timeout: 10 * time.Second}
\tresp, err := client.Get(target + "/")
\tif err != nil {
\t\tfmt.Printf("[-] Error: %v\\n", err)
\t\tos.Exit(1)
\t}
\tdefer resp.Body.Close()
\tif resp.StatusCode == 200 {
\t\tfmt.Printf("[+] %s may be vulnerable to ${cveId}\\n", target)
\t} else {
\t\tfmt.Printf("[-] %s does not appear vulnerable (HTTP %d)\\n", target, resp.StatusCode)
\t}
}
`,
        javascript: `#!/usr/bin/env node
// ${cveId} PoC - Proof of Concept

const target = process.argv[2];
if (!target) {
  console.error(\`Usage: \${process.argv[1]} <target_url>\`);
  process.exit(1);
}

fetch(\`\${target.replace(/\\/$/, "")}/\`)
  .then((resp) => {
    if (resp.ok) {
      console.log(\`[+] \${target} may be vulnerable to ${cveId}\`);
    } else {
      console.log(\`[-] \${target} does not appear vulnerable (HTTP \${resp.status})\`);
    }
  })
  .catch((err) => console.error(\`[-] Error: \${err.message}\`));
`,
        c: `/* ${cveId} PoC - Proof of Concept */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <curl/curl.h>

static size_t discard_cb(void *ptr, size_t size, size_t nmemb, void *data) {
    (void)ptr; (void)data;
    return size * nmemb;
}

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <target_url>\\n", argv[0]);
        return 1;
    }

    CURL *curl = curl_easy_init();
    if (!curl) {
        fprintf(stderr, "[-] Failed to init curl\\n");
        return 1;
    }

    char url[2048];
    snprintf(url, sizeof(url), "%s/", argv[1]);

    curl_easy_setopt(curl, CURLOPT_URL, url);
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, discard_cb);
    curl_easy_setopt(curl, CURLOPT_TIMEOUT, 10L);
    curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 0L);

    CURLcode res = curl_easy_perform(curl);
    if (res == CURLE_OK) {
        long code;
        curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &code);
        if (code == 200)
            printf("[+] %s may be vulnerable to ${cveId}\\n", argv[1]);
        else
            printf("[-] %s does not appear vulnerable (HTTP %ld)\\n", argv[1], code);
    } else {
        fprintf(stderr, "[-] Error: %s\\n", curl_easy_strerror(res));
    }

    curl_easy_cleanup(curl);
    return 0;
}
`,
      };
      return { language: lang, content: templates[lang] || templates.python };
    }
    return { language: "markdown", content: `# ${cveId} - Manual Testing\n\n## Steps to Reproduce\n\n1. Navigate to the target application\n2. ...\n\n## Expected Result\n\n...\n\n## Actual Result\n\n...\n\n## Impact\n\n...\n` };
  }, [cveId]);

  const handleNewPoc = useCallback(() => {
    const { content, language } = generateTemplate("nuclei");
    setFormName(`${cveId} PoC`);
    setFormType("nuclei");
    setFormLang(language);
    setFormContent(content);
    setEditing({ id: "", name: "", type: "nuclei", language, content: "", created: 0 });
  }, [cveId, generateTemplate]);

  const handleTypeChange = useCallback((newType: PocTemplate["type"]) => {
    setFormType(newType);
    const { content, language } = generateTemplate(newType, newType === "script" ? "python" : undefined);
    setFormLang(language);
    if (!editing?.id) {
      setFormContent(content);
    }
  }, [generateTemplate, editing]);

  const handleLangChange = useCallback((newLang: string) => {
    setFormLang(newLang);
    if (!editing?.id) {
      const { content } = generateTemplate("script", newLang);
      setFormContent(content);
    }
  }, [generateTemplate, editing]);

  const handleEditPoc = useCallback((poc: PocTemplate) => {
    setFormName(poc.name);
    setFormType(poc.type);
    setFormLang(poc.language);
    setFormContent(poc.content);
    setEditing(poc);
  }, []);

  const handleSavePoc = useCallback(() => {
    if (!formName.trim() || !formContent.trim()) return;
    const poc: PocTemplate = {
      id: editing?.id || `poc-${Date.now()}`,
      name: formName.trim(),
      type: formType,
      language: formLang,
      content: formContent,
      created: editing?.created || Date.now(),
    };
    onUpdateLink((l) => {
      const existing = l.pocTemplates.findIndex((p) => p.id === poc.id);
      const templates = existing >= 0 ? l.pocTemplates.map((p) => p.id === poc.id ? poc : p) : [...l.pocTemplates, poc];
      return { ...l, pocTemplates: templates };
    });
    setEditing(null);
    setFormName("");
    setFormContent("");
  }, [editing, formName, formType, formLang, formContent, onUpdateLink]);

  const handleDeletePoc = useCallback((id: string) => {
    onUpdateLink((l) => ({ ...l, pocTemplates: l.pocTemplates.filter((p) => p.id !== id) }));
  }, [onUpdateLink]);

  const handleCopyContent = useCallback((content: string) => {
    navigator.clipboard.writeText(content);
  }, []);

  const typeIcon = (type: PocTemplate["type"]) => {
    if (type === "nuclei") return <Zap className="w-3 h-3 text-orange-400/60" />;
    if (type === "script") return <FileCode2 className="w-3 h-3 text-emerald-400/60" />;
    return <FileText className="w-3 h-3 text-blue-400/60" />;
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">
          {t("vulnIntel.pocTemplates", "PoC Templates")}
        </span>
        <button onClick={handleNewPoc}
          className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors">
          <Plus className="w-2.5 h-2.5" /> {t("vulnIntel.addPoc", "Add PoC")}
        </button>
      </div>

      {editing && (
        <div className="space-y-2 p-2.5 border border-border/15 rounded bg-[var(--bg-hover)]/20">
          <div className="flex items-center gap-2">
            <input
              value={formName}
              onChange={(e) => setFormName(e.target.value)}
              placeholder={t("vulnIntel.pocName", "PoC name...")}
              className="flex-1 h-6 px-2 text-[10px] bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
            />
            <select value={formType} onChange={(e) => handleTypeChange(e.target.value as PocTemplate["type"])}
              className="h-6 px-1.5 text-[9px] bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground outline-none">
              <option value="nuclei">Nuclei YAML</option>
              <option value="script">Script</option>
              <option value="manual">Manual</option>
            </select>
            {formType === "script" && (
              <select value={formLang} onChange={(e) => handleLangChange(e.target.value)}
                className="h-6 px-1.5 text-[9px] bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground outline-none">
                <option value="python">Python</option>
                <option value="bash">Bash</option>
                <option value="go">Go</option>
                <option value="c">C</option>
                <option value="javascript">JS</option>
              </select>
            )}
          </div>
          <textarea
            value={formContent}
            onChange={(e) => setFormContent(e.target.value)}
            placeholder={t("vulnIntel.pocContentPlaceholder", "Paste or write your PoC template here...")}
            rows={12}
            className="w-full px-3 py-2 text-[10px] font-mono bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 resize-y leading-relaxed"
          />
          <div className="flex items-center gap-2">
            <button onClick={handleSavePoc} disabled={!formName.trim() || !formContent.trim()}
              className="px-3 py-1 rounded text-[9px] font-medium text-accent bg-accent/10 hover:bg-accent/20 transition-colors disabled:opacity-30">
              {editing.id ? t("vulnIntel.updatePoc", "Update") : t("vulnIntel.savePoc", "Save PoC")}
            </button>
            <button onClick={() => setEditing(null)}
              className="px-3 py-1 rounded text-[9px] text-muted-foreground/40 hover:text-foreground transition-colors">
              {t("common.cancel")}
            </button>
          </div>
        </div>
      )}

      {link.pocTemplates.length === 0 && !editing ? (
        <div className="flex flex-col items-center justify-center py-6 gap-2 text-muted-foreground/20">
          <Code className="w-8 h-8" />
          <p className="text-[10px]">{t("vulnIntel.noPoc", "No PoC templates")}</p>
          <p className="text-[9px] text-muted-foreground/15 max-w-xs text-center">{t("vulnIntel.pocHint", "Add Nuclei YAML templates, scripts, or manual testing notes for this vulnerability")}</p>
        </div>
      ) : (
        <div className="space-y-1">
          {link.pocTemplates.map((poc) => (
            <div key={poc.id} className="border border-border/10 rounded overflow-hidden">
              <div
                className="flex items-center gap-2 px-2 py-1.5 cursor-pointer hover:bg-muted/5 transition-colors group"
                onClick={() => setExpandedPoc(expandedPoc === poc.id ? null : poc.id)}
              >
                {typeIcon(poc.type)}
                <span className="text-[10px] text-foreground/70 truncate flex-1">{poc.name}</span>
                <span className="text-[8px] text-muted-foreground/25 px-1.5 py-0.5 bg-muted/10 rounded">{poc.type}</span>
                <span className="text-[8px] text-muted-foreground/20">{poc.language}</span>
                <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button onClick={(e) => { e.stopPropagation(); handleCopyContent(poc.content); }}
                    className="p-0.5 rounded text-muted-foreground/30 hover:text-accent transition-colors">
                    <Copy className="w-3 h-3" />
                  </button>
                  <button onClick={(e) => { e.stopPropagation(); handleEditPoc(poc); }}
                    className="p-0.5 rounded text-muted-foreground/30 hover:text-accent transition-colors">
                    <FileCode2 className="w-3 h-3" />
                  </button>
                  <button onClick={(e) => { e.stopPropagation(); handleDeletePoc(poc.id); }}
                    className="p-0.5 rounded text-muted-foreground/30 hover:text-destructive transition-colors">
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
                {expandedPoc === poc.id ? <ChevronDown className="w-3 h-3 text-muted-foreground/30" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/30" />}
              </div>
              {expandedPoc === poc.id && (
                <pre className="px-3 py-2 text-[10px] font-mono text-foreground/50 bg-[var(--bg-hover)]/20 border-t border-border/10 overflow-x-auto max-h-64 overflow-y-auto leading-relaxed">
                  {poc.content}
                </pre>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function HistoryTab({ link }: { link: VulnLink }) {
  const { t } = useTranslation();

  const resultBadge = (result: ScanHistoryEntry["result"]) => {
    const map: Record<string, { cls: string; label: string }> = {
      vulnerable: { cls: "text-red-400 bg-red-500/10", label: t("vulnIntel.vulnerable", "Vulnerable") },
      not_vulnerable: { cls: "text-green-400 bg-green-500/10", label: t("vulnIntel.notVulnerable", "Not Vulnerable") },
      error: { cls: "text-yellow-400 bg-yellow-500/10", label: t("common.error") },
      pending: { cls: "text-zinc-400 bg-zinc-500/10", label: t("vulnIntel.pending", "Pending") },
    };
    const m = map[result] || map.pending;
    return <span className={cn("text-[8px] px-1.5 py-0.5 rounded-full font-medium", m.cls)}>{m.label}</span>;
  };

  if (link.scanHistory.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-6 gap-2 text-muted-foreground/20">
        <History className="w-8 h-8" />
        <p className="text-[10px]">{t("vulnIntel.noHistory", "No scan history")}</p>
        <p className="text-[9px] text-muted-foreground/15">{t("vulnIntel.historyHint", "Scan history will appear here when you test targets with this vulnerability's PoC")}</p>
      </div>
    );
  }

  return (
    <div className="space-y-1">
      {link.scanHistory.map((entry, i) => (
        <div key={`${entry.target}-${entry.date}-${i}`} className="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-muted/5 transition-colors">
          {resultBadge(entry.result)}
          <span className="text-[10px] font-mono text-foreground/60 truncate flex-1">{entry.target}</span>
          <span className="text-[8px] text-muted-foreground/20">{new Date(entry.date).toLocaleDateString()}</span>
          {entry.details && <span className="text-[8px] text-muted-foreground/30 max-w-[150px] truncate">{entry.details}</span>}
        </div>
      ))}
    </div>
  );
}

function VulnKbTopBar({ activeTab, onTabChange }: { activeTab: TopTab; onTabChange: (tab: TopTab) => void }) {
  const { t } = useTranslation();
  const tabs: { id: TopTab; icon: typeof Shield; label: string }[] = [
    { id: "intel", icon: Shield, label: t("vulnKb.intelTab", "Intel") },
    { id: "wiki", icon: BookOpen, label: t("vulnKb.wikiTab", "Wiki") },
    { id: "poc-library", icon: Code, label: t("vulnKb.pocTab", "PoC Library") },
  ];

  return (
    <div className="flex items-center gap-1 px-3 py-2 border-b border-border/20 flex-shrink-0">
      <AlertTriangle className="w-3.5 h-3.5 text-accent/70 mr-1" />
      <span className="text-[11px] font-medium mr-3">{t("vulnKb.title", "Vulnerability KB")}</span>
      {tabs.map((tab) => (
        <button
          key={tab.id}
          onClick={() => onTabChange(tab.id)}
          className={cn(
            "flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors",
            activeTab === tab.id ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground hover:bg-muted/10"
          )}
        >
          <tab.icon className="w-3 h-3" />
          {tab.label}
        </button>
      ))}
    </div>
  );
}

function PocLibraryView({ vulnLinks, onLinksChange, onJumpToCve }: { vulnLinks: Record<string, VulnLink>; onLinksChange: (links: Record<string, VulnLink>) => void; onJumpToCve?: (cveId: string) => void }) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [expandedPoc, setExpandedPoc] = useState<string | null>(null);
  const [filterType, setFilterType] = useState<"all" | "nuclei" | "script" | "manual">("all");
  const [runTarget, setRunTarget] = useState<{ cveId: string; poc: PocTemplate } | null>(null);
  const [targetUrl, setTargetUrl] = useState("");

  const allPocs = useMemo(() => {
    const result: { cveId: string; poc: PocTemplate }[] = [];
    for (const [cveId, link] of Object.entries(vulnLinks)) {
      for (const poc of link.pocTemplates) {
        result.push({ cveId, poc });
      }
    }
    return result.sort((a, b) => b.poc.created - a.poc.created);
  }, [vulnLinks]);

  const filtered = useMemo(() => {
    let items = allPocs;
    if (filterType !== "all") items = items.filter((p) => p.poc.type === filterType);
    if (search.trim()) {
      const q = search.toLowerCase();
      items = items.filter((p) => p.poc.name.toLowerCase().includes(q) || p.cveId.toLowerCase().includes(q) || p.poc.content.toLowerCase().includes(q));
    }
    return items;
  }, [allPocs, filterType, search]);

  const handleDeletePoc = useCallback((cveId: string, pocId: string) => {
    const next = { ...vulnLinks };
    const link = next[cveId];
    if (link) {
      next[cveId] = { ...link, pocTemplates: link.pocTemplates.filter((p) => p.id !== pocId) };
      saveVulnLinks(next);
      onLinksChange(next);
    }
  }, [vulnLinks, onLinksChange]);

  const handleCopy = useCallback((content: string) => {
    navigator.clipboard.writeText(content);
  }, []);

  const handleRunPoc = useCallback(() => {
    if (!runTarget || !targetUrl.trim()) return;
    const rendered = runTarget.poc.content.replace(/\{\{BaseURL\}\}/g, targetUrl.trim());
    navigator.clipboard.writeText(rendered);

    const next = { ...vulnLinks };
    const link = next[runTarget.cveId] || { wikiPaths: [], pocTemplates: [], scanHistory: [] };
    link.scanHistory = [
      { target: targetUrl.trim(), date: Date.now(), result: "pending" as const },
      ...link.scanHistory,
    ];
    next[runTarget.cveId] = link;
    saveVulnLinks(next);
    onLinksChange(next);

    setRunTarget(null);
    setTargetUrl("");
  }, [runTarget, targetUrl, vulnLinks, onLinksChange]);

  const typeIcon = (type: PocTemplate["type"]) => {
    if (type === "nuclei") return <Zap className="w-3.5 h-3.5 text-orange-400/60" />;
    if (type === "script") return <FileCode2 className="w-3.5 h-3.5 text-emerald-400/60" />;
    return <FileText className="w-3.5 h-3.5 text-blue-400/60" />;
  };

  return (
    <div className="h-full flex flex-col">
      {/* Toolbar */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border/10 flex-shrink-0">
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("vulnKb.searchPoc", "Search PoC templates...")}
            className="w-full h-7 pl-8 pr-3 text-[11px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
          />
        </div>
        <select
          value={filterType}
          onChange={(e) => setFilterType(e.target.value as typeof filterType)}
          className="h-7 px-2 text-[10px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground outline-none"
        >
          <option value="all">{t("vulnKb.allTypes", "All Types")}</option>
          <option value="nuclei">Nuclei</option>
          <option value="script">Script</option>
          <option value="manual">Manual</option>
        </select>
        <span className="text-[10px] text-muted-foreground/30">
          {filtered.length} / {allPocs.length} templates
        </span>
      </div>

      {/* PoC list */}
      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <Code className="w-12 h-12" />
            <p className="text-[13px] font-medium">{t("vulnKb.noPocTemplates", "No PoC templates")}</p>
            <p className="text-[11px] text-muted-foreground/15 max-w-sm text-center">
              {t("vulnKb.pocLibraryHint", "Add PoC templates to vulnerabilities in the Intel tab, and they'll appear here")}
            </p>
          </div>
        ) : (
          <div className="divide-y divide-border/5">
            {filtered.map(({ cveId, poc }) => (
              <div key={poc.id} className="group">
                <div
                  className="flex items-center gap-3 px-4 py-2.5 cursor-pointer hover:bg-[var(--bg-hover)]/30 transition-colors"
                  onClick={() => setExpandedPoc(expandedPoc === poc.id ? null : poc.id)}
                >
                  {typeIcon(poc.type)}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-[11px] text-foreground/70 truncate">{poc.name}</span>
                      <button
                        onClick={(e) => { e.stopPropagation(); onJumpToCve?.(cveId); }}
                        className="text-[8px] px-1.5 py-0.5 rounded bg-accent/10 text-accent font-mono hover:bg-accent/20 transition-colors"
                        title="View in Intel"
                      >
                        {cveId}
                      </button>
                    </div>
                    <div className="flex items-center gap-2 mt-0.5">
                      <span className="text-[9px] text-muted-foreground/25 px-1.5 py-0.5 bg-muted/10 rounded">{poc.type}</span>
                      <span className="text-[9px] text-muted-foreground/20">{poc.language}</span>
                      <span className="text-[8px] text-muted-foreground/20">{new Date(poc.created).toLocaleDateString()}</span>
                    </div>
                  </div>
                  <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                    <button onClick={(e) => { e.stopPropagation(); setRunTarget({ cveId, poc }); setTargetUrl(""); }}
                      className="p-1 rounded text-emerald-400/40 hover:text-emerald-400 transition-colors" title="Run PoC">
                      <Play className="w-3 h-3" />
                    </button>
                    <button onClick={(e) => { e.stopPropagation(); handleCopy(poc.content); }}
                      className="p-1 rounded text-muted-foreground/30 hover:text-accent transition-colors" title="Copy">
                      <Copy className="w-3 h-3" />
                    </button>
                    <button onClick={(e) => { e.stopPropagation(); handleDeletePoc(cveId, poc.id); }}
                      className="p-1 rounded text-muted-foreground/30 hover:text-destructive transition-colors" title="Delete">
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>
                  {expandedPoc === poc.id ? <ChevronDown className="w-3 h-3 text-muted-foreground/30" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/30" />}
                </div>
                {expandedPoc === poc.id && (
                  <pre className="mx-4 mb-2 px-3 py-2 text-[10px] font-mono text-foreground/50 bg-[var(--bg-hover)]/20 border border-border/10 rounded overflow-x-auto max-h-80 overflow-y-auto leading-relaxed">
                    {poc.content}
                  </pre>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Run PoC Dialog */}
      {runTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setRunTarget(null)}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-md w-full" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center gap-2 mb-3">
              <Play className="w-4 h-4 text-emerald-400" />
              <h3 className="text-[13px] font-semibold text-foreground">Run PoC</h3>
              <span className="text-[9px] px-1.5 py-0.5 rounded bg-accent/10 text-accent font-mono">{runTarget.cveId}</span>
            </div>
            <p className="text-[10px] text-muted-foreground/50 mb-3">{runTarget.poc.name}</p>
            <div className="space-y-2">
              <label className="text-[10px] text-muted-foreground/50">Target URL</label>
              <input
                value={targetUrl}
                onChange={(e) => setTargetUrl(e.target.value)}
                placeholder="https://target.example.com"
                className="w-full h-8 px-3 text-[12px] font-mono bg-background rounded-lg border border-border/20 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
                autoFocus
                onKeyDown={(e) => e.key === "Enter" && handleRunPoc()}
              />
              <p className="text-[9px] text-muted-foreground/30">
                {"{{BaseURL}}"} in the template will be replaced with the target URL. The rendered PoC will be copied to clipboard.
              </p>
            </div>
            <div className="flex justify-end gap-2 mt-4">
              <button onClick={() => setRunTarget(null)}
                className="text-[11px] px-3 py-1.5 rounded-lg text-muted-foreground/50 hover:text-foreground transition-colors">
                Cancel
              </button>
              <button onClick={handleRunPoc} disabled={!targetUrl.trim()}
                className="flex items-center gap-1.5 text-[11px] px-4 py-1.5 rounded-lg font-medium bg-emerald-500/15 text-emerald-400 hover:bg-emerald-500/25 transition-colors disabled:opacity-30">
                <Play className="w-3 h-3" />
                Run & Copy
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
