import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  Bell,
  BookOpen,
  Bot,
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
  MessageSquare,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import { useTranslation } from "react-i18next";
import { useStore } from "@/store";
import { sendPromptSession, initAiSession, buildProviderConfig, respondToToolApproval, setAgentMode, type AiProvider } from "@/lib/ai";
import { getSettings } from "@/lib/settings";
import { Markdown } from "@/components/Markdown";

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

interface DbVulnLinkFull {
  wiki_paths: string[];
  poc_templates: Array<{ id: string; name: string; type: string; language: string; content: string; created: number }>;
  scan_history: Array<{ id: string; target: string; date: number; result: string; details?: string }>;
}

function dbToVulnLink(db: DbVulnLinkFull): VulnLink {
  return {
    wikiPaths: db.wiki_paths,
    pocTemplates: db.poc_templates.map((p) => ({
      id: p.id,
      name: p.name,
      type: p.type as PocTemplate["type"],
      language: p.language,
      content: p.content,
      created: p.created,
    })),
    scanHistory: db.scan_history.map((s) => ({
      target: s.target,
      date: s.date,
      result: s.result as ScanHistoryEntry["result"],
      details: s.details,
    })),
  };
}

const EMPTY_LINK: VulnLink = { wikiPaths: [], pocTemplates: [], scanHistory: [] };

function getOrCreateLink(links: Record<string, VulnLink>, cveId: string): VulnLink {
  return links[cveId] || EMPTY_LINK;
}

type ViewMode = "feed" | "matched" | "feeds-config";
type DetailTab = "intel" | "wiki" | "poc" | "history" | "research";
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
}: {
  entry: VulnEntry;
  link: VulnLink;
  detailTab: DetailTab;
  onTabChange: (tab: DetailTab) => void;
  onUpdateLink: (updater: (link: VulnLink) => VulnLink) => void;
}) {
  const [ingesting, setIngesting] = useState(false);
  const [researchSessionId, setResearchSessionId] = useState<string | null>(null);
  const [researchError, setResearchError] = useState<string | null>(null);
  const [hasResearchHistory, setHasResearchHistory] = useState(false);

  // Check if DB has previous research for this CVE
  useEffect(() => {
    invoke<{ turns: unknown[]; status: string } | null>("kb_research_load", { cveId: entry.cve_id })
      .then((log) => {
        if (log?.turns && Array.isArray(log.turns) && log.turns.length > 0) {
          setHasResearchHistory(true);
        }
      })
      .catch(() => {});
  }, [entry.cve_id]);

  // Load fresh link data from DB when viewing this CVE
  useEffect(() => {
    invoke<DbVulnLinkFull>("vuln_link_get", { cveId: entry.cve_id })
      .then((dbLink) => {
        const link = dbToVulnLink(dbLink);
        if (link.wikiPaths.length > 0 || link.pocTemplates.length > 0 || link.scanHistory.length > 0) {
          onUpdateLink(() => link);
        }
      })
      .catch(() => {});
  }, [entry.cve_id, onUpdateLink]);

  const handleAiResearch = useCallback(async () => {
    setIngesting(true);
    setResearchError(null);
    try {
      const sessionId = `kb-research-${entry.cve_id}-${Date.now()}`;
      const state = useStore.getState();
      const parentSession = state.sessions[state.activeSessionId ?? ""];
      const workspace = parentSession?.workingDirectory || ".";

      state.addSession(
        {
          id: sessionId,
          logicalTerminalId: crypto.randomUUID(),
          name: `KB: ${entry.cve_id}`,
          workingDirectory: workspace,
          createdAt: new Date().toISOString(),
          mode: "agent",
          inputMode: "agent",
        },
        { isPaneSession: true }
      );

      const settings = await getSettings();
      const researchProvider = (settings.ai.research_provider ?? settings.ai.default_provider) as AiProvider;
      const researchModel = settings.ai.research_model ?? settings.ai.default_model;

      const config = await buildProviderConfig(settings, workspace, {
        provider: researchProvider,
        model: researchModel,
      });
      await initAiSession(sessionId, config);
      useStore.getState().setSessionAiConfig(sessionId, {
        provider: researchProvider,
        model: researchModel,
        status: "ready",
      });

      setResearchSessionId(sessionId);
      onTabChange("research");

      const product = entry.affected_products?.length > 0
        ? entry.affected_products.join(", ")
        : "unknown product";

      const slug = product.split(",")[0].trim().toLowerCase().replace(/\s+/g, "-");
      const prompt = `# Vulnerability Knowledge Base — Ingest Guide

## Source CVE
- **CVE**: ${entry.cve_id}
- **Title**: ${entry.title}
- **Severity**: ${entry.severity}${entry.cvss_score != null ? ` (CVSS ${entry.cvss_score})` : ""}
- **Affected**: ${product}
- **Description**: ${entry.description.slice(0, 500)}

## Wiki Architecture

The vulnerability wiki follows a Karpathy-style compounding knowledge model. You maintain TWO types of pages:

### Products (\`products/{product-slug}/\`)
One page per CVE, specific to the affected product. Contains vulnerability details, exploitation, PoC, detection.
- Path: \`products/${slug}/${entry.cve_id}.md\`

### Techniques (\`techniques/\`)
One page per attack technique. Shared across multiple CVEs. Contains methodology, variants, examples.
- Example: \`techniques/jndi-injection.md\`, \`techniques/deserialization.md\`, \`techniques/ssrf.md\`

## Writing Standards

Every wiki page MUST have:
- YAML frontmatter: \`title\`, \`category\`, \`tags\`, \`cves\`, \`status\`
- Rich content with clear sections (## headings)
- Cross-references to related pages (markdown links to other wiki paths)
- Citations to sources (NVD, advisories, blog posts)
- Status: \`draft\`, \`partial\`, \`complete\`, \`needs-poc\`, \`verified\`

## Ingest Workflow

### Step 1: Check existing knowledge
Use \`search_knowledge_base\` with query "${entry.cve_id}" to find existing pages.
- If the CVE product page exists and status is \`complete\`/\`verified\`, check if technique pages also exist. If everything is covered, report completion.
- If pages exist but are \`draft\`/\`partial\`/\`needs-poc\`, continue to enrich them.

### Step 2: Create the product page
Use \`ingest_cve\` with cve_id="${entry.cve_id}" and product="${slug}" to create the base product page (if it doesn't exist).

### Step 3: Research
Search the web for:
- Exploit details and attack chains
- PoC code or exploit scripts
- Technical advisories and patch details
- Related CVEs and attack techniques

### Step 4: Update the product page
Use \`write_knowledge\` with \`cve_id: "${entry.cve_id}"\` to update \`products/${slug}/${entry.cve_id}.md\` with:
- Detailed vulnerability analysis
- Exploitation method and attack chain
- PoC code (if publicly available)
- Detection signatures and mitigation
- References and citations
- Cross-references to technique pages

### Step 5: Create/update technique pages
Identify the core attack technique(s) used (e.g., JNDI injection, deserialization, SSRF).
For EACH technique:
1. \`search_knowledge_base\` to check if a technique page exists
2. If not, use \`write_knowledge\` with \`cve_id: "${entry.cve_id}"\` to create \`techniques/{technique-slug}.md\` with:
   - Technique overview and methodology
   - Common variants
   - List of CVEs that use this technique (including this one)
   - Detection and prevention strategies
3. If it exists, use \`read_knowledge\` then \`write_knowledge\` with \`cve_id: "${entry.cve_id}"\` to ADD this CVE to the existing technique page's CVE list and update any new information.
**CRITICAL**: Always pass the \`cve_id\` parameter when calling \`write_knowledge\` so the page is automatically linked to this CVE and appears in the Wiki tab.

### Step 6: Cross-reference check
Use \`search_knowledge_base\` to find related pages. Add "See Also" cross-references where appropriate.

### Step 7: Save PoC templates
If you found exploit code, detection templates, or testing scripts during research, save them using \`save_poc\`:
- Use \`cve_id: "${entry.cve_id}"\`
- For Nuclei YAML templates: \`poc_type: "nuclei"\`, \`language: "yaml"\`
- For exploit scripts: \`poc_type: "script"\`, \`language: "python"\` (or bash, go, etc.)
- For manual testing procedures: \`poc_type: "manual"\`, \`language: "markdown"\`
Save each distinct PoC or template as a separate entry.

### Step 8: Set status
Update the product page frontmatter \`status\`:
- \`complete\` if you found exploit details + PoC
- \`partial\` if missing key sections
- \`needs-poc\` if analysis is thorough but no public PoC exists

## IMPORTANT
- A single CVE ingest should typically create/update 2-5 wiki pages.
- Technique pages are SHARED — multiple CVEs reference the same technique page.
- Always check for existing content before creating new pages.
- Never overwrite existing content — merge and enrich.
- Always save found PoC/exploit code using \`save_poc\` so it appears in the PoC tab.`;

      await sendPromptSession(sessionId, prompt);

      const expectedPath = `products/${slug}/${entry.cve_id}.md`;
      onUpdateLink((l) => ({
        ...l,
        wikiPaths: l.wikiPaths.includes(expectedPath) ? l.wikiPaths : [...l.wikiPaths, expectedPath],
      }));
      invoke("vuln_link_add_wiki", { cveId: entry.cve_id, wikiPath: expectedPath }).catch(console.error);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("Failed to trigger AI research:", e);
      setResearchError(msg);
    } finally {
      setIngesting(false);
    }
  }, [entry, onTabChange]);

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
        <div className="ml-auto">
          <button
            onClick={handleAiResearch}
            disabled={ingesting}
            className="flex items-center gap-1 px-2.5 py-1 rounded text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors disabled:opacity-50"
            title="AI researches this CVE: searches web, writes wiki page with exploit details, PoCs, and analysis"
          >
            {ingesting ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Bot className="w-3.5 h-3.5" />}
            AI Research
          </button>
        </div>
      </div>

      {researchError && (
        <div className="flex items-center gap-2 px-4 py-2 bg-red-500/10 border-b border-red-500/20">
          <AlertTriangle className="w-3.5 h-3.5 text-red-400 flex-shrink-0" />
          <span className="text-[10px] text-red-400 flex-1">{researchError}</span>
          <button onClick={() => setResearchError(null)} className="text-red-400/50 hover:text-red-400">
            <X className="w-3 h-3" />
          </button>
        </div>
      )}

      {/* Detail tabs */}
      <div className="flex items-center gap-0.5 px-3 py-1.5 border-b border-border/10 bg-muted/3 flex-shrink-0">
        {([
          { id: "intel" as const, icon: Shield, label: "Intel" },
          { id: "wiki" as const, icon: BookOpen, label: `Wiki${link.wikiPaths.length > 0 ? ` (${link.wikiPaths.length})` : ""}` },
          { id: "poc" as const, icon: Code, label: `PoC${link.pocTemplates.length > 0 ? ` (${link.pocTemplates.length})` : ""}` },
          { id: "history" as const, icon: History, label: `History${link.scanHistory.length > 0 ? ` (${link.scanHistory.length})` : ""}` },
          ...(researchSessionId || hasResearchHistory ? [{ id: "research" as const, icon: MessageSquare, label: "Research" }] : []),
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
        {detailTab === "wiki" && <WikiTab link={link} cveId={entry.cve_id} onUpdateLink={onUpdateLink} />}
        {detailTab === "poc" && <PocTab link={link} cveId={entry.cve_id} onUpdateLink={onUpdateLink} />}
        {detailTab === "history" && <HistoryTab link={link} />}
        {detailTab === "research" && <ResearchTab sessionId={researchSessionId} cveId={entry.cve_id} />}
      </div>
    </>
  );
}

type TurnBlock =
  | { type: "text"; content: string }
  | { type: "tool"; id: string; name: string; status: string };

interface CompletedTurn {
  id: string;
  blocks: TurnBlock[];
  /** @deprecated kept for backward compat with old DB records */
  text?: string;
  /** @deprecated kept for backward compat with old DB records */
  toolCalls?: Array<{ id: string; name: string; status: string }>;
}

const EMPTY_STREAMING: never[] = [];

function ResearchTab({ sessionId, cveId }: { sessionId: string | null; cveId: string }) {
  const sid = sessionId ?? "";
  const isResponding = useStore((s) => sid ? (s.isAgentResponding[sid] ?? false) : false);
  const streamingBlocks = useStore((s) => {
    if (!sid) return EMPTY_STREAMING;
    return s.streamingBlocks[sid] ?? EMPTY_STREAMING;
  });
  const isThinking = useStore((s) => sid ? (s.isAgentThinking[sid] ?? false) : false);
  const storeApproval = useStore((s) => sid ? (s.pendingToolApproval[sid] ?? null) : null);
  const storeAskHuman = useStore((s) => sid ? (s.pendingAskHuman[sid] ?? null) : null);
  const [dismissedApprovalId, setDismissedApprovalId] = useState<string | null>(null);
  const [dismissedAskHumanId, setDismissedAskHumanId] = useState<string | null>(null);
  const agentMode = useStore((s) => sid ? (s.sessions[sid]?.agentMode ?? "default") : "default");
  const isAutoApprove = agentMode === "auto-approve";
  const pendingApproval = isResponding && !isAutoApprove && storeApproval && storeApproval.id !== dismissedApprovalId ? storeApproval : null;
  const pendingAskHuman = isResponding && storeAskHuman && storeAskHuman.requestId !== dismissedAskHumanId ? storeAskHuman : null;
  const scrollRef = useRef<HTMLDivElement>(null);
  const [askHumanInput, setAskHumanInput] = useState("");

  const [completedTurns, setCompletedTurns] = useState<CompletedTurn[]>([]);
  const [loadedFromDb, setLoadedFromDb] = useState(false);
  const prevBlocksRef = useRef<typeof streamingBlocks>([]);

  // Load previous research conversation from DB on mount
  useEffect(() => {
    invoke<{ turns: Array<Record<string, unknown>>; status: string } | null>("kb_research_load", { cveId })
      .then((log) => {
        if (log?.turns && Array.isArray(log.turns) && log.turns.length > 0) {
          const migrated: CompletedTurn[] = log.turns.map((raw) => {
            if (Array.isArray(raw.blocks)) return raw as unknown as CompletedTurn;
            // Migrate old format: { text, toolCalls } -> { blocks }
            const blocks: TurnBlock[] = [];
            const oldTools = Array.isArray(raw.toolCalls) ? raw.toolCalls as Array<{ id: string; name: string; status: string }> : [];
            for (const tc of oldTools) blocks.push({ type: "tool", ...tc });
            if (typeof raw.text === "string" && raw.text) blocks.push({ type: "text", content: raw.text });
            return { id: (raw.id as string) || crypto.randomUUID(), blocks };
          });
          setCompletedTurns(migrated);
        }
      })
      .catch((e) => console.error("Failed to load research log:", e))
      .finally(() => setLoadedFromDb(true));
  }, [cveId]);

  // Capture completed turns preserving block order and persist to DB
  useEffect(() => {
    const hadBlocks = prevBlocksRef.current.length > 0;
    const nowEmpty = streamingBlocks.length === 0;

    if (hadBlocks && nowEmpty) {
      const blocks: TurnBlock[] = [];
      let textAcc = "";
      for (const b of prevBlocksRef.current) {
        if (b.type === "text") {
          textAcc += b.content;
        } else if (b.type === "tool") {
          if (textAcc) { blocks.push({ type: "text", content: textAcc }); textAcc = ""; }
          blocks.push({ type: "tool", id: b.toolCall.id, name: b.toolCall.name, status: b.toolCall.status });
        }
      }
      if (textAcc) blocks.push({ type: "text", content: textAcc });

      if (blocks.length > 0) {
        const turn: CompletedTurn = { id: crypto.randomUUID(), blocks };
        setCompletedTurns((prev) => [...prev, turn]);
        invoke("kb_research_save_turn", { cveId, sessionId: sid, turn }).catch((e) =>
          console.error("Failed to save research turn:", e)
        );
      }
    }
    prevBlocksRef.current = streamingBlocks;
  }, [streamingBlocks, cveId, sid]);

  // Mark research as completed when agent finishes and we have content
  const prevRespondingRef = useRef(false);
  useEffect(() => {
    if (prevRespondingRef.current && !isResponding && completedTurns.length > 0) {
      invoke("kb_research_set_status", { cveId, status: "completed" }).catch((e) =>
        console.error("Failed to set research status:", e)
      );
    }
    prevRespondingRef.current = isResponding;
  }, [isResponding, completedTurns.length, cveId]);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [completedTurns, streamingBlocks, isResponding, pendingApproval, pendingAskHuman]);

  const isEmpty = loadedFromDb && completedTurns.length === 0 && streamingBlocks.length === 0 && !isResponding;
  const [clearing, setClearing] = useState(false);

  const handleClearHistory = useCallback(async () => {
    if (!confirm("Delete all research history for this CVE? This cannot be undone.")) return;
    setClearing(true);
    try {
      await invoke("kb_research_clear", { cveId });
      setCompletedTurns([]);
    } catch (e) {
      console.error("Failed to clear research history:", e);
    }
    setClearing(false);
  }, [cveId]);

  const proseClasses = "text-[11px] leading-relaxed text-foreground/80 prose prose-invert prose-sm max-w-none prose-headings:text-foreground/90 prose-headings:text-[12px] prose-headings:font-semibold prose-p:text-[11px] prose-p:leading-relaxed prose-code:text-[10px] prose-code:bg-muted/20 prose-code:px-1 prose-code:rounded prose-pre:bg-muted/10 prose-pre:border prose-pre:border-border/10 prose-pre:text-[10px] prose-li:text-[11px] prose-a:text-accent";

  return (
    <div ref={scrollRef} className="space-y-3 -mx-4 -my-3 px-4 py-3 overflow-y-auto max-h-full">
      {/* Header with clear button */}
      {completedTurns.length > 0 && !isResponding && (
        <div className="flex items-center justify-end">
          <button
            onClick={handleClearHistory}
            disabled={clearing}
            className="flex items-center gap-1 text-[9px] text-destructive/50 hover:text-destructive transition-colors disabled:opacity-30"
          >
            {clearing ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : <Trash2 className="w-2.5 h-2.5" />}
            Clear History
          </button>
        </div>
      )}

      {!loadedFromDb && (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
        </div>
      )}
      {isEmpty && (
        <div className="flex flex-col items-center justify-center py-12 text-muted-foreground/30">
          <Bot className="w-8 h-8 mb-2" />
          <span className="text-[11px]">No research started yet</span>
        </div>
      )}

      {/* Completed turns history — blocks rendered in order */}
      {completedTurns.map((turn) => (
        <div key={turn.id} className="space-y-2">
          {turn.blocks.map((block, i) =>
            block.type === "tool" ? (
              <div key={block.id || i} className="flex items-center gap-2 px-3 py-1.5 rounded bg-muted/8 border border-border/5">
                <Zap className="w-3 h-3 text-accent/60 flex-shrink-0" />
                <span className="text-[10px] font-mono text-foreground/60 truncate">{block.name}</span>
                <span className={cn(
                  "text-[9px] ml-auto px-1.5 py-0.5 rounded",
                  block.status === "completed" ? "text-green-400 bg-green-500/10" :
                  block.status === "error" ? "text-red-400 bg-red-500/10" :
                  "text-muted-foreground/40 bg-muted/10"
                )}>
                  {block.status === "completed" ? "done" : block.status === "error" ? "error" : "done"}
                </span>
              </div>
            ) : block.content ? (
              <div key={i} className={proseClasses}>
                <Markdown content={block.content} />
              </div>
            ) : null
          )}
        </div>
      ))}

      {/* Live streaming: rendered in order (interleaved) */}
      {isThinking && (
        <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-muted/10 border border-border/5">
          <Loader2 className="w-3 h-3 animate-spin text-muted-foreground/50" />
          <span className="text-[10px] text-muted-foreground/50">Thinking...</span>
        </div>
      )}
      {streamingBlocks.map((block, i) =>
        block.type === "tool" ? (
          <div key={block.toolCall.id} className="flex items-center gap-2 px-3 py-1.5 rounded bg-muted/8 border border-border/5">
            <Zap className="w-3 h-3 text-accent/60 flex-shrink-0" />
            <span className="text-[10px] font-mono text-foreground/60 truncate">{block.toolCall.name}</span>
            <span className={cn(
              "text-[9px] ml-auto px-1.5 py-0.5 rounded",
              block.toolCall.status === "completed" ? "text-green-400 bg-green-500/10" :
              block.toolCall.status === "error" ? "text-red-400 bg-red-500/10" :
              "text-muted-foreground/40 bg-muted/10"
            )}>
              {block.toolCall.status === "completed" ? "done" : block.toolCall.status === "error" ? "error" : "running..."}
            </span>
          </div>
        ) : block.type === "text" && block.content ? (
          <div key={`text-${i}`} className={proseClasses}>
            <Markdown content={block.content} streaming={isResponding} />
          </div>
        ) : null
      )}

      {/* Approval / Ask Human cards */}
      {pendingApproval && (
        <div className="rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 space-y-2">
          <div className="flex items-center gap-2">
            <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />
            <span className="text-[11px] font-medium text-amber-400">Tool approval needed</span>
          </div>
          <div className="text-[10px] font-mono text-foreground/60">{pendingApproval.name}</div>
          {pendingApproval.args && (
            <pre className="text-[9px] text-muted-foreground/40 bg-muted/10 rounded p-2 overflow-x-auto max-h-24">
              {JSON.stringify(pendingApproval.args, null, 2)}
            </pre>
          )}
          <div className="flex gap-2">
            <button
              onClick={() => { setDismissedApprovalId(pendingApproval.id); respondToToolApproval(sid, { request_id: pendingApproval.id, approved: true, remember: false, always_allow: false }).catch(console.error); }}
              className="px-3 py-1 rounded text-[10px] font-medium bg-emerald-500/15 text-emerald-400 hover:bg-emerald-500/25 transition-colors"
            >
              Approve
            </button>
            <button
              onClick={() => {
                setDismissedApprovalId(pendingApproval.id);
                respondToToolApproval(sid, { request_id: pendingApproval.id, approved: true, remember: false, always_allow: false }).catch(console.error);
                const ws = useStore.getState().sessions[sid]?.workingDirectory || ".";
                setAgentMode(sid, "auto-approve", ws).catch(console.error);
              }}
              className="px-3 py-1 rounded text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors"
            >
              Run Everything
            </button>
            <button
              onClick={() => { setDismissedApprovalId(pendingApproval.id); respondToToolApproval(sid, { request_id: pendingApproval.id, approved: false, remember: false, always_allow: false }).catch(console.error); }}
              className="px-3 py-1 rounded text-[10px] font-medium bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors"
            >
              Deny
            </button>
          </div>
        </div>
      )}

      {pendingAskHuman && (
        <div className="rounded-lg border border-accent/20 bg-accent/5 p-3 space-y-2">
          <div className="flex items-center gap-2">
            <MessageSquare className="w-3.5 h-3.5 text-accent" />
            <span className="text-[11px] font-medium text-accent">AI needs your input</span>
          </div>
          <div className="text-[11px] text-foreground/70">{pendingAskHuman.question}</div>
          {pendingAskHuman.options.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {pendingAskHuman.options.map((opt) => (
                <button
                  key={opt}
                  onClick={() => { setDismissedAskHumanId(pendingAskHuman.requestId); respondToToolApproval(sid, { request_id: pendingAskHuman.requestId, approved: true, reason: opt, remember: false, always_allow: false }).catch(console.error); }}
                  className="px-2.5 py-1 rounded text-[10px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
                >
                  {opt}
                </button>
              ))}
            </div>
          ) : (
            <div className="flex gap-2">
              <input
                type="text"
                value={askHumanInput}
                onChange={(e) => setAskHumanInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && askHumanInput.trim()) {
                    setDismissedAskHumanId(pendingAskHuman.requestId);
                    respondToToolApproval(sid, { request_id: pendingAskHuman.requestId, approved: true, reason: askHumanInput.trim(), remember: false, always_allow: false }).catch(console.error);
                    setAskHumanInput("");
                  }
                }}
                placeholder="Type your response..."
                className="flex-1 px-2.5 py-1 rounded text-[10px] bg-muted/10 border border-border/10 text-foreground placeholder:text-muted-foreground/30 focus:outline-none focus:ring-1 focus:ring-accent/30"
              />
              <button
                onClick={() => {
                  if (askHumanInput.trim()) {
                    setDismissedAskHumanId(pendingAskHuman.requestId);
                    respondToToolApproval(sid, { request_id: pendingAskHuman.requestId, approved: true, reason: askHumanInput.trim(), remember: false, always_allow: false }).catch(console.error);
                    setAskHumanInput("");
                  }
                }}
                className="px-2.5 py-1 rounded text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors"
              >
                Send
              </button>
            </div>
          )}
        </div>
      )}

      {isResponding && !isThinking && streamingBlocks.length === 0 && (
        <div className="flex items-center gap-2 py-2">
          <Loader2 className="w-3 h-3 animate-spin text-accent/60" />
          <span className="text-[10px] text-muted-foreground/40">Researching...</span>
        </div>
      )}

      {!isResponding && completedTurns.length > 0 && (
        <div className="flex items-center gap-1.5 px-3 py-2 rounded-lg bg-green-500/5 border border-green-500/10">
          <div className="w-1.5 h-1.5 rounded-full bg-green-500" />
          <span className="text-[10px] text-green-400">Research complete</span>
        </div>
      )}
    </div>
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

interface WikiTreeNode {
  path: string;
  name: string;
  is_dir: boolean;
  children?: WikiTreeNode[];
}

function WikiTab({ link, cveId, onUpdateLink }: { link: VulnLink; cveId: string; onUpdateLink: (updater: (l: VulnLink) => VulnLink) => void }) {
  const [fullTree, setFullTree] = useState<WikiTreeNode[]>([]);
  const [loadingTree, setLoadingTree] = useState(true);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [articleContents, setArticleContents] = useState<Record<string, string>>({});
  const [editingPath, setEditingPath] = useState<string | null>(null);
  const [editContent, setEditContent] = useState("");
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [adding, setAdding] = useState(false);
  const [newPath, setNewPath] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<{ path: string; title: string; snippet: string }[]>([]);
  const [searching, setSearching] = useState(false);
  const [creating, setCreating] = useState(false);
  const [createPath, setCreatePath] = useState("");

  const linkedSet = useMemo(() => new Set(link.wikiPaths), [link.wikiPaths]);

  const reloadTree = useCallback(() => {
    setLoadingTree(true);
    invoke<WikiTreeNode[]>("wiki_list")
      .then((tree) => {
        setFullTree(Array.isArray(tree) ? tree : []);
        const dirs = new Set<string>();
        for (const p of link.wikiPaths) {
          const parts = p.split("/");
          for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join("/"));
        }
        setExpandedDirs(dirs);
        if (link.wikiPaths.length > 0 && !selectedPath) setSelectedPath(link.wikiPaths[0]);
      })
      .catch(console.error)
      .finally(() => setLoadingTree(false));
  }, [link.wikiPaths, selectedPath]);

  useEffect(() => { reloadTree(); }, []);

  useEffect(() => {
    if (!selectedPath || articleContents[selectedPath] !== undefined) return;
    invoke<string>("wiki_read", { path: selectedPath })
      .then((content) => setArticleContents((prev) => ({ ...prev, [selectedPath]: content })))
      .catch(() => setArticleContents((prev) => ({ ...prev, [selectedPath]: "" })));
  }, [selectedPath]);

  const toggleDir = useCallback((dir: string) => {
    setExpandedDirs((prev) => {
      const next = new Set(prev);
      if (next.has(dir)) next.delete(dir);
      else next.add(dir);
      return next;
    });
  }, []);

  const handleLinkWiki = useCallback((path: string) => {
    onUpdateLink((l) => ({
      ...l,
      wikiPaths: l.wikiPaths.includes(path) ? l.wikiPaths : [...l.wikiPaths, path],
    }));
    invoke("vuln_link_add_wiki", { cveId, wikiPath: path }).catch(console.error);
  }, [onUpdateLink, cveId]);

  const handleUnlinkWiki = useCallback((path: string) => {
    onUpdateLink((l) => ({ ...l, wikiPaths: l.wikiPaths.filter((p) => p !== path) }));
    invoke("vuln_link_remove_wiki", { cveId, wikiPath: path }).catch(console.error);
    if (selectedPath === path) setSelectedPath(null);
  }, [onUpdateLink, cveId, selectedPath]);

  const handleStartEdit = useCallback((path: string) => {
    setEditingPath(path);
    setEditContent(articleContents[path] || "");
  }, [articleContents]);

  const handleSaveEdit = useCallback(async () => {
    if (!editingPath) return;
    try {
      await invoke("wiki_write", { path: editingPath, content: editContent });
      setArticleContents((prev) => ({ ...prev, [editingPath]: editContent }));
      setEditingPath(null);
    } catch (err) {
      console.error("Failed to save wiki article:", err);
    }
  }, [editingPath, editContent]);

  const handleDeletePage = useCallback(async (path: string) => {
    if (!confirm(`Delete wiki page "${path}"? This cannot be undone.`)) return;
    try {
      await invoke("wiki_delete", { path });
      setArticleContents((prev) => {
        const next = { ...prev };
        delete next[path];
        return next;
      });
      if (selectedPath === path) setSelectedPath(null);
      if (linkedSet.has(path)) handleUnlinkWiki(path);
      reloadTree();
    } catch (err) {
      console.error("Failed to delete wiki page:", err);
    }
  }, [selectedPath, linkedSet, handleUnlinkWiki, reloadTree]);

  const handleCreatePage = useCallback(async () => {
    const p = createPath.trim();
    if (!p) return;
    const path = p.endsWith(".md") ? p : `${p}.md`;
    const template = `---\ntitle: ${path.split("/").pop()?.replace(/\.md$/, "") || "New Page"}\ncategory: ${path.split("/")[0] || "uncategorized"}\ntags: []\ncves: [${cveId}]\nstatus: draft\n---\n\n# ${path.split("/").pop()?.replace(/\.md$/, "") || "New Page"}\n\nContent here.\n`;
    try {
      await invoke("wiki_write", { path, content: template });
      handleLinkWiki(path);
      setCreating(false);
      setCreatePath("");
      setArticleContents((prev) => ({ ...prev, [path]: template }));
      reloadTree();
      setSelectedPath(path);
    } catch (err) {
      console.error("Failed to create wiki page:", err);
    }
  }, [createPath, cveId, handleLinkWiki, reloadTree]);

  // Full-text search via DB
  const handleSearch = useCallback(async (query: string) => {
    if (!query.trim()) { setSearchResults([]); return; }
    setSearching(true);
    try {
      const results = await invoke<{ path: string; title: string; category: string; tags: string[]; status: string | null }[]>(
        "wiki_search_db", { query: query.trim(), limit: 20 }
      );
      setSearchResults(results.map((r) => ({
        path: r.path,
        title: r.title || r.path,
        snippet: `[${r.category}] ${r.tags.join(", ")}${r.status ? ` • ${r.status}` : ""}`,
      })));
    } catch {
      setSearchResults([]);
    }
    setSearching(false);
  }, []);

  useEffect(() => {
    const t = setTimeout(() => handleSearch(searchQuery), 300);
    return () => clearTimeout(t);
  }, [searchQuery, handleSearch]);

  // Navigate to a wiki page (used by cross-reference links)
  const navigateToWikiPage = useCallback((path: string) => {
    const expandPath = (p: string) => {
      const parts = p.split("/");
      const dirs = new Set<string>();
      for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join("/"));
      return dirs;
    };
    setExpandedDirs((prev) => {
      const next = new Set(prev);
      for (const d of expandPath(path)) next.add(d);
      return next;
    });
    setArticleContents((prev) => {
      if (prev[path] !== undefined) return prev;
      return { ...prev };
    });
    setSelectedPath(path);
    setSearchQuery("");
    setSearchResults([]);
  }, []);

  const stripFrontmatter = (content: string) => {
    const match = content.match(/^---\n[\s\S]*?\n---\n?/);
    return match ? content.slice(match[0].length) : content;
  };

  const extractFrontmatterField = (content: string, field: string): string | null => {
    const fm = content.match(/^---\n([\s\S]*?)\n---/);
    if (!fm) return null;
    const m = fm[1].match(new RegExp(`^${field}:\\s*(.+)$`, "m"));
    return m?.[1]?.trim() || null;
  };

  const extractTitle = (content: string) => {
    const t = extractFrontmatterField(content, "title");
    if (t) return t.replace(/^["']|["']$/g, "");
    const h1 = content.match(/^#\s+(.+)$/m);
    return h1?.[1] || null;
  };

  const extractStatus = (content: string) => extractFrontmatterField(content, "status");

  const extractTags = (content: string): string[] => {
    const fm = content.match(/^---\n([\s\S]*?)\n---/);
    if (!fm) return [];
    const m = fm[1].match(/tags:\s*\[([^\]]*)\]/);
    if (m) return m[1].split(",").map((t) => t.trim().replace(/^["']|["']$/g, "")).filter(Boolean);
    return [];
  };

  const extractCves = (content: string): string[] => {
    const fm = content.match(/^---\n([\s\S]*?)\n---/);
    if (!fm) return [];
    const m = fm[1].match(/cves:\s*\[([^\]]*)\]/);
    if (m) return m[1].split(",").map((c) => c.trim().replace(/^["']|["']$/g, "")).filter(Boolean);
    return [];
  };

  // Intercept wiki-link clicks in rendered markdown
  const handleContentClick = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement;
    const anchor = target.closest("a");
    if (!anchor) return;
    const href = anchor.getAttribute("href") || "";
    if (href.match(/^(https?:|mailto:|#)/)) return;
    if (href.endsWith(".md") || !href.includes("://")) {
      e.preventDefault();
      const resolved = selectedPath
        ? new URL(href, `file:///${selectedPath}`).pathname.replace(/^\//, "")
        : href;
      navigateToWikiPage(resolved);
    }
  }, [selectedPath, navigateToWikiPage]);

  const statusColors: Record<string, string> = {
    draft: "text-yellow-400 bg-yellow-500/10",
    partial: "text-orange-400 bg-orange-500/10",
    complete: "text-green-400 bg-green-500/10",
    "needs-poc": "text-blue-400 bg-blue-500/10",
    verified: "text-emerald-400 bg-emerald-500/10",
  };

  const categoryIcons: Record<string, string> = {
    products: "📦",
    techniques: "⚔️",
    pocs: "🔧",
    experience: "📝",
    analysis: "🔬",
  };

  const proseClasses = "text-[11px] leading-relaxed text-foreground/80 prose prose-invert prose-sm max-w-none prose-headings:text-foreground/90 prose-headings:text-[12px] prose-headings:font-semibold prose-p:text-[11px] prose-p:leading-relaxed prose-code:text-[10px] prose-code:bg-muted/20 prose-code:px-1 prose-code:rounded prose-pre:bg-muted/10 prose-pre:border prose-pre:border-border/10 prose-pre:text-[10px] prose-li:text-[11px] prose-a:text-accent";

  // Filter tree nodes by search
  const filterTree = useCallback((nodes: WikiTreeNode[], q: string): WikiTreeNode[] => {
    if (!q) return nodes;
    const lower = q.toLowerCase();
    return nodes.reduce<WikiTreeNode[]>((acc, node) => {
      if (node.is_dir) {
        const filtered = filterTree(node.children || [], q);
        if (filtered.length > 0) acc.push({ ...node, children: filtered });
      } else if (node.name.toLowerCase().includes(lower) || node.path.toLowerCase().includes(lower)) {
        acc.push(node);
      }
      return acc;
    }, []);
  }, []);

  const displayTree = searchQuery && !searchResults.length ? filterTree(fullTree, searchQuery) : fullTree;

  const renderTreeNode = (node: WikiTreeNode, depth = 0) => {
    if (node.is_dir) {
      const isExpanded = expandedDirs.has(node.path) || !!searchQuery;
      const icon = categoryIcons[node.name] || "";
      const hasLinkedChildren = link.wikiPaths.some((p) => p.startsWith(node.path + "/"));
      return (
        <div key={node.path}>
          <button
            onClick={() => toggleDir(node.path)}
            className={cn(
              "flex items-center gap-1 w-full px-1.5 py-1 rounded text-left hover:bg-muted/10 transition-colors",
              hasLinkedChildren && "text-foreground/80"
            )}
            style={{ paddingLeft: `${depth * 12 + 6}px` }}
          >
            {isExpanded ? (
              <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/40 flex-shrink-0" />
            ) : (
              <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/40 flex-shrink-0" />
            )}
            {icon ? (
              <span className="text-[10px] flex-shrink-0">{icon}</span>
            ) : (
              <BookOpen className="w-3 h-3 text-muted-foreground/30 flex-shrink-0" />
            )}
            <span className={cn("text-[10px] truncate", hasLinkedChildren ? "text-foreground/70 font-medium" : "text-muted-foreground/50")}>
              {node.name}
            </span>
            {hasLinkedChildren && (
              <span className="text-[7px] text-accent/50 ml-auto flex-shrink-0">linked</span>
            )}
          </button>
          {isExpanded && node.children?.map((child) => renderTreeNode(child, depth + 1))}
        </div>
      );
    }

    const isLinked = linkedSet.has(node.path);
    const isSelected = selectedPath === node.path;
    return (
      <div key={node.path} className="group/file flex items-center">
        <button
          onClick={() => setSelectedPath(node.path)}
          className={cn(
            "flex items-center gap-1.5 flex-1 px-1.5 py-1 rounded text-left transition-colors",
            isSelected
              ? "bg-accent/15 text-accent"
              : isLinked
                ? "text-foreground/70 hover:bg-muted/10"
                : "text-muted-foreground/40 hover:bg-muted/10 hover:text-muted-foreground/60"
          )}
          style={{ paddingLeft: `${depth * 12 + 6}px` }}
        >
          <FileText className={cn("w-3 h-3 flex-shrink-0", isSelected ? "text-accent" : isLinked ? "text-blue-400/60" : "text-muted-foreground/25")} />
          <span className="text-[10px] truncate flex-1">{node.name.replace(/\.md$/, "")}</span>
          {isLinked && <span className="w-1.5 h-1.5 rounded-full bg-accent/60 flex-shrink-0" />}
        </button>
        {!isLinked && (
          <button
            onClick={() => handleLinkWiki(node.path)}
            className="p-0.5 text-accent/0 group-hover/file:text-accent/50 hover:!text-accent transition-colors flex-shrink-0"
            title="Link to this CVE"
          >
            <Link2 className="w-2.5 h-2.5" />
          </button>
        )}
      </div>
    );
  };

  const selectedContent = selectedPath ? articleContents[selectedPath] || "" : "";
  const selectedTitle = selectedContent ? extractTitle(selectedContent) || selectedPath?.split("/").pop()?.replace(/\.md$/, "") : null;
  const selectedStatus = selectedContent ? extractStatus(selectedContent) : null;
  const selectedBody = selectedContent ? stripFrontmatter(selectedContent) : "";
  const selectedTags = selectedContent ? extractTags(selectedContent) : [];
  const selectedCves = selectedContent ? extractCves(selectedContent) : [];
  const isEditing = editingPath === selectedPath;

  return (
    <div className="flex h-full" style={{ minHeight: "300px" }}>
      {/* Left: directory tree */}
      <div className="w-[200px] flex-shrink-0 border-r border-border/10 flex flex-col">
        {/* Tree header with actions */}
        <div className="flex items-center justify-between px-2 py-1.5 border-b border-border/5">
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Wiki</span>
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => { setCreating(!creating); setAdding(false); }}
              className="p-0.5 text-muted-foreground/30 hover:text-emerald-400 transition-colors"
              title="Create new wiki page"
            >
              <Plus className="w-3 h-3" />
            </button>
            <button
              onClick={() => { setAdding(!adding); setCreating(false); }}
              className="p-0.5 text-muted-foreground/30 hover:text-accent transition-colors"
              title="Link existing wiki article"
            >
              <Link2 className="w-3 h-3" />
            </button>
          </div>
        </div>

        {/* Search bar */}
        <div className="px-2 py-1.5 border-b border-border/5">
          <div className="relative">
            <Search className="absolute left-1.5 top-1/2 -translate-y-1/2 w-2.5 h-2.5 text-muted-foreground/25" />
            <input
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search wiki..."
              className="w-full h-5 pl-5 pr-1.5 text-[9px] bg-[var(--bg-hover)]/30 rounded border border-border/10 text-foreground placeholder:text-muted-foreground/25 outline-none focus:border-accent/30"
            />
            {searching && <Loader2 className="absolute right-1.5 top-1/2 -translate-y-1/2 w-2.5 h-2.5 animate-spin text-muted-foreground/30" />}
          </div>
        </div>

        {/* Create new page form */}
        {creating && (
          <div className="px-2 py-1.5 border-b border-border/5 space-y-1">
            <input
              value={createPath}
              onChange={(e) => setCreatePath(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") handleCreatePage(); }}
              placeholder="products/myapp/CVE-XXXX.md"
              className="w-full h-5 px-1.5 text-[9px] font-mono bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
              autoFocus
            />
            <div className="flex gap-1">
              <button onClick={handleCreatePage} disabled={!createPath.trim()}
                className="text-[8px] text-emerald-400 disabled:opacity-30">Create</button>
              <button onClick={() => { setCreating(false); setCreatePath(""); }}
                className="text-[8px] text-muted-foreground/30">Cancel</button>
            </div>
          </div>
        )}

        {/* Manual link input */}
        {adding && (
          <div className="px-2 py-1.5 border-b border-border/5 space-y-1">
            <input
              value={newPath}
              onChange={(e) => setNewPath(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && newPath.trim()) {
                  handleLinkWiki(newPath.trim());
                  setAdding(false);
                  setNewPath("");
                }
              }}
              placeholder="Path to link..."
              className="w-full h-5 px-1.5 text-[9px] font-mono bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
              autoFocus
            />
            <div className="flex gap-1">
              <button onClick={() => { handleLinkWiki(newPath.trim()); setAdding(false); setNewPath(""); }}
                disabled={!newPath.trim()} className="text-[8px] text-accent disabled:opacity-30">Link</button>
              <button onClick={() => { setAdding(false); setNewPath(""); }}
                className="text-[8px] text-muted-foreground/30">Cancel</button>
            </div>
          </div>
        )}

        {/* DB search results */}
        {searchQuery && searchResults.length > 0 && (
          <div className="border-b border-border/5 max-h-32 overflow-y-auto">
            <div className="px-2 py-0.5">
              <span className="text-[7px] text-muted-foreground/25 uppercase">DB Results</span>
            </div>
            {searchResults.map((r) => (
              <button
                key={r.path}
                onClick={() => navigateToWikiPage(r.path)}
                className="flex flex-col w-full px-2 py-1 hover:bg-muted/10 transition-colors text-left"
              >
                <span className="text-[9px] text-foreground/70 truncate">{r.title}</span>
                <span className="text-[7px] text-muted-foreground/30 truncate">{r.snippet}</span>
              </button>
            ))}
          </div>
        )}

        {/* Tree content */}
        <div className="flex-1 overflow-y-auto py-1">
          {loadingTree ? (
            <div className="flex items-center justify-center py-6">
              <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/20" />
            </div>
          ) : displayTree.length === 0 ? (
            <div className="text-[9px] text-muted-foreground/20 text-center py-6">
              {searchQuery ? "No matches" : "No wiki pages yet"}
            </div>
          ) : (
            displayTree.map((node) => renderTreeNode(node))
          )}
        </div>

        {/* Linked count */}
        {link.wikiPaths.length > 0 && (
          <div className="px-2 py-1 border-t border-border/5">
            <span className="text-[8px] text-accent/40">
              {link.wikiPaths.length} linked
            </span>
          </div>
        )}
      </div>

      {/* Right: article content */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {selectedPath ? (
          <>
            {/* Article header */}
            <div className="flex items-center gap-2 px-3 py-2 border-b border-border/5">
              <FileText className="w-3.5 h-3.5 text-blue-400/60 flex-shrink-0" />
              <span className="text-[11px] text-foreground/80 font-medium truncate flex-1">
                {selectedTitle || selectedPath}
              </span>
              {selectedStatus && (
                <span className={cn("text-[8px] px-1.5 py-0.5 rounded", statusColors[selectedStatus] || "text-muted-foreground/40 bg-muted/10")}>
                  {selectedStatus}
                </span>
              )}
              {linkedSet.has(selectedPath) ? (
                <button onClick={() => handleUnlinkWiki(selectedPath)}
                  className="text-[8px] px-1.5 py-0.5 rounded text-destructive/50 hover:text-destructive hover:bg-destructive/10 transition-colors">
                  Unlink
                </button>
              ) : (
                <button onClick={() => handleLinkWiki(selectedPath)}
                  className="text-[8px] px-1.5 py-0.5 rounded text-accent/50 hover:text-accent hover:bg-accent/10 transition-colors">
                  Link
                </button>
              )}
              {isEditing ? (
                <>
                  <button onClick={handleSaveEdit}
                    className="text-[9px] px-2 py-0.5 rounded bg-emerald-500/15 text-emerald-400 hover:bg-emerald-500/25 transition-colors">
                    Save
                  </button>
                  <button onClick={() => setEditingPath(null)}
                    className="text-[9px] px-2 py-0.5 rounded text-muted-foreground/40 hover:text-muted-foreground/60 transition-colors">
                    Cancel
                  </button>
                </>
              ) : (
                <>
                  <button onClick={() => handleStartEdit(selectedPath)}
                    className="text-[9px] px-2 py-0.5 rounded text-muted-foreground/30 hover:text-accent transition-colors">
                    Edit
                  </button>
                  <button onClick={() => handleDeletePage(selectedPath)}
                    className="text-[9px] px-2 py-0.5 rounded text-destructive/30 hover:text-destructive transition-colors">
                    Delete
                  </button>
                </>
              )}
            </div>

            {/* Metadata row: path + tags + related CVEs */}
            <div className="px-3 py-1.5 border-b border-border/3 space-y-1">
              <span className="text-[8px] font-mono text-muted-foreground/25">{selectedPath}</span>
              {(selectedTags.length > 0 || selectedCves.length > 0) && (
                <div className="flex items-center gap-1 flex-wrap">
                  {selectedTags.map((tag) => (
                    <span key={tag} className="text-[7px] px-1 py-0.5 rounded bg-accent/10 text-accent/60">{tag}</span>
                  ))}
                  {selectedCves.filter((c) => c !== cveId).map((c) => (
                    <span key={c} className="text-[7px] px-1 py-0.5 rounded bg-blue-500/10 text-blue-400/60">{c}</span>
                  ))}
                </div>
              )}
            </div>

            {/* Article body */}
            {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions */}
            <div className="flex-1 overflow-y-auto px-3 py-3" onClick={handleContentClick}>
              {articleContents[selectedPath] === undefined ? (
                <div className="flex items-center justify-center py-8">
                  <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/20" />
                </div>
              ) : isEditing ? (
                <textarea
                  value={editContent}
                  onChange={(e) => setEditContent(e.target.value)}
                  className="w-full h-full min-h-[200px] p-2 rounded bg-[var(--bg-hover)]/30 border border-border/15 text-[10px] font-mono text-foreground/80 resize-y outline-none focus:border-accent/40"
                  spellCheck={false}
                />
              ) : selectedBody ? (
                <div className={proseClasses}>
                  <Markdown content={selectedBody} />
                </div>
              ) : (
                <div className="text-[10px] text-muted-foreground/20 py-8 text-center">Empty article</div>
              )}
            </div>
          </>
        ) : (
          <div className="flex-1 flex flex-col items-center justify-center gap-2 text-muted-foreground/15">
            <BookOpen className="w-10 h-10" />
            <p className="text-[10px]">Select a wiki article from the tree</p>
            {link.wikiPaths.length === 0 && (
              <p className="text-[9px] text-muted-foreground/10">
                Run AI Research to generate wiki articles for this vulnerability
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

interface GithubPocResult {
  full_name: string;
  html_url: string;
  description: string | null;
  language: string | null;
  stars: number;
  updated_at: string;
  topics: string[];
}

interface NucleiTemplateResult {
  name: string;
  path: string;
  html_url: string;
  content: string | null;
  severity: string | null;
}

function PocTab({ link, cveId, onUpdateLink }: { link: VulnLink; cveId: string; onUpdateLink: (updater: (l: VulnLink) => VulnLink) => void }) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<PocTemplate | null>(null);
  const [formName, setFormName] = useState("");
  const [formType, setFormType] = useState<PocTemplate["type"]>("nuclei");
  const [formLang, setFormLang] = useState("yaml");
  const [formContent, setFormContent] = useState("");
  const [expandedPoc, setExpandedPoc] = useState<string | null>(null);
  const [ghResults, setGhResults] = useState<GithubPocResult[]>([]);
  const [ghSearching, setGhSearching] = useState(false);
  const [ghSearched, setGhSearched] = useState(false);
  const [ghError, setGhError] = useState<string | null>(null);

  const [nucleiResults, setNucleiResults] = useState<NucleiTemplateResult[]>([]);
  const [nucleiSearching, setNucleiSearching] = useState(false);
  const [nucleiSearched, setNucleiSearched] = useState(false);
  const [nucleiError, setNucleiError] = useState<string | null>(null);
  const [nucleiImporting, setNucleiImporting] = useState<string | null>(null);

  const searchGithubPoc = useCallback(async () => {
    setGhSearching(true);
    setGhError(null);
    try {
      const results = await invoke<GithubPocResult[]>("intel_search_github_poc", { cveId });
      setGhResults(results);
    } catch (e) {
      setGhError(String(e));
      setGhResults([]);
    }
    setGhSearching(false);
    setGhSearched(true);
  }, [cveId]);

  const searchNucleiTemplates = useCallback(async () => {
    setNucleiSearching(true);
    setNucleiError(null);
    try {
      const results = await invoke<NucleiTemplateResult[]>("intel_search_nuclei_templates", { cveId });
      setNucleiResults(results);
    } catch (e) {
      setNucleiError(String(e));
      setNucleiResults([]);
    }
    setNucleiSearching(false);
    setNucleiSearched(true);
  }, [cveId]);

  const importNucleiTemplate = useCallback(async (template: NucleiTemplateResult) => {
    if (!template.content) return;
    setNucleiImporting(template.name);
    try {
      const dbPoc = await invoke<{ id: string; name: string; type: string; language: string; content: string; created: number }>(
        "vuln_link_add_poc",
        { cveId, name: `[Nuclei] ${template.name}`, pocType: "nuclei", language: "yaml", content: template.content }
      );
      onUpdateLink((l) => ({
        ...l,
        pocTemplates: [...l.pocTemplates, {
          id: dbPoc.id,
          name: dbPoc.name,
          type: dbPoc.type as PocTemplate["type"],
          language: dbPoc.language,
          content: dbPoc.content,
          created: dbPoc.created,
        }],
      }));
    } catch (e) {
      console.error("Failed to import nuclei template:", e);
    }
    setNucleiImporting(null);
  }, [cveId, onUpdateLink]);

  const importAllNucleiTemplates = useCallback(async () => {
    const importable = nucleiResults.filter((t) => t.content);
    for (const template of importable) {
      await importNucleiTemplate(template);
    }
  }, [nucleiResults, importNucleiTemplate]);

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
    const isNew = !editing;
    if (isNew) {
      invoke<{ id: string; name: string; type: string; language: string; content: string; created: number }>(
        "vuln_link_add_poc",
        { cveId, name: formName.trim(), pocType: formType, language: formLang, content: formContent }
      ).then((dbPoc) => {
        onUpdateLink((l) => ({
          ...l,
          pocTemplates: [...l.pocTemplates, {
            id: dbPoc.id,
            name: dbPoc.name,
            type: dbPoc.type as PocTemplate["type"],
            language: dbPoc.language,
            content: dbPoc.content,
            created: dbPoc.created,
          }],
        }));
      }).catch(console.error);
    } else {
      invoke("vuln_link_update_poc", { pocId: editing.id, name: formName.trim(), content: formContent }).catch(console.error);
      onUpdateLink((l) => ({
        ...l,
        pocTemplates: l.pocTemplates.map((p) =>
          p.id === editing.id ? { ...p, name: formName.trim(), content: formContent } : p
        ),
      }));
    }
    setEditing(null);
    setFormName("");
    setFormContent("");
  }, [editing, formName, formType, formLang, formContent, onUpdateLink, cveId]);

  const handleDeletePoc = useCallback((id: string) => {
    onUpdateLink((l) => ({ ...l, pocTemplates: l.pocTemplates.filter((p) => p.id !== id) }));
    invoke("vuln_link_remove_poc", { pocId: id }).catch(console.error);
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

      {/* GitHub PoC Search */}
      <div className="border border-border/10 rounded-lg overflow-hidden">
        <div className="flex items-center justify-between px-3 py-2">
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">GitHub PoC</span>
          <button
            onClick={searchGithubPoc}
            disabled={ghSearching}
            className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors disabled:opacity-30"
          >
            {ghSearching ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : <Search className="w-2.5 h-2.5" />}
            {ghSearched ? "Refresh" : "Search GitHub"}
          </button>
        </div>
        {ghError && (
          <div className="px-3 py-1.5 text-[9px] text-red-400/70 border-t border-border/5">{ghError}</div>
        )}
        {ghSearched && ghResults.length === 0 && !ghError && (
          <div className="px-3 py-2 text-[9px] text-muted-foreground/25 border-t border-border/5">
            No GitHub repositories found for {cveId}
          </div>
        )}
        {ghResults.length > 0 && (
          <div className="border-t border-border/5 max-h-48 overflow-y-auto">
            {ghResults.map((repo) => (
              <div key={repo.full_name} className="flex items-start gap-2 px-3 py-2 hover:bg-muted/5 transition-colors border-b border-border/3 last:border-b-0">
                <div className="flex-1 min-w-0">
                  <a href={repo.html_url} target="_blank" rel="noopener noreferrer"
                    className="text-[10px] text-accent/80 hover:text-accent transition-colors font-medium truncate block">
                    {repo.full_name}
                  </a>
                  {repo.description && (
                    <p className="text-[9px] text-muted-foreground/40 truncate mt-0.5">{repo.description}</p>
                  )}
                  <div className="flex items-center gap-2 mt-0.5">
                    {repo.language && <span className="text-[8px] text-muted-foreground/30">{repo.language}</span>}
                    <span className="text-[8px] text-yellow-400/50">★ {repo.stars}</span>
                    <span className="text-[8px] text-muted-foreground/20">{new Date(repo.updated_at).toLocaleDateString()}</span>
                  </div>
                </div>
                <a href={repo.html_url} target="_blank" rel="noopener noreferrer"
                  className="p-1 text-muted-foreground/25 hover:text-accent transition-colors flex-shrink-0">
                  <ExternalLink className="w-3 h-3" />
                </a>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Nuclei Template Search */}
      <div className="border border-border/10 rounded-lg overflow-hidden">
        <div className="flex items-center justify-between px-3 py-2">
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Nuclei Templates</span>
          <div className="flex items-center gap-2">
            {nucleiSearched && nucleiResults.filter((t) => t.content).length > 0 && (
              <button
                onClick={importAllNucleiTemplates}
                className="flex items-center gap-1 text-[9px] text-emerald-400/60 hover:text-emerald-400 transition-colors"
              >
                <Plus className="w-2.5 h-2.5" /> Import All
              </button>
            )}
            <button
              onClick={searchNucleiTemplates}
              disabled={nucleiSearching}
              className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors disabled:opacity-30"
            >
              {nucleiSearching ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : <Search className="w-2.5 h-2.5" />}
              {nucleiSearched ? "Refresh" : "Search"}
            </button>
          </div>
        </div>
        {nucleiError && (
          <div className="px-3 py-1.5 text-[9px] text-red-400/70 border-t border-border/5">{nucleiError}</div>
        )}
        {nucleiSearched && nucleiResults.length === 0 && !nucleiError && (
          <div className="px-3 py-2 text-[9px] text-muted-foreground/25 border-t border-border/5">
            No Nuclei templates found for {cveId}
          </div>
        )}
        {nucleiResults.length > 0 && (
          <div className="border-t border-border/5 max-h-56 overflow-y-auto">
            {nucleiResults.map((tmpl) => {
              const alreadyImported = link.pocTemplates.some((p) => p.name === `[Nuclei] ${tmpl.name}`);
              const severityColor = tmpl.severity === "critical" ? "text-red-400"
                : tmpl.severity === "high" ? "text-orange-400"
                : tmpl.severity === "medium" ? "text-yellow-400"
                : tmpl.severity === "low" ? "text-blue-400"
                : "text-muted-foreground/40";
              return (
                <div key={tmpl.path} className="flex items-start gap-2 px-3 py-2 hover:bg-muted/5 transition-colors border-b border-border/3 last:border-b-0">
                  <Zap className="w-3 h-3 text-orange-400/60 flex-shrink-0 mt-0.5" />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5">
                      <span className="text-[10px] text-foreground/70 font-medium truncate">{tmpl.name}</span>
                      {tmpl.severity && (
                        <span className={cn("text-[8px] px-1 py-0.5 rounded bg-muted/10 font-medium", severityColor)}>
                          {tmpl.severity}
                        </span>
                      )}
                    </div>
                    <p className="text-[9px] text-muted-foreground/35 truncate mt-0.5">{tmpl.path}</p>
                  </div>
                  <div className="flex items-center gap-1 flex-shrink-0">
                    {tmpl.content && !alreadyImported && (
                      <button
                        onClick={() => importNucleiTemplate(tmpl)}
                        disabled={nucleiImporting === tmpl.name}
                        className="px-1.5 py-0.5 rounded text-[8px] font-medium text-accent/70 bg-accent/10 hover:bg-accent/20 transition-colors disabled:opacity-30"
                      >
                        {nucleiImporting === tmpl.name ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : "Import"}
                      </button>
                    )}
                    {alreadyImported && (
                      <span className="text-[8px] text-emerald-400/50 px-1.5">Imported</span>
                    )}
                    <a href={tmpl.html_url} target="_blank" rel="noopener noreferrer"
                      className="p-1 text-muted-foreground/25 hover:text-accent transition-colors">
                      <ExternalLink className="w-3 h-3" />
                    </a>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {link.pocTemplates.length === 0 && !editing ? (
        <div className="flex flex-col items-center justify-center py-4 gap-2 text-muted-foreground/20">
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

interface BatchNucleiResult {
  cve_id: string;
  templates: NucleiTemplateResult[];
  error: string | null;
}

function PocLibraryView({ vulnLinks, onLinksChange, onJumpToCve }: { vulnLinks: Record<string, VulnLink>; onLinksChange: (links: Record<string, VulnLink>) => void; onJumpToCve?: (cveId: string) => void }) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [expandedPoc, setExpandedPoc] = useState<string | null>(null);
  const [filterType, setFilterType] = useState<"all" | "nuclei" | "script" | "manual">("all");
  const [runTarget, setRunTarget] = useState<{ cveId: string; poc: PocTemplate } | null>(null);
  const [targetUrl, setTargetUrl] = useState("");
  const [batchSearching, setBatchSearching] = useState(false);
  const [batchProgress, setBatchProgress] = useState("");
  const [batchFound, setBatchFound] = useState(0);

  const batchSearchNuclei = useCallback(async () => {
    const cveIds = Object.keys(vulnLinks).filter((id) => id.startsWith("CVE-"));
    if (cveIds.length === 0) {
      setBatchProgress("No CVEs found in links. Add CVE entries first.");
      return;
    }
    setBatchSearching(true);
    setBatchFound(0);
    setBatchProgress(`Searching ${cveIds.length} CVEs...`);
    try {
      const results = await invoke<BatchNucleiResult[]>("intel_batch_search_nuclei_templates", { cveIds });
      let imported = 0;
      const next = { ...vulnLinks };
      for (const r of results) {
        for (const tmpl of r.templates) {
          if (!tmpl.content) continue;
          const link = next[r.cve_id];
          if (!link) continue;
          const name = `[Nuclei] ${tmpl.name}`;
          if (link.pocTemplates.some((p) => p.name === name)) continue;
          try {
            const dbPoc = await invoke<{ id: string; name: string; type: string; language: string; content: string; created: number }>(
              "vuln_link_add_poc",
              { cveId: r.cve_id, name, pocType: "nuclei", language: "yaml", content: tmpl.content }
            );
            next[r.cve_id] = {
              ...next[r.cve_id],
              pocTemplates: [...next[r.cve_id].pocTemplates, {
                id: dbPoc.id, name: dbPoc.name, type: dbPoc.type as PocTemplate["type"],
                language: dbPoc.language, content: dbPoc.content, created: dbPoc.created,
              }],
            };
            imported++;
          } catch { /* skip failed imports */ }
        }
      }
      onLinksChange(next);
      setBatchFound(imported);
      setBatchProgress(`Done: ${imported} templates imported from ${results.filter((r) => r.templates.length > 0).length}/${cveIds.length} CVEs`);
    } catch (e) {
      setBatchProgress(`Error: ${String(e)}`);
    }
    setBatchSearching(false);
  }, [vulnLinks, onLinksChange]);

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
      onLinksChange(next);
      invoke("vuln_link_remove_poc", { pocId }).catch(console.error);
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
    onLinksChange(next);
    invoke("vuln_link_add_scan", {
      cveId: runTarget.cveId,
      target: targetUrl.trim(),
      result: "pending",
    }).catch(console.error);

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
        <button
          onClick={batchSearchNuclei}
          disabled={batchSearching}
          className="flex items-center gap-1.5 h-7 px-2.5 text-[10px] font-medium rounded-lg bg-orange-500/10 text-orange-400/70 hover:bg-orange-500/20 hover:text-orange-400 transition-colors disabled:opacity-30"
          title="Batch search Nuclei templates for all CVEs"
        >
          {batchSearching ? <Loader2 className="w-3 h-3 animate-spin" /> : <Zap className="w-3 h-3" />}
          Nuclei Batch
        </button>
        <span className="text-[10px] text-muted-foreground/30">
          {filtered.length} / {allPocs.length} templates
        </span>
      </div>

      {/* Batch progress */}
      {batchProgress && (
        <div className="flex items-center gap-2 px-4 py-1.5 border-b border-border/10 bg-muted/5">
          {batchSearching && <Loader2 className="w-3 h-3 animate-spin text-orange-400/60" />}
          <span className="text-[10px] text-muted-foreground/50">{batchProgress}</span>
          {!batchSearching && batchFound > 0 && <Zap className="w-3 h-3 text-orange-400/50" />}
          {!batchSearching && (
            <button onClick={() => setBatchProgress("")} className="ml-auto p-0.5 text-muted-foreground/30 hover:text-foreground transition-colors">
              <X className="w-3 h-3" />
            </button>
          )}
        </div>
      )}

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
