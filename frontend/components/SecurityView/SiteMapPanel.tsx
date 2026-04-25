import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity, ArrowRight, ChevronRight,
  Download, Globe, Loader2, RefreshCw,
  Search, Send, Trash2, TreePine, X, Zap,
} from "lucide-react";
import { getRootDomain } from "@/lib/domain";
import { cn } from "@/lib/utils";
import { zapGetHistory, zapGetSiteMapData, zapGetMessage } from "@/lib/pentest/zap-api";
import type { SiteMapEntry, SiteMapData } from "@/lib/pentest/zap-api";
import type { HttpHistoryEntry, HttpMessageDetail } from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";
import { useStore } from "@/store";
import { ResizeHandle, methodColor, statusColor } from "./shared";
import { buildRawRequest, DetailTabs } from "./HttpHistoryPanel";
import { ZapContextMenu } from "./ZapContextMenu";

interface SiteTreeNode {
  name: string;
  fullPath: string;
  methods: Set<string>;
  entries: HttpHistoryEntry[];
  children: Map<string, SiteTreeNode>;
  isEndpoint: boolean;
  nodeType?: "domain" | "subdomain" | "api" | "static" | "path";
}

import { formatBytes } from "@/lib/format";

function normalizeHost(raw: string): string {
  let h = raw.toLowerCase().trim();
  h = h.replace(/^https?:\/\//, "");
  h = h.replace(/\/.*$/, "");
  return h;
}

function buildSiteTree(entries: HttpHistoryEntry[]): Map<string, SiteTreeNode> {
  const domainGroups = new Map<string, Map<string, HttpHistoryEntry[]>>();

  for (const entry of entries) {
    const host = normalizeHost(entry.host || entry.url);
    const root = getRootDomain(host);
    if (!domainGroups.has(root)) domainGroups.set(root, new Map());
    const subMap = domainGroups.get(root)!;
    if (!subMap.has(host)) subMap.set(host, []);
    subMap.get(host)!.push(entry);
  }

  const roots = new Map<string, SiteTreeNode>();

  for (const [rootDomain, subdomainMap] of domainGroups) {
    const rootNode: SiteTreeNode = {
      name: rootDomain, fullPath: rootDomain, methods: new Set(), entries: [],
      children: new Map(), isEndpoint: false, nodeType: "domain",
    };

    for (const [host, hostEntries] of subdomainMap) {
      let hostNode = rootNode;
      if (host !== rootDomain) {
        if (!rootNode.children.has(host)) {
          rootNode.children.set(host, {
            name: host, fullPath: host, methods: new Set(), entries: [],
            children: new Map(), isEndpoint: false, nodeType: "subdomain",
          });
        }
        hostNode = rootNode.children.get(host)!;
      }

      for (const entry of hostEntries) {
        rootNode.entries.push(entry);
        rootNode.methods.add(entry.method);
        if (hostNode !== rootNode) {
          hostNode.entries.push(entry);
          hostNode.methods.add(entry.method);
        }

        const pathStr = (entry.path || "/").split("?")[0].split("#")[0];
        const segments = pathStr.split("/").filter(Boolean);

        let current = hostNode;
        let builtPath = host;
        for (let i = 0; i < segments.length; i++) {
          const seg = segments[i];
          builtPath += `/${seg}`;
          if (!current.children.has(seg)) {
            current.children.set(seg, {
              name: seg, fullPath: builtPath, methods: new Set(), entries: [],
              children: new Map(), isEndpoint: false,
            });
          }
          const child = current.children.get(seg)!;
          child.entries.push(entry);
          child.methods.add(entry.method);
          current = child;
        }
        current.isEndpoint = true;
      }
    }

    roots.set(rootDomain, rootNode);
  }
  return roots;
}

export function SiteMapPanel({ onSendToRepeater, onSendToIntruder, onActiveScan, onBatchScan }: { onSendToRepeater: (raw: string) => void; onSendToIntruder?: (raw: string) => void; onActiveScan?: (url: string) => void; onBatchScan?: (urls: string[]) => void }) {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [siteMapData, setSiteMapData] = useState<SiteMapData | null>(null);
  const [allEntries, setAllEntries] = useState<HttpHistoryEntry[]>([]);
  const [selectedEntry, setSelectedEntry] = useState<HttpHistoryEntry | null>(null);
  const [detail, setDetail] = useState<HttpMessageDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [filterMode, setFilterMode] = useState<"all" | "api" | "js" | "captured">("all");
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; entry: HttpHistoryEntry } | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [hideBeforeId, setHideBeforeId] = useState(0);
  const [scanSelection, setScanSelection] = useState<Set<string>>(new Set());

  const entryKey = (e: HttpHistoryEntry) => `${e.method}:${e.url}`;

  const captureMap = useMemo(() => {
    if (!siteMapData?.entries) return new Map<string, SiteMapEntry>();
    const map = new Map<string, SiteMapEntry>();
    for (const entry of Object.values(siteMapData.entries)) {
      const key = `${entry.method}:${(entry.url || "").split("?")[0].split("#")[0]}`;
      map.set(key, entry);
    }
    return map;
  }, [siteMapData]);

  const entries = useMemo(() => allEntries.filter((e) => e.id > hideBeforeId), [allEntries, hideBeforeId]);

  const loadEntries = useCallback(async () => {
    try {
      const dbData = await zapGetSiteMapData(currentProjectPath);
      setSiteMapData(dbData);

      let items: HttpHistoryEntry[] = [];
      try {
        items = await zapGetHistory(0, 2000);
      } catch { /* ZAP not running */ }

      if (items.length === 0 && dbData?.entries) {
        // Fallback: reconstruct entries from persisted DB data
        let idx = 1;
        items = Object.values(dbData.entries).map((e) => ({
          id: idx++,
          method: e.method || "GET",
          url: e.url,
          status_code: e.status_code,
          content_length: e.content_length || 0,
          time_ms: 0,
          timestamp: e.last_seen || e.first_seen || "",
          host: e.host,
          path: e.path || new URL(e.url).pathname,
        }));
      }
      setAllEntries(items);
    } catch { /* ignore */ }
  }, [currentProjectPath]);

  useEffect(() => {
    loadEntries();
    // Listen for backend event when new sitemap data arrives
    let unlisten: (() => void) | null = null;
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen("sitemap-updated", () => loadEntries()).then((fn) => { unlisten = fn; });
    });
    // Fallback: poll every 15s in case events are missed
    intervalRef.current = setInterval(loadEntries, 15000);
    return () => {
      if (unlisten) unlisten();
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [loadEntries]);

  const isCaptured = useCallback((entry: HttpHistoryEntry) => {
    const pathNoQuery = (entry.url || "").split("?")[0].split("#")[0];
    const key = `${entry.method}:${pathNoQuery}`;
    return captureMap.get(key)?.captured ?? false;
  }, [captureMap]);

  const getCaptureInfo = useCallback((entry: HttpHistoryEntry) => {
    const pathNoQuery = (entry.url || "").split("?")[0].split("#")[0];
    const key = `${entry.method}:${pathNoQuery}`;
    return captureMap.get(key);
  }, [captureMap]);

  const filtered = useMemo(() => {
    let items = entries;
    if (filterMode === "js") {
      items = items.filter((e) => /\.(js|jsx|ts|tsx|mjs|cjs|css|woff2?|svg|png|jpg|gif|ico)(\?|$)/i.test(e.path));
    } else if (filterMode === "api") {
      items = items.filter((e) => !(/\.(js|jsx|ts|tsx|mjs|cjs|css|woff2?|svg|png|jpg|gif|ico|html?)(\?|$)/i.test(e.path)));
    } else if (filterMode === "captured") {
      items = items.filter((e) => isCaptured(e));
    }
    if (search.trim()) {
      const q = search.toLowerCase();
      items = items.filter((e) =>
        e.url.toLowerCase().includes(q) || e.host.toLowerCase().includes(q)
      );
    }
    return items;
  }, [entries, search, filterMode, isCaptured]);

  const deduped = useMemo(() => {
    const seen = new Map<string, HttpHistoryEntry>();
    for (const e of filtered) {
      const pathNoQuery = (e.path || "/").split("?")[0].split("#")[0];
      const host = normalizeHost(e.host || e.url);
      const key = `${e.method}:${host}${pathNoQuery}`;
      const existing = seen.get(key);
      if (!existing) {
        seen.set(key, e);
        continue;
      }
      const newIsOk = e.status_code >= 200 && e.status_code < 300;
      const oldIsOk = existing.status_code >= 200 && existing.status_code < 300;
      // Prefer 2xx over non-2xx; among same quality, prefer the most recent
      if ((newIsOk && !oldIsOk) || (newIsOk === oldIsOk && e.id > existing.id)) {
        seen.set(key, e);
      }
    }
    return [...seen.values()];
  }, [filtered]);

  const handleBatchScan = useCallback(() => {
    if (scanSelection.size === 0) return;
    const urls = deduped
      .filter((e) => scanSelection.has(entryKey(e)))
      .map((e) => e.url);
    if (onBatchScan) {
      onBatchScan(urls);
    } else if (onActiveScan) {
      urls.forEach((url) => onActiveScan(url));
    }
    setScanSelection(new Set());
  }, [scanSelection, deduped, onBatchScan, onActiveScan]);

  const tree = useMemo(() => buildSiteTree(deduped), [deduped]);

  const handleSelectEntry = useCallback(async (entry: HttpHistoryEntry) => {
    setSelectedEntry(entry);
    setDetailLoading(true);
    try {
      const msg = await zapGetMessage(entry.id, entry.url);
      if (msg && (msg.request_headers || msg.response_headers)) {
        setDetail(msg);
      } else {
        throw new Error("empty");
      }
    } catch {
      setDetail({
        id: entry.id,
        method: entry.method,
        url: entry.url,
        status_code: entry.status_code,
        request_headers: `${entry.method} ${new URL(entry.url).pathname} HTTP/1.1\r\nHost: ${entry.host}\r\n`,
        request_body: "",
        response_headers: entry.status_code ? `HTTP/1.1 ${entry.status_code}\r\nContent-Length: ${entry.content_length}\r\n` : "",
        response_body: `(Response body not available — ZAP session expired.\nURL: ${entry.url}\nStatus: ${entry.status_code}\nSize: ${entry.content_length} bytes)`,
      });
    } finally {
      setDetailLoading(false);
    }
  }, []);

  const handleCtxSendToRepeater = useCallback(async (entry: HttpHistoryEntry) => {
    try {
      const msg = await zapGetMessage(entry.id, entry.url);
      onSendToRepeater(buildRawRequest(msg));
    } catch { /* ignore */ }
  }, [onSendToRepeater]);

  useEffect(() => {
    if (!ctxMenu) return;
    const close = () => setCtxMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [ctxMenu]);

  const treeScrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!selectedEntry) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") return;
      e.preventDefault();

      const container = treeScrollRef.current;
      if (!container) return;

      const rows = Array.from(container.querySelectorAll<HTMLElement>("[data-entry-id]"));
      const currentIdx = rows.findIndex((r) => r.dataset.entryId === String(selectedEntry.id));
      if (currentIdx === -1) return;

      const nextIdx = e.key === "ArrowDown"
        ? Math.min(currentIdx + 1, rows.length - 1)
        : Math.max(currentIdx - 1, 0);
      if (nextIdx === currentIdx) return;

      const nextId = Number(rows[nextIdx].dataset.entryId);
      const nextEntry = deduped.find((d) => d.id === nextId);
      if (nextEntry) {
        handleSelectEntry(nextEntry);
        rows[nextIdx].scrollIntoView({ block: "nearest", behavior: "smooth" });
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [selectedEntry, deduped, handleSelectEntry]);

  const hostCount = tree.size;
  const endpointCount = deduped.length;
  const capturedCount = useMemo(() => deduped.filter((e) => isCaptured(e)).length, [deduped, isCaptured]);
  const siteContainerRef = useRef<HTMLDivElement>(null);
  const [siteDetailWidth, setSiteDetailWidth] = useState<number | null>(null);

  useEffect(() => {
    if (selectedEntry && siteDetailWidth === null && siteContainerRef.current) {
      setSiteDetailWidth(Math.floor(siteContainerRef.current.offsetWidth / 2));
    }
  }, [selectedEntry, siteDetailWidth]);

  const handleSiteResize = useCallback((delta: number) => {
    const maxW = siteContainerRef.current ? siteContainerRef.current.offsetWidth - 280 : 900;
    setSiteDetailWidth((w) => Math.min(maxW, Math.max(280, (w ?? 520) - delta)));
  }, []);

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border/10 flex-shrink-0">
        <div className="relative flex-1 max-w-xs">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("security.filterSiteMap", "Filter endpoints...")}
            className="w-full h-7 pl-8 pr-3 text-[11px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
          />
        </div>
        {(["all", "api", "js", "captured"] as const).map((mode) => (
          <button
            key={mode}
            type="button"
            onClick={() => setFilterMode(mode)}
            className={cn(
              "px-2.5 py-1 text-[10px] rounded-md font-medium transition-colors",
              filterMode === mode
                ? mode === "api" ? "bg-green-500/15 text-green-400"
                  : mode === "js" ? "bg-yellow-500/15 text-yellow-400"
                  : mode === "captured" ? "bg-blue-500/15 text-blue-400"
                  : "bg-accent/15 text-accent"
                : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]"
            )}
          >
            {mode === "all" ? "All" : mode === "api" ? "API" : mode === "js" ? "Static" : "Saved"}
          </button>
        ))}
        <div className="flex items-center gap-1 px-1.5 py-0.5 rounded-md bg-accent/8 text-accent text-[10px]">
          <TreePine className="w-3 h-3" />
        </div>
        {scanSelection.size > 0 && (
          <button
            type="button"
            onClick={handleBatchScan}
            className="flex items-center gap-1 px-2.5 py-1 text-[10px] font-medium rounded-md bg-orange-500/15 text-orange-400 hover:bg-orange-500/25 transition-colors"
          >
            <Zap className="w-3 h-3" />
            Scan {scanSelection.size} selected
          </button>
        )}
        <span className="text-[10px] text-muted-foreground/50">
          {hostCount} {t("security.hosts", "hosts")} · {endpointCount} {t("security.endpoints", "endpoints")}{capturedCount > 0 && <> · <span className="text-blue-400">{capturedCount} saved</span></>}
        </span>
        <button
          type="button"
          onClick={() => {
            if (!confirm(t("security.clearSiteMapConfirm", "Clear all site map data?"))) return;
            const maxId = allEntries.reduce((m, e) => Math.max(m, e.id), 0);
            setHideBeforeId(maxId);
            setSelectedEntry(null);
            setDetail(null);
          }}
          title={t("security.clearSiteMap", "Clear site map")}
          className="p-1.5 rounded-md text-muted-foreground/40 hover:text-destructive hover:bg-destructive/10 transition-colors"
        >
          <Trash2 className="w-3 h-3" />
        </button>
        <button
          type="button"
          onClick={loadEntries}
          className="p-1.5 rounded-md text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          <RefreshCw className="w-3 h-3" />
        </button>
      </div>

      <div ref={siteContainerRef} className="flex-1 flex min-h-0">
        <div ref={treeScrollRef} className="flex-1 overflow-y-auto min-w-0">
          {deduped.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/40">
              <Globe className="w-8 h-8" />
              <p className="text-[11px]">{t("security.noSiteData", "No site data yet")}</p>
            </div>
          ) : (
            <div className="py-1">
              {[...tree.entries()].map(([host, node]) => (
                <SiteTreeNodeView
                  key={host}
                  node={node}
                  depth={0}
                  onSelect={handleSelectEntry}
                  selectedId={selectedEntry?.id ?? null}
                  onContextMenu={(e, entry) => {
                    e.preventDefault();
                    setCtxMenu({ x: e.clientX, y: e.clientY, entry });
                  }}
                />
              ))}
            </div>
          )}
        </div>

        {(selectedEntry || detailLoading) && <ResizeHandle onResize={handleSiteResize} />}

        <div className="flex flex-col overflow-hidden" style={{ width: (selectedEntry || detailLoading) ? (siteDetailWidth ?? 0) : 0, flexShrink: 0 }}>
          {selectedEntry && detail ? (
            <div className="flex-1 flex flex-col min-h-0">
              <div className="flex items-center gap-2 px-3 py-2 border-b border-border/10 flex-shrink-0">
                <span className={cn("text-[10px] font-mono font-bold", methodColor(selectedEntry.method))}>
                  {selectedEntry.method}
                </span>
                <span className="text-[11px] font-mono text-foreground/70 truncate flex-1">
                  {selectedEntry.url}
                </span>
                <span className={cn("text-[10px] font-mono", statusColor(selectedEntry.status_code))}>
                  {selectedEntry.status_code}
                </span>
                {isCaptured(selectedEntry) && (
                  <span className="flex items-center gap-1 px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 text-[9px] font-medium">
                    <Download className="w-2.5 h-2.5" />
                    Saved
                    {getCaptureInfo(selectedEntry)?.capture?.file_size != null && (
                      <span className="text-blue-400/60">
                        ({formatBytes(getCaptureInfo(selectedEntry)!.capture!.file_size)})
                      </span>
                    )}
                  </span>
                )}
                <button
                  type="button"
                  onClick={() => onSendToRepeater(buildRawRequest(detail))}
                  className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium text-muted-foreground/60 hover:text-accent hover:bg-accent/10 transition-colors"
                >
                  <Send className="w-3 h-3" />
                  {t("security.sendToRepeater")}
                </button>
                <button
                  type="button"
                  onClick={() => { setSelectedEntry(null); setDetail(null); }}
                  className="p-1 rounded text-muted-foreground/30 hover:text-foreground transition-colors"
                >
                  <X className="w-3 h-3" />
                </button>
              </div>
              <DetailTabs detail={detail} />
            </div>
          ) : detailLoading ? (
            <div className="flex-1 flex items-center justify-center">
              <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/50" />
            </div>
          ) : (
            <div className="flex-1 flex items-center justify-center text-muted-foreground/40">
              <div className="flex flex-col items-center gap-2">
                <Search className="w-8 h-8" />
                <p className="text-[12px]">{t("security.selectEndpoint", "Select an endpoint to view details")}</p>
              </div>
            </div>
          )}
        </div>
      </div>
      {ctxMenu && (
        <ZapContextMenu
          x={ctxMenu.x}
          y={ctxMenu.y}
          entry={ctxMenu.entry}
          onClose={() => setCtxMenu(null)}
          onSendToRepeater={(e) => handleCtxSendToRepeater(e as any)}
          onSendToIntruder={onSendToIntruder ? async (e) => {
            try { const msg = await zapGetMessage(e.id, e.url); onSendToIntruder(buildRawRequest(msg)); } catch { /* ignore */ }
          } : undefined}
          onActiveScan={onActiveScan}
        />
      )}
    </div>
  );
}

function SiteTreeNodeView({
  node, depth, onSelect, selectedId, onContextMenu,
}: {
  node: SiteTreeNode; depth: number;
  onSelect: (e: HttpHistoryEntry) => void; selectedId: number | null;
  onContextMenu?: (e: React.MouseEvent, entry: HttpHistoryEntry) => void;
}) {
  const [open, setOpen] = useState(false);
  const hasChildren = node.children.size > 0;
  const isJs = /\.(js|jsx|ts|tsx|mjs|cjs)(\?|$)/i.test(node.name);
  const isApi = /^(api|v\d|graphql|rest)/i.test(node.name);

  const latestEntry = node.entries[node.entries.length - 1];
  const isSelected = latestEntry && selectedId === latestEntry.id;

  return (
    <div>
      <div
        data-entry-id={latestEntry?.id}
        className={cn(
          "flex items-center gap-1 py-0.5 pr-2 cursor-pointer transition-colors text-[11px] hover:bg-[var(--bg-hover)]/40",
          isSelected && "!bg-[#7aa2f7]/20 border-l-2 !border-l-[#7aa2f7] !text-[#c0caf5]",
        )}
        style={{ paddingLeft: `${depth * 16 + 8}px` }}
        onClick={() => {
          if (hasChildren) setOpen(!open);
          if (node.isEndpoint && latestEntry) onSelect(latestEntry);
        }}
        onContextMenu={(e) => {
          if (latestEntry && onContextMenu) onContextMenu(e, latestEntry);
        }}
      >
        {hasChildren ? (
          <ChevronRight className={cn("w-3 h-3 text-muted-foreground/40 transition-transform flex-shrink-0", open && "rotate-90")} />
        ) : (
          <span className="w-3 flex-shrink-0" />
        )}
        {node.nodeType === "domain" ? (
          <Globe className="w-3 h-3 text-blue-400 flex-shrink-0" />
        ) : node.nodeType === "subdomain" ? (
          <Globe className="w-3 h-3 text-cyan-400 flex-shrink-0" />
        ) : isJs ? (
          <Activity className="w-3 h-3 text-yellow-400 flex-shrink-0" />
        ) : isApi ? (
          <ArrowRight className="w-3 h-3 text-green-400 flex-shrink-0" />
        ) : (
          <span className="w-3 h-3 flex items-center justify-center text-muted-foreground/40 flex-shrink-0">
            {node.isEndpoint ? "·" : "/"}
          </span>
        )}
        <span className={cn(
          "truncate flex-1",
          node.nodeType === "domain" ? "font-medium text-foreground/80"
            : node.nodeType === "subdomain" ? "font-medium text-foreground/70"
            : "text-foreground/60",
        )}>
          {node.nodeType === "domain" || node.nodeType === "subdomain" ? node.name : `/${node.name}`}
        </span>
        <span className="flex items-center gap-0.5 flex-shrink-0">
          {[...node.methods].map((m) => (
            <span key={m} className={cn("text-[8px] font-mono font-bold px-1 rounded", methodColor(m))}>
              {m}
            </span>
          ))}
        </span>
        <span className="text-[9px] text-muted-foreground/50 ml-1 flex-shrink-0">
          {node.entries.length > 1 ? `${node.entries.length}` : ""}
        </span>
      </div>
      {open && hasChildren && (
        <div>
          {[...node.children.entries()]
            .sort(([a], [b]) => a.localeCompare(b))
            .map(([name, child]) => (
              <SiteTreeNodeView
                key={name}
                node={child}
                depth={depth + 1}
                onSelect={onSelect}
                selectedId={selectedId}
                onContextMenu={onContextMenu}
              />
            ))}
        </div>
      )}
    </div>
  );
}

// ── Audit Log Panel ──


