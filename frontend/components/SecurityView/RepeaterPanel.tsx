import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity, ArrowRight, Check, Loader2, Send,
  ShieldAlert, Square, X, Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  zapSendRequest, zapStartScan, zapScanProgress, zapStopScan,
  zapGetAlerts,
} from "@/lib/pentest/zap-api";
import type { ZapAlert, ManualRequestResult } from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";
import { ScanResultsView } from "./AlertsPanel";
import { DetailSection } from "./shared";

const DEFAULT_SCRIPT = `// Pre-send script: modify \`req\` before sending.
// Available: req.method, req.url, req.headers (object), req.body
// Builtins: hmacSHA256(key, data), md5(data), sha256(data),
//           base64(data), timestamp(), randomHex(len)
//
// Example - HMAC signature:
//   const ts = timestamp();
//   req.headers['X-Timestamp'] = ts;
//   req.headers['X-Signature'] = hmacSHA256('secret-key', req.body + ts);
`;

function parseRawRequest(raw: string): { firstLine: string; headers: Record<string, string>; body: string } {
  const idx = raw.indexOf("\n\n");
  const headerPart = idx >= 0 ? raw.slice(0, idx) : raw;
  const body = idx >= 0 ? raw.slice(idx + 2) : "";
  const lines = headerPart.split("\n");
  const firstLine = lines[0] || "";
  const headers: Record<string, string> = {};
  for (let i = 1; i < lines.length; i++) {
    const colon = lines[i].indexOf(":");
    if (colon > 0) headers[lines[i].slice(0, colon).trim()] = lines[i].slice(colon + 1).trim();
  }
  return { firstLine, headers, body };
}

function rebuildRawRequest(firstLine: string, headers: Record<string, string>, body: string): string {
  const hLines = Object.entries(headers).map(([k, v]) => `${k}: ${v}`);
  return [firstLine, ...hLines, "", body].join("\n");
}

async function applyScript(raw: string, script: string): Promise<string> {
  const { firstLine, headers, body } = parseRawRequest(raw);
  const parts = firstLine.split(" ");
  const req = { method: parts[0] || "GET", url: parts[1] || "/", headers: { ...headers }, body };

  const { subtle } = globalThis.crypto;
  const enc = new TextEncoder();

  const builtins = {
    timestamp: () => String(Math.floor(Date.now() / 1000)),
    randomHex: (len: number) => Array.from(crypto.getRandomValues(new Uint8Array(len)), (b) => b.toString(16).padStart(2, "0")).join(""),
    base64: (data: string) => btoa(data),
    md5: async (data: string) => {
      const buf = await subtle.digest("SHA-256", enc.encode(data));
      return Array.from(new Uint8Array(buf), (b) => b.toString(16).padStart(2, "0")).join("");
    },
    sha256: async (data: string) => {
      const buf = await subtle.digest("SHA-256", enc.encode(data));
      return Array.from(new Uint8Array(buf), (b) => b.toString(16).padStart(2, "0")).join("");
    },
    hmacSHA256: async (key: string, data: string) => {
      const cryptoKey = await subtle.importKey("raw", enc.encode(key), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
      const sig = await subtle.sign("HMAC", cryptoKey, enc.encode(data));
      return Array.from(new Uint8Array(sig), (b) => b.toString(16).padStart(2, "0")).join("");
    },
  };

  const fn = new Function("req", ...Object.keys(builtins), `return (async () => { ${script} })();`);
  await fn(req, ...Object.values(builtins));
  return rebuildRawRequest(`${req.method} ${req.url} HTTP/1.1`, req.headers, req.body);
}

function prettyFormatBody(body: string, contentType: string): string {
  if (!body.trim()) return body;
  const ct = contentType.toLowerCase();
  if (ct.includes("json") || ct.includes("javascript")) {
    try { return JSON.stringify(JSON.parse(body), null, 2); } catch { return body; }
  }
  if (ct.includes("html") || ct.includes("xml") || body.trimStart().startsWith("<")) {
    try {
      let indent = 0;
      const lines: string[] = [];
      const tokens = body.replace(/>\s*</g, ">\n<").split("\n");
      for (const raw of tokens) {
        const line = raw.trim();
        if (!line) continue;
        const isClosing = /^<\//.test(line);
        const isSelfClosing = /\/>$/.test(line) || /^<(meta|link|br|hr|img|input|!doctype)\b/i.test(line);
        if (isClosing) indent = Math.max(0, indent - 1);
        lines.push("  ".repeat(indent) + line);
        if (!isClosing && !isSelfClosing && /^<[a-zA-Z]/.test(line) && !line.includes("</")) indent++;
      }
      return lines.join("\n");
    } catch { return body; }
  }
  return body;
}

function extractContentType(headers: string): string {
  const match = headers.match(/content-type:\s*([^\r\n]+)/i);
  return match?.[1] ?? "";
}

interface RepeaterTab {
  id: string;
  name: string;
  request: string;
  response: ManualRequestResult | null;
  script: string;
  scriptEnabled: boolean;
  sending: boolean;
  paused: boolean;
  scanId: string | null;
  scanProgress: number;
  scanState: "idle" | "running" | "completed" | "stopped";
  scanAlerts: ZapAlert[];
}

let repeaterTabCounter = 0;

function createRepeaterTab(request?: string): RepeaterTab {
  repeaterTabCounter++;
  const raw = request || "GET / HTTP/1.1\nHost: example.com\nUser-Agent: golish-platform/1.0\n\n";
  return {
    id: `rep-${Date.now()}-${repeaterTabCounter}`,
    name: repeaterTabLabel(raw),
    request: raw,
    response: null,
    script: DEFAULT_SCRIPT,
    scriptEnabled: false,
    sending: false,
    paused: false,
    scanId: null,
    scanProgress: 0,
    scanState: "idle",
    scanAlerts: [],
  };
}

function extractMethodFromRequest(raw: string): string {
  return raw.split("\n")[0]?.split(" ")[0]?.toUpperCase() || "GET";
}

function extractUrlFromRequest(raw: string): string {
  const lines = raw.split("\n");
  const firstLine = lines[0] || "";
  const parts = firstLine.split(" ");
  const pathOrUrl = parts[1] || "/";
  if (pathOrUrl.startsWith("http")) return pathOrUrl;
  const hostMatch = raw.match(/^host:\s*(.+)/im);
  const host = hostMatch?.[1]?.trim() || "localhost";
  const scheme = raw.toLowerCase().includes("https") ? "https" : "http";
  return `${scheme}://${host}${pathOrUrl}`;
}

function extractBodyFromRequest(raw: string): string {
  const idx = raw.indexOf("\n\n");
  return idx >= 0 ? raw.substring(idx + 2) : "";
}

type MethodSafety = "safe" | "warn" | "blocked";
function getMethodSafety(method: string): MethodSafety {
  const m = method.toUpperCase();
  if (m === "DELETE") return "blocked";
  if (m === "POST" || m === "PUT" || m === "PATCH") return "warn";
  return "safe";
}

function repeaterTabLabel(raw: string): string {
  const firstLine = raw.split("\n")[0] || "";
  const parts = firstLine.split(" ");
  const method = parts[0] || "GET";
  let host = "";
  const hostMatch = raw.match(/^host:\s*(.+)/im);
  if (hostMatch) host = hostMatch[1].trim();
  else if (parts[1]) { try { host = new URL(parts[1]).host; } catch { host = parts[1].split("/")[0]; } }
  return host ? `${method} ${host}` : method;
}

export function RepeaterPanel({ injectedRequest, onInjectedConsumed }: { injectedRequest: string | null; onInjectedConsumed: () => void }) {
  const { t } = useTranslation();
  const [tabs, setTabs] = useState<RepeaterTab[]>(() => [createRepeaterTab()]);
  const [activeTabId, setActiveTabId] = useState(() => tabs[0]?.id ?? "");
  const [scriptError, setScriptError] = useState<string | null>(null);
  const [showScript, setShowScript] = useState(false);
  const [prettyMode, setPrettyMode] = useState(true);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? tabs[0];
  const sending = activeTab?.sending ?? false;

  const updateTab = useCallback((id: string, patch: Partial<RepeaterTab>) => {
    setTabs((prev) => prev.map((t) => t.id === id ? { ...t, ...patch } : t));
  }, []);

  useEffect(() => {
    if (injectedRequest) {
      const raw = injectedRequest.replace(/\r\n/g, "\n");
      const newTab = createRepeaterTab(raw);
      setTabs((prev) => [...prev, newTab]);
      setActiveTabId(newTab.id);
      onInjectedConsumed();
    }
  }, [injectedRequest, onInjectedConsumed]);

  const handleSend = useCallback(async () => {
    if (!activeTab || activeTab.sending) return;
    updateTab(activeTab.id, { sending: true, paused: false });
    setScriptError(null);
    try {
      let finalRequest = activeTab.request;
      if (activeTab.scriptEnabled && activeTab.script.trim()) {
        try {
          finalRequest = await applyScript(activeTab.request, activeTab.script);
          updateTab(activeTab.id, { request: finalRequest, name: repeaterTabLabel(finalRequest) });
        } catch (e) {
          setScriptError(String(e));
          updateTab(activeTab.id, { sending: false });
          return;
        }
      }
      const result = await zapSendRequest(finalRequest);
      updateTab(activeTab.id, { response: result, sending: false });
    } catch {
      updateTab(activeTab.id, { response: null, sending: false });
    }
  }, [activeTab, updateTab]);

  const handleAddTab = useCallback(() => {
    const newTab = createRepeaterTab();
    setTabs((prev) => [...prev, newTab]);
    setActiveTabId(newTab.id);
  }, []);

  const handleCloseTab = useCallback((id: string) => {
    setTabs((prev) => {
      const next = prev.filter((t) => t.id !== id);
      if (next.length === 0) {
        const fresh = createRepeaterTab();
        setActiveTabId(fresh.id);
        return [fresh];
      }
      if (activeTabId === id) {
        const idx = prev.findIndex((t) => t.id === id);
        setActiveTabId(next[Math.min(idx, next.length - 1)].id);
      }
      return next;
    });
  }, [activeTabId]);

  const scanPollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const stopScanPolling = useCallback(() => {
    if (scanPollRef.current) {
      clearInterval(scanPollRef.current);
      scanPollRef.current = null;
    }
  }, []);

  useEffect(() => () => stopScanPolling(), [stopScanPolling]);

  const handleScanStart = useCallback(async () => {
    if (!activeTab) return;
    const method = extractMethodFromRequest(activeTab.request);
    const safety = getMethodSafety(method);

    if (safety === "blocked") {
      alert(t("security.scanBlockDelete"));
      return;
    }
    if (safety === "warn") {
      const msg = method === "POST" ? t("security.scanConfirmPost") : t("security.scanConfirmPut");
      if (!confirm(msg)) return;
    }

    const url = extractUrlFromRequest(activeTab.request);
    const body = extractBodyFromRequest(activeTab.request);

    updateTab(activeTab.id, { scanState: "running", scanProgress: 0, scanAlerts: [] });

    try {
      const scanId = await zapStartScan(
        url,
        method !== "GET" ? method : undefined,
        body.trim() ? body : undefined,
      );
      updateTab(activeTab.id, { scanId });

      stopScanPolling();
      const tabId = activeTab.id;
      scanPollRef.current = setInterval(async () => {
        try {
          const prog = await zapScanProgress(scanId);
          const alerts = await zapGetAlerts(url, 0, 500);
          updateTab(tabId, { scanProgress: prog.progress, scanAlerts: alerts });
          if (prog.state === "completed" || prog.progress >= 100) {
            stopScanPolling();
            updateTab(tabId, { scanState: "completed", scanProgress: 100 });
          }
        } catch {
          stopScanPolling();
          updateTab(tabId, { scanState: "stopped" });
        }
      }, 2000);
    } catch {
      updateTab(activeTab.id, { scanState: "idle" });
    }
  }, [activeTab, updateTab, stopScanPolling, t]);

  const handleScanStop = useCallback(async () => {
    if (!activeTab?.scanId) return;
    stopScanPolling();
    try {
      await zapStopScan(activeTab.scanId);
    } catch { /* ignore */ }
    updateTab(activeTab.id, { scanState: "stopped" });
  }, [activeTab, updateTab, stopScanPolling]);

  const [showScanResults, setShowScanResults] = useState(false);

  const formattedBody = useMemo(() => {
    if (!activeTab?.response?.response_body) return "(empty body)";
    if (!prettyMode) return activeTab.response.response_body;
    return prettyFormatBody(activeTab.response.response_body, extractContentType(activeTab.response.response_header)) || "(empty body)";
  }, [activeTab?.response, prettyMode]);

  const response = activeTab?.response ?? null;

  return (
    <div className="h-full flex flex-col">
      {/* Tab bar */}
      <div className="flex items-center border-b border-border/10 flex-shrink-0 overflow-x-auto">
        {tabs.map((tab) => (
          <div
            key={tab.id}
            className={cn(
              "group flex items-center gap-1.5 px-3 py-1.5 text-[10px] font-mono cursor-pointer border-r border-border/5 transition-colors min-w-0 max-w-[180px]",
              tab.id === activeTabId
                ? "bg-accent/10 text-accent"
                : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]/30"
            )}
            onClick={() => setActiveTabId(tab.id)}
          >
            <span className="truncate flex-1">{tab.name}</span>
            {tab.response && (
              <span className={cn(
                "text-[8px] px-1 rounded flex-shrink-0",
                tab.response.status_code >= 200 && tab.response.status_code < 300
                  ? "text-green-400" : tab.response.status_code >= 400 ? "text-red-400" : "text-yellow-400"
              )}>
                {tab.response.status_code}
              </span>
            )}
            <button
              type="button"
              onClick={(e) => { e.stopPropagation(); handleCloseTab(tab.id); }}
              className="p-0.5 rounded text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-foreground transition-colors flex-shrink-0"
            >
              <X className="w-2.5 h-2.5" />
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={handleAddTab}
          className="px-2.5 py-1.5 text-muted-foreground/30 hover:text-foreground hover:bg-[var(--bg-hover)]/30 transition-colors flex-shrink-0"
          title="New tab"
        >
          <span className="text-[12px] font-medium">+</span>
        </button>
      </div>

      {/* Content */}
      {activeTab && (
        <div className="h-full flex flex-1 min-h-0">
          <div className="flex-1 flex flex-col border-r border-border/10">
            <div className="flex items-center justify-between px-3 py-2 border-b border-border/10 flex-shrink-0">
              <div className="flex items-center gap-2">
                <span className="text-[11px] font-medium text-muted-foreground/40">
                  {t("security.request")}
                </span>
                <button
                  type="button"
                  onClick={() => setShowScript(!showScript)}
                  className={cn(
                    "flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md transition-colors",
                    showScript ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]"
                  )}
                >
                  <Activity className="w-3 h-3" />
                  Script
                </button>
                {activeTab.scriptEnabled && (
                  <span className="text-[9px] text-green-400 font-medium">ON</span>
                )}
              </div>
              <div className="flex items-center gap-1.5">
                {activeTab.scanState === "running" ? (
                  <button
                    type="button"
                    onClick={handleScanStop}
                    className="flex items-center gap-1.5 px-3 py-1 rounded-lg text-[11px] font-medium bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors"
                  >
                    <Square className="w-3 h-3" />
                    {t("security.stopScan")}
                  </button>
                ) : (
                  <button
                    type="button"
                    onClick={handleScanStart}
                    disabled={sending || !activeTab.request.trim()}
                    className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-[11px] font-medium bg-orange-500/10 text-orange-400 hover:bg-orange-500/20 transition-colors disabled:opacity-30"
                    title={t("security.scanParamsDesc")}
                  >
                    <Zap className="w-3 h-3" />
                    {t("security.scanParams")}
                  </button>
                )}
                <button
                  type="button"
                  onClick={handleSend}
                  disabled={sending || !activeTab.request.trim()}
                  className="flex items-center gap-1.5 px-3 py-1 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors disabled:opacity-30"
                >
                  {sending ? (
                    <Loader2 className="w-3 h-3 animate-spin" />
                  ) : (
                    <Send className="w-3 h-3" />
                  )}
                  {t("security.send")}
                </button>
              </div>
            </div>

            {/* Scan progress bar */}
            {activeTab.scanState !== "idle" && (
              <div className="px-3 py-1.5 border-b border-border/10 flex-shrink-0 bg-muted/5">
                <div className="flex items-center gap-2">
                  {activeTab.scanState === "running" ? (
                    <Loader2 className="w-3 h-3 animate-spin text-orange-400" />
                  ) : activeTab.scanState === "completed" ? (
                    <Check className="w-3 h-3 text-green-400" />
                  ) : (
                    <Square className="w-3 h-3 text-muted-foreground/40" />
                  )}
                  <span className="text-[10px] text-muted-foreground/60">
                    {activeTab.scanState === "running"
                      ? `${t("security.scanRunning")} ${activeTab.scanProgress}%`
                      : activeTab.scanState === "completed"
                        ? t("security.scanDone")
                        : t("security.scanStopped")}
                  </span>
                  <div className="flex-1 h-1 bg-muted/20 rounded-full overflow-hidden">
                    <div
                      className={cn(
                        "h-full rounded-full transition-all duration-300",
                        activeTab.scanState === "running" ? "bg-orange-400" :
                        activeTab.scanState === "completed" ? "bg-green-400" : "bg-muted-foreground/30"
                      )}
                      style={{ width: `${activeTab.scanProgress}%` }}
                    />
                  </div>
                  {activeTab.scanAlerts.length > 0 && (
                    <button
                      type="button"
                      onClick={() => setShowScanResults(!showScanResults)}
                      className="flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md font-medium bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors"
                    >
                      <ShieldAlert className="w-3 h-3" />
                      {activeTab.scanAlerts.length}
                    </button>
                  )}
                  {activeTab.scanState !== "running" && activeTab.scanAlerts.length === 0 && activeTab.scanState === "completed" && (
                    <span className="text-[10px] text-green-400/60">{t("security.scanNoResults")}</span>
                  )}
                </div>
              </div>
            )}

            {showScript && (
              <div className="border-b border-border/10 flex-shrink-0">
                <div className="flex items-center gap-2 px-3 py-1.5 bg-muted/5">
                  <span className="text-[10px] text-muted-foreground/40 font-medium">Pre-send Script</span>
                  <div className="flex-1" />
                  <button
                    type="button"
                    onClick={() => updateTab(activeTab.id, { scriptEnabled: !activeTab.scriptEnabled })}
                    className={cn(
                      "w-7 h-4 rounded-full transition-colors flex items-center px-0.5",
                      activeTab.scriptEnabled ? "bg-green-500/30" : "bg-muted/30"
                    )}
                  >
                    <div className={cn("w-3 h-3 rounded-full transition-all", activeTab.scriptEnabled ? "bg-green-400 ml-3" : "bg-muted-foreground/40 ml-0")} />
                  </button>
                </div>
                {scriptError && (
                  <div className="px-3 py-1 text-[10px] text-red-400 bg-red-500/5">{scriptError}</div>
                )}
                <textarea
                  value={activeTab.script}
                  onChange={(e) => updateTab(activeTab.id, { script: e.target.value })}
                  spellCheck={false}
                  className="w-full px-4 py-2 text-[10px] font-mono leading-[1.6] bg-transparent text-foreground/70 outline-none resize-none"
                  style={{ tabSize: 2 }}
                  rows={8}
                />
              </div>
            )}

            <textarea
              value={activeTab.request}
              onChange={(e) => updateTab(activeTab.id, { request: e.target.value, name: repeaterTabLabel(e.target.value) })}
              spellCheck={false}
              className="flex-1 w-full px-4 py-3 text-[11px] font-mono leading-[1.6] bg-transparent text-foreground outline-none resize-none"
              style={{ tabSize: 2 }}
            />
          </div>

          <div className="flex-1 flex flex-col">
            <div className="flex items-center gap-2 px-3 py-2 border-b border-border/10 flex-shrink-0">
              <button
                type="button"
                onClick={() => setShowScanResults(false)}
                className={cn(
                  "text-[11px] font-medium transition-colors",
                  !showScanResults ? "text-accent" : "text-muted-foreground/40 hover:text-foreground"
                )}
              >
                {t("security.response")}
              </button>
              {activeTab.scanAlerts.length > 0 && (
                <button
                  type="button"
                  onClick={() => setShowScanResults(true)}
                  className={cn(
                    "flex items-center gap-1 text-[11px] font-medium transition-colors",
                    showScanResults ? "text-red-400" : "text-muted-foreground/40 hover:text-foreground"
                  )}
                >
                  <ShieldAlert className="w-3 h-3" />
                  {t("security.scanResults")} ({activeTab.scanAlerts.length})
                </button>
              )}
              <div className="flex-1" />
              {!showScanResults && response && (
                <>
                  <span
                    className={cn(
                      "text-[10px] px-1.5 py-0.5 rounded-full font-mono",
                      response.status_code >= 200 && response.status_code < 300
                        ? "bg-green-500/10 text-green-400"
                        : response.status_code >= 400
                          ? "bg-red-500/10 text-red-400"
                          : "bg-yellow-500/10 text-yellow-400"
                    )}
                  >
                    {response.status_code}
                  </span>
                  <button
                    type="button"
                    onClick={() => setPrettyMode(!prettyMode)}
                    className={cn(
                      "px-2 py-0.5 text-[10px] rounded-md font-medium transition-colors",
                      prettyMode
                        ? "bg-accent/15 text-accent"
                        : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]"
                    )}
                  >
                    Pretty
                  </button>
                </>
              )}
            </div>
            {showScanResults ? (
              <ScanResultsView alerts={activeTab.scanAlerts} />
            ) : response ? (
              <div className="flex-1 overflow-y-auto">
                <DetailSection title={t("security.responseHeaders")} content={response.response_header} />
                <pre className="px-4 py-3 text-[11px] font-mono leading-[1.6] text-foreground/70 whitespace-pre-wrap break-all">
                  {formattedBody}
                </pre>
              </div>
            ) : (
              <div className="flex-1 flex items-center justify-center text-muted-foreground/20">
                <div className="flex flex-col items-center gap-2">
                  <ArrowRight className="w-8 h-8" />
                  <p className="text-[12px]">{t("security.sendToSee")}</p>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ── Scan Results View ──


