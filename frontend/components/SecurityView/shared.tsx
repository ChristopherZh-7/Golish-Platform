import { useCallback, useEffect, useRef, useState } from "react";
import {
  Check, ChevronDown, ChevronRight, Copy, Download, Loader2, Play,
  RefreshCw, Shield, ShieldCheck, ShieldX,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { zapDownloadRootCert, zapInstallRootCert } from "@/lib/pentest/zap-api";
import type { ZapStatusInfo } from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";

export function StyledSelect({ value, onChange, options, className }: {
  value: string; onChange: (v: string) => void;
  options: { value: string; label: string }[];
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const selected = options.find((o) => o.value === value) ?? options[0];

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false); };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  return (
    <div ref={ref} className={cn("relative", className)}>
      <button type="button" onClick={() => setOpen(!open)}
        className="flex items-center gap-1 w-full h-full px-2.5 text-[10px] rounded-lg bg-white/[0.03] border border-white/[0.06] hover:border-white/[0.12] text-foreground/70 transition-colors cursor-pointer"
      >
        <span className="flex-1 text-left truncate">{selected?.label}</span>
        <ChevronDown className={cn("w-2.5 h-2.5 text-muted-foreground/30 transition-transform flex-shrink-0", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute z-50 mt-0.5 w-full min-w-[100px] rounded-lg border border-white/[0.08] bg-[#1a1a1f] shadow-xl py-0.5 max-h-[200px] overflow-y-auto">
          {options.map((o) => (
            <button key={o.value} type="button" onClick={() => { onChange(o.value); setOpen(false); }}
              className={cn("w-full text-left px-2.5 py-1.5 text-[10px] transition-colors", o.value === value ? "bg-accent/15 text-accent" : "text-foreground/60 hover:bg-white/[0.05] hover:text-foreground")}
            >
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function StatusBadge({ status }: { status: ZapStatusInfo }) {
  const colors: Record<string, string> = {
    running: "bg-green-500/15 text-green-400",
    starting: "bg-yellow-500/15 text-yellow-400",
    stopped: "bg-zinc-500/15 text-zinc-400",
    error: "bg-red-500/15 text-red-400",
  };

  return (
    <span
      className={cn(
        "text-[9px] px-2 py-0.5 rounded-full font-medium flex items-center gap-1",
        colors[status.status] || colors.stopped
      )}
    >
      {status.status === "running" && (
        <span className="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse" />
      )}
      {status.status}
      {status.version && ` v${status.version}`}
      {status.status === "running" && ` :${status.port}`}
    </span>
  );
}

export function ZapNotInstalled({ onRetry }: { onRetry: () => void }) {
  const { t } = useTranslation();
  const [installing, setInstalling] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);

  const handleBrewInstall = useCallback(async () => {
    setInstalling(true);
    setInstallError(null);
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const { getSettings } = await import("@/lib/settings");
      const settings = await getSettings().catch(() => null);
      const proxyUrl = settings?.network?.proxy_url || null;
      await invoke("pentest_install_runtime", { runtimeType: "brew-cask:zap", proxyUrl });
      onRetry();
    } catch (e) {
      setInstallError(String(e));
    } finally {
      setInstalling(false);
    }
  }, [onRetry]);

  return (
    <div className="h-full flex flex-col items-center justify-center gap-5">
      <ShieldX className="w-16 h-16 text-destructive/40" />
      <div className="text-center">
        <p className="text-[15px] font-semibold text-foreground/80">{t("security.zapNotInstalled")}</p>
        <p className="text-[12px] text-muted-foreground/50 max-w-md mt-1.5 leading-relaxed">
          {t("security.zapNotInstalledHint")}
        </p>
      </div>

      {installError && (
        <p className="text-[11px] text-destructive max-w-sm text-center">{installError}</p>
      )}

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={handleBrewInstall}
          disabled={installing}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg text-[13px] font-semibold bg-accent text-accent-foreground hover:bg-accent/90 transition-colors disabled:opacity-50 shadow-sm"
        >
          {installing ? (
            <Loader2 className="w-4 h-4 animate-spin" />
          ) : (
            <Download className="w-4 h-4" />
          )}
          {t("security.installViaBrew")}
        </button>
        <button
          type="button"
          onClick={onRetry}
          className="flex items-center gap-2 px-4 py-2 rounded-lg text-[12px] font-medium text-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          {t("security.recheckInstall")}
        </button>
      </div>

      <div className="max-w-md mt-2 text-center">
        <p className="text-[11px] text-muted-foreground/40">
          {t("security.manualInstallHint")}
        </p>
        <code className="text-[12px] text-foreground/60 bg-muted/30 px-3 py-1 rounded mt-1.5 inline-block font-mono">
          brew install --cask zap
        </code>
      </div>
    </div>
  );
}

export function ZapNotRunning({ onStart, loading, error }: { onStart: () => void; loading: boolean; error: string | null }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [certLoading, setCertLoading] = useState(false);
  const [certResult, setCertResult] = useState<{ ok: boolean; msg: string } | null>(null);
  const proxyAddr = "127.0.0.1:8090";

  const copyProxy = useCallback(async () => {
    await navigator.clipboard.writeText(proxyAddr);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }, []);

  const handleDownloadCert = useCallback(async () => {
    setCertLoading(true);
    setCertResult(null);
    try {
      const path = await zapDownloadRootCert();
      setCertResult({ ok: true, msg: path });
    } catch (e) { setCertResult({ ok: false, msg: String(e) }); }
    finally { setCertLoading(false); }
  }, []);

  const handleInstallCert = useCallback(async () => {
    setCertLoading(true);
    setCertResult(null);
    try {
      const path = await zapInstallRootCert();
      setCertResult({ ok: true, msg: t("browser.certInstalled", `Certificate installed: ${path}`) });
    } catch (e) { setCertResult({ ok: false, msg: String(e) }); }
    finally { setCertLoading(false); }
  }, [t]);

  return (
    <div className="h-full overflow-y-auto">
      <div className="flex flex-col items-center gap-6 px-8 py-10 max-w-lg mx-auto">
        <Shield className="w-14 h-14 text-accent/30" />
        <div className="text-center">
          <p className="text-[15px] font-semibold text-foreground/80">{t("security.zapNotRunning")}</p>
          <p className="text-[12px] text-muted-foreground/50 mt-1.5 leading-relaxed">
            {t("security.zapNotRunningHint")}
          </p>
        </div>
        {error && (
          <p className="text-[11px] text-destructive max-w-sm text-center">{error}</p>
        )}
        <button
          type="button"
          onClick={onStart}
          disabled={loading}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg text-[13px] font-semibold bg-accent text-accent-foreground hover:bg-accent/90 transition-colors disabled:opacity-50 shadow-sm"
        >
          {loading ? <Loader2 className="w-4 h-4 animate-spin" /> : <Play className="w-4 h-4" />}
          {t("security.startZap")}
        </button>

        <div className="w-full border-t border-border/15 pt-5 mt-2 space-y-4">
          <h3 className="text-[12px] font-semibold text-foreground/70 text-center">{t("browser.proxyConfig", "Proxy & Certificate Setup")}</h3>

          <div className="rounded-lg border border-border/15 bg-[var(--bg-hover)]/15 p-3.5">
            <span className="text-[10px] font-medium text-foreground/50 block mb-2">
              {t("browser.proxyConfig", "HTTP Proxy")}
            </span>
            <div className="flex items-center gap-2 bg-background/50 rounded-md px-3 py-2 border border-border/10">
              <code className="text-[12px] font-mono text-accent/80 flex-1">{proxyAddr}</code>
              <button onClick={copyProxy} className="p-1 rounded text-muted-foreground/40 hover:text-foreground transition-colors">
                {copied ? <Check className="w-3 h-3 text-green-400" /> : <Copy className="w-3 h-3" />}
              </button>
            </div>
            <p className="text-[10px] text-muted-foreground/40 mt-2 leading-relaxed">
              {t("browser.proxyManualHint", "Configure this proxy in your browser (e.g. FoxyProxy) to route traffic through ZAP.")}
            </p>
          </div>

          <div className="rounded-lg border border-border/15 bg-[var(--bg-hover)]/15 p-3.5">
            <span className="text-[10px] font-medium text-foreground/50 block mb-2">
              {t("browser.sslCert", "HTTPS Certificate")}
            </span>
            <p className="text-[10px] text-muted-foreground/40 mb-3 leading-relaxed">
              {t("browser.sslCertHint", "Install ZAP's root CA certificate to intercept HTTPS traffic without warnings.")}
            </p>
            <div className="flex items-center gap-2">
              <button onClick={handleDownloadCert} disabled={certLoading}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium bg-[var(--bg-hover)]/50 text-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-50">
                {certLoading ? <Loader2 className="w-3 h-3 animate-spin" /> : <Download className="w-3 h-3" />}
                {t("browser.downloadCert", "Download")}
              </button>
              <button onClick={handleInstallCert} disabled={certLoading}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors disabled:opacity-50">
                {certLoading ? <Loader2 className="w-3 h-3 animate-spin" /> : <ShieldCheck className="w-3 h-3" />}
                {t("browser.installCert", "Install to Keychain")}
              </button>
            </div>
            {certResult && (
              <div className={cn("mt-2 px-3 py-1.5 rounded-md text-[10px] font-mono break-all",
                certResult.ok ? "bg-green-500/10 text-green-400" : "bg-red-500/10 text-red-400"
              )}>{certResult.msg}</div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export function ResizeHandle({ onResize, direction = "horizontal" }: {
  onResize: (delta: number) => void;
  direction?: "horizontal" | "vertical";
}) {
  const isHorizontal = direction === "horizontal";
  const handleRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = handleRef.current;
    if (!el) return;
    let startPos = 0;

    const onPointerMove = (e: PointerEvent) => {
      const delta = isHorizontal ? e.clientX - startPos : e.clientY - startPos;
      startPos = isHorizontal ? e.clientX : e.clientY;
      onResize(delta);
    };
    const onPointerUp = () => {
      document.removeEventListener("pointermove", onPointerMove);
      document.removeEventListener("pointerup", onPointerUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    const onPointerDown = (e: PointerEvent) => {
      e.preventDefault();
      startPos = isHorizontal ? e.clientX : e.clientY;
      document.body.style.cursor = isHorizontal ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";
      document.addEventListener("pointermove", onPointerMove);
      document.addEventListener("pointerup", onPointerUp);
    };
    el.addEventListener("pointerdown", onPointerDown);
    return () => {
      el.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("pointermove", onPointerMove);
      document.removeEventListener("pointerup", onPointerUp);
    };
  }, [onResize, isHorizontal]);

  return (
    <div
      ref={handleRef}
      className={cn(
        "flex-shrink-0 relative group",
        isHorizontal
          ? "w-[5px] cursor-col-resize hover:bg-accent/20 active:bg-accent/30"
          : "h-[5px] cursor-row-resize hover:bg-accent/20 active:bg-accent/30",
        "transition-colors duration-100"
      )}
    >
      <div className={cn(
        "absolute bg-accent/40 opacity-0 group-hover:opacity-100 group-active:opacity-100 transition-opacity duration-100",
        isHorizontal ? "top-0 bottom-0 left-[2px] w-[1px]" : "left-0 right-0 top-[2px] h-[1px]"
      )} />
    </div>
  );
}


export function methodColor(m: string): string {
  const c: Record<string, string> = {
    GET: "text-green-400", POST: "text-blue-400", PUT: "text-yellow-400",
    DELETE: "text-red-400", PATCH: "text-purple-400", OPTIONS: "text-zinc-400",
    HEAD: "text-cyan-400",
  };
  return c[m] || "text-muted-foreground";
}

export function statusColor(code: number): string {
  if (code >= 200 && code < 300) return "text-green-400";
  if (code >= 300 && code < 400) return "text-blue-400";
  if (code >= 400 && code < 500) return "text-yellow-400";
  if (code >= 500) return "text-red-400";
  return "text-muted-foreground";
}

export function formatSize(bytes: number): string {
  if (bytes === 0) return "-";
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}K`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}M`;
}

export function DetailSection({ title, content }: { title: string; content: string }) {
  const [expanded, setExpanded] = useState(true);
  const lines = content.split("\n").map((l) => l.replace(/\r$/, ""));
  const firstLine = lines[0] || "";
  const headers: [string, string][] = [];
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) continue;
    const idx = line.indexOf(":");
    if (idx > 0) headers.push([line.substring(0, idx).trim(), line.substring(idx + 1).trim()]);
  }

  return (
    <div className="border-b border-border/5">
      <button type="button" onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-1.5 px-3 py-1.5 text-[10px] font-medium text-muted-foreground/40 hover:text-foreground transition-colors">
        {expanded ? <ChevronDown className="w-2.5 h-2.5" /> : <ChevronRight className="w-2.5 h-2.5" />}
        {title}
      </button>
      {expanded && (
        <div className="text-[10px]">
          {firstLine && (
            <div className="px-3 py-1.5 font-mono text-foreground/80 bg-[var(--bg-hover)]/20 border-b border-border/5">
              {firstLine}
            </div>
          )}
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
      )}
    </div>
  );
}
