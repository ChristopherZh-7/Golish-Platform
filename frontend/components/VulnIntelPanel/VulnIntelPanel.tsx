import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  Bell,
  ExternalLink,
  Plus,
  RefreshCw,
  Search,
  Shield,
  Trash2,
  X,
  Crosshair,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";

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

type ViewMode = "feed" | "matched" | "feeds-config";

export function VulnIntelPanel() {
  const [entries, setEntries] = useState<VulnEntry[]>([]);
  const [matchedEntries, setMatchedEntries] = useState<VulnEntry[]>([]);
  const [feeds, setFeeds] = useState<VulnFeed[]>([]);
  const [loading, setLoading] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>("feed");
  const [searchQuery, setSearchQuery] = useState("");
  const [expandedCve, setExpandedCve] = useState<string | null>(null);
  const [showAddFeed, setShowAddFeed] = useState(false);
  const [newFeed, setNewFeed] = useState({ name: "", feed_type: "custom", url: "" });

  const loadCached = useCallback(async () => {
    try {
      const cached = await invoke<VulnEntry[]>("intel_get_cached");
      setEntries(cached);
    } catch { /* ignore */ }
  }, []);

  const loadFeeds = useCallback(async () => {
    try {
      const f = await invoke<VulnFeed[]>("intel_list_feeds");
      setFeeds(f);
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
      setEntries(result);
      loadFeeds();
    } catch (e) {
      console.error("Fetch failed:", e);
    }
    setLoading(false);
  }, [loadFeeds]);

  const handleSearch = useCallback(async () => {
    if (!searchQuery.trim()) {
      loadCached();
      return;
    }
    try {
      const results = await invoke<VulnEntry[]>("intel_search", { query: searchQuery.trim() });
      setEntries(results);
    } catch { /* ignore */ }
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
      setNewFeed({ name: "", feed_type: "custom", url: "" });
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

  const displayEntries = viewMode === "matched" ? matchedEntries : entries;

  return (
    <div className="h-full flex flex-col bg-background/95">
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/20">
        <Shield className="w-3.5 h-3.5 text-accent/70" />
        <span className="text-[11px] font-medium flex-1">Vulnerability Intelligence</span>
        <span className="text-[9px] text-muted-foreground/40">
          {displayEntries.length} {viewMode === "matched" ? "matched" : "entries"}
        </span>
      </div>

      {/* Tab bar + actions */}
      <div className="flex items-center gap-1 px-3 py-1.5 border-b border-border/10">
        <button
          onClick={() => setViewMode("feed")}
          className={cn(
            "text-[9px] px-2 py-0.5 rounded transition-colors",
            viewMode === "feed" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground"
          )}
        >
          <Bell className="w-2.5 h-2.5 inline mr-1" />
          Feed
        </button>
        <button
          onClick={handleMatchTargets}
          className={cn(
            "text-[9px] px-2 py-0.5 rounded transition-colors",
            viewMode === "matched" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground"
          )}
        >
          <Crosshair className="w-2.5 h-2.5 inline mr-1" />
          Match Targets
        </button>
        <button
          onClick={() => setViewMode("feeds-config")}
          className={cn(
            "text-[9px] px-2 py-0.5 rounded transition-colors",
            viewMode === "feeds-config" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground"
          )}
        >
          Feeds
        </button>
        <div className="flex-1" />
        <div className="flex items-center gap-1 bg-background border border-border/20 rounded px-1.5">
          <Search className="w-2.5 h-2.5 text-muted-foreground/30" />
          <input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSearch()}
            placeholder="Search CVEs..."
            className="text-[10px] py-0.5 bg-transparent outline-none w-28"
          />
          {searchQuery && (
            <button onClick={() => { setSearchQuery(""); loadCached(); }} className="text-muted-foreground/30 hover:text-foreground">
              <X className="w-2.5 h-2.5" />
            </button>
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
      {viewMode === "feeds-config" && (
        <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
          {feeds.map((feed) => (
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
      )}

      {/* Entries list */}
      {viewMode !== "feeds-config" && (
        <div className="flex-1 overflow-y-auto px-3 py-2 space-y-0.5">
          {displayEntries.length === 0 ? (
            <div className="text-center text-[11px] text-muted-foreground/30 py-12">
              {loading ? "Fetching vulnerability data..." : viewMode === "matched" ? "No matched vulnerabilities" : "Click refresh to fetch latest CVEs"}
            </div>
          ) : (
            displayEntries.map((entry) => (
              <div key={entry.cve_id} className="group">
                <div
                  className="flex items-start gap-2 py-1.5 px-2 rounded hover:bg-muted/5 transition-colors cursor-pointer"
                  onClick={() => setExpandedCve(expandedCve === entry.cve_id ? null : entry.cve_id)}
                >
                  <span className={cn("w-1.5 h-1.5 rounded-full mt-1.5 flex-shrink-0", SEV_DOT[entry.severity] || "bg-slate-500")} />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5">
                      <span className="text-[10px] font-mono font-medium text-accent/80">{entry.cve_id}</span>
                      <span className={cn("text-[8px] px-1.5 py-0.5 rounded-full border capitalize",
                        SEV_COLORS[entry.severity] || SEV_COLORS.info
                      )}>
                        {entry.severity}
                        {entry.cvss_score != null && ` (${entry.cvss_score})`}
                      </span>
                      <span className="text-[8px] text-muted-foreground/20">{entry.source}</span>
                    </div>
                    <div className="text-[10px] text-foreground/70 truncate mt-0.5">{entry.title}</div>
                    {entry.affected_products.length > 0 && (
                      <div className="text-[8px] text-muted-foreground/30 mt-0.5">
                        {entry.affected_products.join(", ")}
                      </div>
                    )}
                  </div>
                  <span className="text-[8px] text-muted-foreground/20 whitespace-nowrap">
                    {entry.published.slice(0, 10)}
                  </span>
                </div>

                {expandedCve === entry.cve_id && (
                  <div className="mx-2 mb-2 px-3 py-2.5 rounded border border-border/20 bg-muted/5 space-y-2.5">
                    {/* Meta grid */}
                    <div className="grid grid-cols-2 gap-x-4 gap-y-1.5">
                      <div>
                        <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">CVE ID</span>
                        <div className="text-[10px] font-mono text-accent">{entry.cve_id}</div>
                      </div>
                      <div>
                        <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">CVSS Score</span>
                        <div className="flex items-center gap-1.5 mt-0.5">
                          <span className={cn(
                            "text-[11px] font-bold",
                            (entry.cvss_score ?? 0) >= 9 ? "text-red-400" :
                            (entry.cvss_score ?? 0) >= 7 ? "text-orange-400" :
                            (entry.cvss_score ?? 0) >= 4 ? "text-yellow-400" : "text-blue-400"
                          )}>
                            {entry.cvss_score != null ? entry.cvss_score.toFixed(1) : "N/A"}
                          </span>
                          <span className={cn(
                            "text-[8px] px-1.5 py-0.5 rounded border capitalize",
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
                      <a
                        href={`https://nvd.nist.gov/vuln/detail/${entry.cve_id}`}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors"
                      >
                        <ExternalLink className="w-2.5 h-2.5" />
                        NVD
                      </a>
                      <a
                        href={`https://cve.mitre.org/cgi-bin/cvename.cgi?name=${entry.cve_id}`}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors"
                      >
                        <ExternalLink className="w-2.5 h-2.5" />
                        MITRE
                      </a>
                      <a
                        href={`https://www.google.com/search?q=${entry.cve_id}+exploit+poc`}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="flex items-center gap-1 text-[9px] text-red-400/60 hover:text-red-400 transition-colors"
                      >
                        <AlertTriangle className="w-2.5 h-2.5" />
                        Search PoC
                      </a>
                    </div>

                    {/* References */}
                    {entry.references.length > 0 && (
                      <div>
                        <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">References ({entry.references.length})</span>
                        <div className="space-y-0.5 mt-0.5 max-h-24 overflow-y-auto">
                          {entry.references.map((ref_, i) => (
                            <a
                              key={i}
                              href={ref_}
                              target="_blank"
                              rel="noopener noreferrer"
                              className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors truncate"
                            >
                              <ExternalLink className="w-2.5 h-2.5 flex-shrink-0" />
                              {ref_}
                            </a>
                          ))}
                        </div>
                      </div>
                    )}
                  </div>
                )}
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
