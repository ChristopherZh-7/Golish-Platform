import { useCallback, useEffect, useState } from "react";
import {
  Check, Copy, Download, ExternalLink, Globe, Loader2, ShieldCheck,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { zapDownloadRootCert, zapInstallRootCert, zapStatus } from "@/lib/pentest/zap-api";

interface BrowserViewProps {
  initialUrl?: string;
  sessionId?: string;
}

const DEFAULT_PORT = 8090;

export function BrowserView({ initialUrl = "" }: BrowserViewProps) {
  const { t } = useTranslation();
  const [url, setUrl] = useState(initialUrl);
  const [copied, setCopied] = useState(false);
  const [certLoading, setCertLoading] = useState(false);
  const [certResult, setCertResult] = useState<{ ok: boolean; msg: string } | null>(null);
  const [zapPort, setZapPort] = useState(DEFAULT_PORT);

  useEffect(() => {
    zapStatus().then((s) => setZapPort(s.port)).catch(() => {});
  }, []);

  const proxyAddr = `127.0.0.1:${zapPort}`;

  const openSystemBrowser = useCallback(async () => {
    const target = url.trim() || "https://example.com";
    const { open } = await import("@tauri-apps/plugin-shell");
    await open(target);
  }, [url]);

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
    } catch (e) {
      setCertResult({ ok: false, msg: String(e) });
    } finally {
      setCertLoading(false);
    }
  }, []);

  const handleInstallCert = useCallback(async () => {
    setCertLoading(true);
    setCertResult(null);
    try {
      const path = await zapInstallRootCert();
      setCertResult({ ok: true, msg: t("browser.certInstalled", `Certificate installed: ${path}`) });
    } catch (e) {
      setCertResult({ ok: false, msg: String(e) });
    } finally {
      setCertLoading(false);
    }
  }, [t]);

  return (
    <div className="h-full w-full flex flex-col bg-[var(--bg-primary)]">
      <div className="flex-1 flex flex-col items-center justify-center gap-6 px-8">
        <Globe className="w-14 h-14 text-muted-foreground/15" />
        <div className="text-center">
          <h2 className="text-[16px] font-semibold text-foreground/80 mb-1">{t("browser.title", "Browser")}</h2>
          <p className="text-[12px] text-muted-foreground/40 max-w-md">
            {t("browser.externalHint", "Configure proxy in your browser to capture traffic through ZAP, then open it from here.")}
          </p>
        </div>

        <div className="w-full max-w-md space-y-4">
          {/* Proxy info */}
          <div className="rounded-xl border border-border/15 bg-[var(--bg-hover)]/15 p-4">
            <span className="text-[11px] font-medium text-muted-foreground/50 block mb-2">
              {t("browser.proxyConfig", "Proxy Configuration")}
            </span>
            <div className="flex items-center gap-2 bg-background/50 rounded-lg px-3 py-2 border border-border/10">
              <span className="text-[11px] text-muted-foreground/40 flex-shrink-0">HTTP Proxy:</span>
              <code className="text-[12px] font-mono text-accent/80 flex-1">{proxyAddr}</code>
              <button
                type="button"
                onClick={copyProxy}
                className="p-1 rounded text-muted-foreground/40 hover:text-foreground transition-colors"
              >
                {copied ? <Check className="w-3 h-3 text-green-400" /> : <Copy className="w-3 h-3" />}
              </button>
            </div>
            <p className="text-[10px] text-muted-foreground/30 mt-2 leading-relaxed">
              {t("browser.proxyManualHint", "Configure this proxy in your browser's network settings (e.g. FoxyProxy extension or browser proxy settings) to route traffic through ZAP.")}
            </p>
          </div>

          {/* SSL Certificate */}
          <div className="rounded-xl border border-border/15 bg-[var(--bg-hover)]/15 p-4">
            <span className="text-[11px] font-medium text-muted-foreground/50 block mb-2">
              {t("browser.sslCert", "HTTPS Certificate")}
            </span>
            <p className="text-[10px] text-muted-foreground/30 mb-3 leading-relaxed">
              {t("browser.sslCertHint", "To intercept HTTPS traffic, install ZAP's root CA certificate as trusted. Without it, browsers will show certificate warnings for HTTPS sites.")}
            </p>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={handleDownloadCert}
                disabled={certLoading}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-[var(--bg-hover)]/50 text-foreground/70 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-50"
              >
                {certLoading ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
                {t("browser.downloadCert", "Download Cert")}
              </button>
              <button
                type="button"
                onClick={handleInstallCert}
                disabled={certLoading}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors disabled:opacity-50"
              >
                {certLoading ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <ShieldCheck className="w-3.5 h-3.5" />}
                {t("browser.installCert", "Install to Keychain")}
              </button>
            </div>
            {certResult && (
              <div className={cn(
                "mt-2 px-3 py-1.5 rounded-lg text-[10px] font-mono break-all",
                certResult.ok
                  ? "bg-green-500/10 text-green-400"
                  : "bg-red-500/10 text-red-400"
              )}>
                {certResult.msg}
              </div>
            )}
          </div>

          {/* URL + Launch */}
          <div className="rounded-xl border border-border/15 bg-[var(--bg-hover)]/15 p-4">
            <label className="text-[11px] font-medium text-muted-foreground/50 block mb-2">
              {t("browser.targetUrl", "Target URL")}
            </label>
            <div className="flex items-center gap-2">
              <div className="relative flex-1">
                <Globe className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
                <input
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://example.com"
                  onKeyDown={(e) => e.key === "Enter" && openSystemBrowser()}
                  className="w-full h-8 pl-8 pr-3 text-[11px] font-mono bg-background/50 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
                />
              </div>
              <button
                type="button"
                onClick={openSystemBrowser}
                className="flex items-center gap-1.5 px-4 py-2 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors flex-shrink-0"
              >
                <ExternalLink className="w-3.5 h-3.5" />
                {t("browser.openBrowser", "Open Browser")}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
