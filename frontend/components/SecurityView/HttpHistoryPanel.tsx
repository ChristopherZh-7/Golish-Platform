import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { js_beautify as jsBeautify, css_beautify as cssBeautify } from "js-beautify";
import {
  ArrowDown, ArrowUp, Copy,
  Loader2, RefreshCw, Search, Send, Trash2, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  zapGetHistory, zapGetHistoryCount, zapGetMessage,
} from "@/lib/pentest/zap-api";
import type { HttpHistoryEntry, HttpMessageDetail } from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";
import { copyToClipboard } from "@/lib/clipboard";
import { formatBytes as formatSize } from "@/lib/format";
import { ResizeHandle, methodColor, statusColor } from "./shared";
import { ZapContextMenu } from "./ZapContextMenu";

export function buildRawRequest(detail: HttpMessageDetail): string {
  let headers = detail.request_headers;
  if (!headers.trim()) {
    const path = (() => { try { return new URL(detail.url).pathname; } catch { return detail.url || "/"; } })();
    headers = `${detail.method || "GET"} ${path} HTTP/1.1\nHost: ${(() => { try { return new URL(detail.url).host; } catch { return "localhost"; } })()}\n`;
  }
  headers = headers.replace(/[\r\n]+$/, "");
  const body = detail.request_body || "";
  return body ? `${headers}\n\n${body}` : `${headers}\n\n`;
}

// ── HTTP History Panel ──

export function HttpHistoryPanel({ onSendToRepeater, onSendToIntruder, onActiveScan }: { onSendToRepeater: (raw: string) => void; onSendToIntruder?: (raw: string) => void; onActiveScan?: (url: string) => void }) {
  const { t } = useTranslation();
  const [allEntries, setAllEntries] = useState<HttpHistoryEntry[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<HttpMessageDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [sortOrder, setSortOrder] = useState<"desc" | "asc">("desc");
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; entry: HttpHistoryEntry } | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [hideBeforeId, setHideBeforeId] = useState(0);

  const entries = useMemo(() => allEntries.filter((e) => e.id > hideBeforeId), [allEntries, hideBeforeId]);

  const loadHistory = useCallback(async () => {
    try {
      const [items, count] = await Promise.all([
        zapGetHistory(0, 200),
        zapGetHistoryCount(),
      ]);
      setAllEntries(items);
      setTotalCount(count);
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    loadHistory();
    let unlisten: (() => void) | null = null;
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen("sitemap-updated", () => loadHistory()).then((fn) => { unlisten = fn; });
    });
    intervalRef.current = setInterval(loadHistory, 15000);
    return () => {
      if (unlisten) unlisten();
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [loadHistory]);

  const handleSelect = useCallback(async (id: number) => {
    setSelectedId(id);
    setLoading(true);
    try {
      const url = allEntries.find((e) => e.id === id)?.url;
      const msg = await zapGetMessage(id, url);
      setDetail(msg);
    } catch {
      setDetail(null);
    } finally {
      setLoading(false);
    }
  }, [allEntries]);

  const handleSendToRepeater = useCallback(async (id: number) => {
    try {
      const url = allEntries.find((e) => e.id === id)?.url;
      const msg = await zapGetMessage(id, url);
      onSendToRepeater(buildRawRequest(msg));
    } catch { /* ignore */ }
  }, [allEntries, onSendToRepeater]);

  const handleClearHistory = useCallback(() => {
    if (!confirm(t("security.clearHistoryConfirm"))) return;
    const maxId = allEntries.reduce((m, e) => Math.max(m, e.id), 0);
    setHideBeforeId(maxId);
    setSelectedId(null);
    setDetail(null);
  }, [t, allEntries]);

  useEffect(() => {
    if (!ctxMenu) return;
    const close = () => setCtxMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [ctxMenu]);

  const filtered = useMemo(() => {
    let items = entries;
    if (search.trim()) {
      const q = search.toLowerCase();
      items = items.filter(
        (e) =>
          e.url.toLowerCase().includes(q) ||
          e.host.toLowerCase().includes(q) ||
          e.method.toLowerCase().includes(q)
      );
    }
    return sortOrder === "desc" ? [...items].reverse() : items;
  }, [entries, search, sortOrder]);

  return (
    <div className="h-full flex flex-col">
      {/* Search bar */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border/10 flex-shrink-0">
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("security.filterHistory")}
            className="w-full h-7 pl-8 pr-3 text-[11px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
          />
        </div>
        <span className="text-[10px] text-muted-foreground/50">
          {filtered.length} / {totalCount} {t("security.requests")}
        </span>
        <button
          type="button"
          onClick={() => setSortOrder((s) => s === "desc" ? "asc" : "desc")}
          title={sortOrder === "desc" ? t("security.newestFirst") : t("security.oldestFirst")}
          className="flex items-center gap-1 p-1.5 rounded-md text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          {sortOrder === "desc" ? <ArrowDown className="w-3 h-3" /> : <ArrowUp className="w-3 h-3" />}
        </button>
        <button
          type="button"
          onClick={handleClearHistory}
          title={t("security.clearHistory")}
          className="p-1.5 rounded-md text-muted-foreground/40 hover:text-destructive hover:bg-destructive/10 transition-colors"
        >
          <Trash2 className="w-3 h-3" />
        </button>
        <button
          type="button"
          onClick={loadHistory}
          className="p-1.5 rounded-md text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          <RefreshCw className="w-3 h-3" />
        </button>
      </div>

      <HistoryBodyPanel
        filtered={filtered}
        entries={entries}
        selectedId={selectedId}
        detail={detail}
        loading={loading}
        onSelect={handleSelect}
        onSendToRepeater={handleSendToRepeater}
        onSendDetailToRepeater={(d) => onSendToRepeater(buildRawRequest(d))}
        onCtxMenu={(x, y, entry) => setCtxMenu({ x, y, entry })}
        onClose={() => { setSelectedId(null); setDetail(null); }}
      />
      {ctxMenu && (
        <ZapContextMenu
          x={ctxMenu.x}
          y={ctxMenu.y}
          entry={ctxMenu.entry}
          onClose={() => setCtxMenu(null)}
          onSendToRepeater={(e) => handleSendToRepeater(e.id)}
          onSendToIntruder={onSendToIntruder ? async (e) => {
            try { const msg = await zapGetMessage(e.id, e.url); onSendToIntruder(buildRawRequest(msg)); } catch { /* ignore */ }
          } : undefined}
          onActiveScan={onActiveScan}
        />
      )}
    </div>
  );
}

function HistoryBodyPanel({
  filtered, entries, selectedId, detail, loading,
  onSelect, onSendToRepeater, onSendDetailToRepeater, onCtxMenu, onClose,
}: {
  filtered: HttpHistoryEntry[]; entries: HttpHistoryEntry[];
  selectedId: number | null; detail: HttpMessageDetail | null; loading: boolean;
  onSelect: (id: number) => void; onSendToRepeater: (id: number) => void;
  onSendDetailToRepeater: (d: HttpMessageDetail) => void;
  onCtxMenu: (x: number, y: number, entry: HttpHistoryEntry) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const detailRef = useRef<HTMLDivElement>(null);
  const [detailWidth, setDetailWidth] = useState<number | null>(null);
  const MIN_DETAIL = 280;

  useEffect(() => {
    if (selectedId !== null && detailWidth === null && containerRef.current) {
      setDetailWidth(Math.floor(containerRef.current.offsetWidth / 2));
    }
  }, [selectedId, detailWidth]);

  const handleResize = useCallback((delta: number) => {
    const maxW = containerRef.current ? containerRef.current.offsetWidth - 280 : 900;
    setDetailWidth((w) => Math.min(maxW, Math.max(MIN_DETAIL, (w ?? 480) - delta)));
  }, []);

  return (
    <div ref={containerRef} className="flex-1 flex min-h-0">
      <div className="flex-1 min-w-0 overflow-y-auto">
        <table className="w-full text-[11px]">
          <thead className="sticky top-0 bg-card z-10">
            <tr className="text-muted-foreground/40 text-left">
              <th className="px-3 py-1.5 font-medium w-[50px]">#</th>
              <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.method")}</th>
              <th className="px-3 py-1.5 font-medium">{t("security.url")}</th>
              <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.status")}</th>
              <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.size")}</th>
              <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.time")}</th>
              <th className="px-3 py-1.5 font-medium w-[36px]" />
            </tr>
          </thead>
          <tbody>
            {filtered.map((entry) => (
              <tr
                key={entry.id}
                onClick={() => onSelect(entry.id)}
                onContextMenu={(e) => { e.preventDefault(); onCtxMenu(e.clientX, e.clientY, entry); }}
                className={cn(
                  "cursor-pointer transition-colors border-b border-border/5 group",
                  selectedId === entry.id ? "bg-accent/10" : "hover:bg-[var(--bg-hover)]/40"
                )}
              >
                <td className="px-3 py-1.5 text-muted-foreground/50 font-mono">{entry.id}</td>
                <td className={cn("px-3 py-1.5 font-mono font-medium", methodColor(entry.method))}>{entry.method}</td>
                <td className="px-3 py-1.5 text-foreground/80 truncate max-w-[400px] font-mono">
                  {entry.host}<span className="text-muted-foreground/40">{entry.path}</span>
                </td>
                <td className={cn("px-3 py-1.5 font-mono", statusColor(entry.status_code))}>{entry.status_code || "-"}</td>
                <td className="px-3 py-1.5 text-muted-foreground/40 font-mono">{formatSize(entry.content_length)}</td>
                <td className="px-3 py-1.5 text-muted-foreground/40 font-mono">{entry.time_ms ? `${entry.time_ms}ms` : "-"}</td>
                <td className="px-1.5 py-1.5">
                  <button type="button" title={t("security.sendToRepeater")}
                    onClick={(e) => { e.stopPropagation(); onSendToRepeater(entry.id); }}
                    className="p-1 rounded text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-accent hover:bg-accent/10 transition-colors">
                    <Send className="w-3 h-3" />
                  </button>
                </td>
              </tr>
            ))}
            {filtered.length === 0 && (
              <tr>
                <td colSpan={7} className="px-3 py-8 text-center text-muted-foreground/50">
                  {entries.length === 0 ? t("security.noHistory") : t("common.noResults")}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {selectedId !== null && (
        <>
          <ResizeHandle onResize={handleResize} />
          <div ref={detailRef} className="flex-shrink-0 flex flex-col overflow-hidden" style={{ width: detailWidth ?? 480 }}>
            <div className="flex items-center justify-between px-3 py-2 border-b border-border/10">
              <span className="text-[11px] font-medium text-muted-foreground/60">
                #{selectedId} {t("security.detail")}
              </span>
              <div className="flex items-center gap-1">
                {detail && (
                  <button type="button" onClick={() => onSendDetailToRepeater(detail)}
                    className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium text-muted-foreground/60 hover:text-accent hover:bg-accent/10 transition-colors">
                    <Send className="w-3 h-3" />{t("security.sendToRepeater")}
                  </button>
                )}
                <button type="button" onClick={onClose}
                  className="p-1 rounded text-muted-foreground/30 hover:text-foreground transition-colors">
                  <X className="w-3 h-3" />
                </button>
              </div>
            </div>
            {loading ? (
              <div className="flex-1 flex items-center justify-center">
                <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/50" />
              </div>
            ) : detail ? (
              <DetailTabs detail={detail} />
            ) : (
              <div className="flex-1 flex items-center justify-center text-muted-foreground/40 text-[12px]">
                {t("security.noDetail")}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}

function prettyPrintBody(content: string, headers?: string): string {
  if (!content || !content.trim()) return content;
  const ct = (headers || "").toLowerCase();
  if (ct.includes("application/json") || ct.includes("+json")) {
    try { return JSON.stringify(JSON.parse(content), null, 2); } catch { /* not valid JSON */ }
  }
  if (/^\s*[{[]/.test(content)) {
    try { return JSON.stringify(JSON.parse(content), null, 2); } catch { /* not valid JSON */ }
  }
  if (ct.includes("text/html") || ct.includes("application/xhtml")) {
    try {
      let depth = 0;
      const lines: string[] = [];
      const raw = content.replace(/>\s*</g, ">\n<");
      for (const line of raw.split("\n")) {
        const trimmed = line.trim();
        if (!trimmed) continue;
        if (/^<\//.test(trimmed)) depth = Math.max(0, depth - 1);
        lines.push("  ".repeat(depth) + trimmed);
        if (/^<[^/!][^>]*[^/]>$/.test(trimmed) && !/<(br|hr|img|input|meta|link|col|area|base|embed|source|track|wbr)[\s/>]/i.test(trimmed)) {
          depth++;
        }
      }
      return lines.join("\n");
    } catch { /* fallback to raw */ }
  }
  if (ct.includes("javascript") || ct.includes("text/css")) {
    try {
      if (content.length > 500_000) return content;
      if (ct.includes("css")) {
        return cssBeautify(content, { indent_size: 2 });
      }
      return jsBeautify(content, { indent_size: 2, space_in_empty_paren: false });
    } catch { /* fallback to raw */ }
  }
  return content;
}


function parseHeaders(raw: string): { firstLine: string; headers: [string, string][] } {
  const lines = raw.split("\n").map((l) => l.replace(/\r$/, ""));
  const firstLine = lines[0] || "";
  const headers: [string, string][] = [];
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) continue;
    const idx = line.indexOf(":");
    if (idx > 0) {
      headers.push([line.substring(0, idx).trim(), line.substring(idx + 1).trim()]);
    }
  }
  return { firstLine, headers };
}

function syntaxHighlightJson(json: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  const re = /("(?:[^"\\]|\\.)*")\s*:|("(?:[^"\\]|\\.)*")|(-?\d+\.?\d*(?:[eE][+-]?\d+)?)|(\btrue\b|\bfalse\b)|(\bnull\b)/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;
  let key = 0;
  while ((match = re.exec(json)) !== null) {
    if (match.index > lastIndex) {
      nodes.push(json.substring(lastIndex, match.index));
    }
    if (match[1]) {
      nodes.push(<span key={key++} className="text-purple-400">{match[1]}</span>);
      nodes.push(<span key={key++} className="text-foreground/40">:</span>);
    } else if (match[2]) {
      nodes.push(<span key={key++} className="text-green-400">{match[2]}</span>);
    } else if (match[3]) {
      nodes.push(<span key={key++} className="text-blue-400">{match[3]}</span>);
    } else if (match[4]) {
      nodes.push(<span key={key++} className="text-orange-400">{match[4]}</span>);
    } else if (match[5]) {
      nodes.push(<span key={key++} className="text-red-400/70">{match[5]}</span>);
    }
    lastIndex = match.index + match[0].length;
  }
  if (lastIndex < json.length) {
    nodes.push(json.substring(lastIndex));
  }
  return nodes;
}

function HeaderTable({ raw }: { raw: string }) {
  const { firstLine, headers } = useMemo(() => parseHeaders(raw), [raw]);
  if (!raw.trim()) return <div className="px-3 py-2 text-[10px] text-muted-foreground/50">(empty)</div>;

  return (
    <div className="text-[10px]">
      <div className="px-3 py-1.5 font-mono text-foreground/80 bg-[var(--bg-hover)]/20 border-b border-border/5">
        {firstLine}
      </div>
      <table className="w-full">
        <tbody>
          {headers.map(([k, v], i) => (
            <tr key={i} className="border-b border-border/[0.03] hover:bg-[var(--bg-hover)]/20 transition-colors">
              <td className="px-3 py-1 font-mono font-medium text-accent/70 whitespace-nowrap align-top w-[1%]">{k}</td>
              <td className="px-2 py-1 font-mono text-foreground/60 break-all">{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function BodyView({ content, headers }: { content: string; headers?: string }) {
  const formatted = useMemo(() => prettyPrintBody(content, headers), [content, headers]);
  const isJson = useMemo(() => {
    try { JSON.parse(content); return true; } catch { return false; }
  }, [content]);

  if (!content || !content.trim()) return <div className="px-3 py-4 text-[10px] text-muted-foreground/50 text-center">(empty)</div>;

  return (
    <div className="relative group">
      <button
        type="button"
        onClick={() => copyToClipboard(formatted)}
        className="absolute top-2 right-2 p-1 rounded text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-accent hover:!bg-accent/10 transition-all z-10"
        title="Copy"
      >
        <Copy className="w-3 h-3" />
      </button>
      <pre className="px-3 py-2 text-[10px] font-mono text-foreground/70 whitespace-pre-wrap break-all overflow-y-auto">
        {isJson ? syntaxHighlightJson(formatted) : formatted}
      </pre>
    </div>
  );
}

export function DetailTabs({ detail }: { detail: HttpMessageDetail }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<"request" | "response">("response");

  return (
    <div className="flex-1 flex flex-col min-h-0">
      <div className="flex items-center gap-0 border-b border-border/10 px-2 flex-shrink-0">
        {(["request", "response"] as const).map((id) => (
          <button
            key={id}
            type="button"
            onClick={() => setTab(id)}
            className={cn(
              "px-3 py-1.5 text-[10px] font-medium transition-colors relative",
              tab === id
                ? "text-accent"
                : "text-muted-foreground/40 hover:text-foreground"
            )}
          >
            {id === "request" ? t("security.request", "Request") : t("security.response", "Response")}
            {tab === id && <div className="absolute bottom-0 left-1 right-1 h-[1.5px] bg-accent rounded-full" />}
          </button>
        ))}
        <div className="flex-1" />
        <span className="text-[9px] text-muted-foreground/25 font-mono mr-1">
          {tab === "response" && detail.response_body ? `${(detail.response_body.length / 1024).toFixed(1)} KB` : ""}
        </span>
      </div>
      <div className="flex-1 overflow-y-auto">
        {tab === "request" ? (
          <>
            <HeaderTable raw={detail.request_headers} />
            {detail.request_body && (
              <>
                <div className="px-3 py-1 text-[9px] font-medium text-muted-foreground/50 bg-[var(--bg-hover)]/10 border-y border-border/5">
                  {t("security.requestBody", "Request Body")}
                </div>
                <BodyView content={detail.request_body} headers={detail.request_headers} />
              </>
            )}
          </>
        ) : (
          <>
            <HeaderTable raw={detail.response_headers} />
            <div className="px-3 py-1 text-[9px] font-medium text-muted-foreground/50 bg-[var(--bg-hover)]/10 border-y border-border/5">
              {t("security.responseBody", "Response Body")}
            </div>
            <BodyView content={detail.response_body} headers={detail.response_headers} />
          </>
        )}
      </div>
    </div>
  );
}


