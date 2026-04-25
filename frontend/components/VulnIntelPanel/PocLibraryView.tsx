import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronDown, ChevronRight, Code, Copy, FileCode2, FileText,
  Loader2, Play, Search, Trash2, X, Zap,
} from "lucide-react";
import { copyToClipboard } from "@/lib/clipboard";
import type { VulnLink, PocTemplate, DbVulnLinkFull } from "./types";
import { dbToVulnLink } from "./types";
import { CustomSelect } from "@/components/ui/custom-select";
import { useTranslation } from "react-i18next";
interface NucleiDiscoverResult {
  total_files: number;
  total_cves: number;
  imported: number;
  skipped: number;
  errors: number;
}

let _discoverState = { searching: false, progress: "", found: 0 };

export const PocLibraryView = memo(function PocLibraryView({ vulnLinks, onLinksChange, onJumpToCve }: { vulnLinks: Record<string, VulnLink>; onLinksChange: (links: Record<string, VulnLink>) => void; onJumpToCve?: (cveId: string) => void }) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [expandedPoc, setExpandedPoc] = useState<string | null>(null);
  const [filterType, setFilterType] = useState<"all" | "nuclei" | "script" | "manual">("all");
  const [runTarget, setRunTarget] = useState<{ cveId: string; poc: PocTemplate } | null>(null);
  const [targetUrl, setTargetUrl] = useState("");
  const [batchSearching, setBatchSearching] = useState(_discoverState.searching);
  const [batchProgress, setBatchProgress] = useState(_discoverState.progress);
  const [batchFound, setBatchFound] = useState(_discoverState.found);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen<{ phase: string; current: number; total: number; cve_id: string | null }>(
        "nuclei-discover-progress",
        (e) => {
          const { phase, current, total, cve_id } = e.payload;
          let msg = "";
          if (phase === "listing") {
            msg = "Fetching nuclei-templates file tree...";
          } else if (phase === "listing_fallback") {
            msg = `Scanning directories: ${current}/${total}${cve_id ? ` — ${cve_id}` : ""}`;
          } else if (phase === "downloading") {
            const pct = total > 0 ? Math.round((current / total) * 100) : 0;
            msg = `Downloading templates: ${current}/${total} (${pct}%)${cve_id ? ` — ${cve_id}` : ""}`;
          } else if (phase === "done") {
            msg = `Finished processing ${total} template files.`;
          }
          _discoverState.progress = msg;
          setBatchProgress(msg);
        }
      ).then((fn) => { unlisten = fn; });
    });
    return () => { unlisten?.(); };
  }, []);

  const setSearchingPersist = useCallback((v: boolean) => { _discoverState.searching = v; setBatchSearching(v); }, []);
  const setProgressPersist = useCallback((v: string) => { _discoverState.progress = v; setBatchProgress(v); }, []);
  const setFoundPersist = useCallback((v: number) => { _discoverState.found = v; setBatchFound(v); }, []);

  const batchSearchNuclei = useCallback(async () => {
    setSearchingPersist(true);
    setFoundPersist(0);
    setProgressPersist("Starting full Nuclei template discovery...");
    try {
      const result = await invoke<NucleiDiscoverResult>("intel_discover_all_nuclei");
      setFoundPersist(result.imported);
      setProgressPersist(
        `Done: ${result.imported} imported, ${result.skipped} skipped, ${result.errors} errors — ${result.total_cves} unique CVEs from ${result.total_files} files`
      );
      try {
        const allLinks = await invoke<Record<string, DbVulnLinkFull>>("vuln_link_get_all");
        const converted: Record<string, VulnLink> = {};
        for (const [cveId, db] of Object.entries(allLinks)) {
          converted[cveId] = dbToVulnLink(db);
        }
        onLinksChange(converted);
      } catch { /* refresh failed, user can reload */ }
    } catch (e) {
      setProgressPersist(`Error: ${String(e)}`);
    }
    setSearchingPersist(false);
  }, [onLinksChange, setSearchingPersist, setProgressPersist, setFoundPersist]);

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
      items = items.filter((p) =>
        p.poc.name.toLowerCase().includes(q)
        || p.cveId.toLowerCase().includes(q)
        || p.poc.tags?.some((tag) => tag.toLowerCase().includes(q))
        || p.poc.description?.toLowerCase().includes(q),
      );
    }
    return items;
  }, [allPocs, filterType, search]);

  const PAGE_SIZE = 100;
  const [displayCount, setDisplayCount] = useState(PAGE_SIZE);
  useEffect(() => { setDisplayCount(PAGE_SIZE); }, [filtered]);
  const displayedPocs = useMemo(() => filtered.slice(0, displayCount), [filtered, displayCount]);

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
    copyToClipboard(content);
  }, []);

  const handleRunPoc = useCallback(() => {
    if (!runTarget || !targetUrl.trim()) return;
    const rendered = runTarget.poc.content.replace(/\{\{BaseURL\}\}/g, targetUrl.trim());
    copyToClipboard(rendered);

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
        <CustomSelect
          value={filterType}
          onChange={(v) => setFilterType(v as typeof filterType)}
          options={[
            { value: "all", label: t("vulnKb.allTypes", "All Types") },
            { value: "nuclei", label: "Nuclei" },
            { value: "script", label: "Script" },
            { value: "manual", label: "Manual" },
          ]}
          size="sm"
          className="min-w-[80px]"
        />
        <button
          onClick={batchSearchNuclei}
          disabled={batchSearching}
          className="flex items-center gap-1.5 h-7 px-2.5 text-[10px] font-medium rounded-lg bg-orange-500/10 text-orange-400/70 hover:bg-orange-500/20 hover:text-orange-400 transition-colors disabled:opacity-30"
          title="Discover ALL CVE-related Nuclei templates and auto-import to database"
        >
          {batchSearching ? <Loader2 className="w-3 h-3 animate-spin" /> : <Zap className="w-3 h-3" />}
          Discover All Nuclei
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
            {displayedPocs.map(({ cveId, poc }) => (
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
            {displayCount < filtered.length && (
              <button
                onClick={() => setDisplayCount((c) => c + PAGE_SIZE)}
                className="w-full py-2.5 text-[10px] text-accent/60 hover:text-accent hover:bg-accent/5 transition-colors"
              >
                Show more ({filtered.length - displayCount} remaining)
              </button>
            )}
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
});
