import {
  AlertCircle,
  Check,
  Copy,
  Download,
  Loader2,
  Play,
  Settings,
  ShieldCheck,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useZapProxyCert } from "@/hooks/useZapProxyCert";
import { cn } from "@/lib/utils";

interface SetupPopoverProps {
  isRunning: boolean;
  onStart: () => void;
  loading: boolean;
  error: string | null;
}

/**
 * Always-accessible setup popover anchored to a gear icon in the SecurityView
 * header. Houses the proxy address (always usable) and the HTTPS root-cert
 * Download / Install actions (require ZAP running).
 *
 * Decouples cert *visibility* from ZAP *running* state: the popover is
 * reachable in any state. When ZAP is stopped, cert buttons are disabled and
 * a banner instructs the user to start ZAP first.
 */
export function SetupPopover({ isRunning, onStart, loading, error }: SetupPopoverProps) {
  const { t } = useTranslation();
  const proxyAddr = "127.0.0.1:8090";
  const { copied, certLoading, certResult, copyProxy, handleDownloadCert, handleInstallCert } =
    useZapProxyCert(proxyAddr);

  const certDisabled = !isRunning || certLoading;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={t("security.setupTitle")}
          title={t("security.setupTitle")}
          className="flex items-center justify-center w-7 h-7 rounded-lg text-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          <Settings className="w-3.5 h-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        sideOffset={6}
        className="w-[380px] p-0 border-border/30 bg-popover/95 backdrop-blur"
      >
        <div className="flex items-center gap-2 px-4 py-3 border-b border-border/15">
          <Settings className="w-3.5 h-3.5 text-accent" />
          <h3 className="text-[12px] font-semibold text-foreground/80">
            {t("security.setupTitle")}
          </h3>
        </div>

        <div className="flex flex-col gap-4 px-4 py-4">
          {!isRunning && (
            <div className="rounded-md border border-amber-500/20 bg-amber-500/5 p-2.5 flex items-start gap-2">
              <AlertCircle className="w-3.5 h-3.5 text-amber-400 mt-[2px] flex-shrink-0" />
              <div className="flex-1">
                <p className="text-[11px] font-medium text-amber-300/90">
                  {t("security.setupZapStoppedTitle")}
                </p>
                <p className="text-[10px] text-amber-300/60 leading-relaxed mt-1">
                  {t("security.setupZapStoppedHint")}
                </p>
                <button
                  type="button"
                  onClick={onStart}
                  disabled={loading}
                  className="mt-2 flex items-center gap-1.5 px-2.5 py-1 rounded text-[10px] font-semibold bg-accent text-accent-foreground hover:bg-accent/90 transition-colors disabled:opacity-50"
                >
                  {loading ? (
                    <Loader2 className="w-3 h-3 animate-spin" />
                  ) : (
                    <Play className="w-3 h-3" />
                  )}
                  {t("security.startZap")}
                </button>
                {error && (
                  <p className="text-[9px] text-destructive/80 mt-1.5 break-all">{error}</p>
                )}
              </div>
            </div>
          )}

          <div>
            <span className="text-[10px] font-medium text-foreground/50 block mb-1.5">
              {t("browser.proxyConfig")}
            </span>
            <div className="flex items-center gap-1.5 bg-background/50 rounded-md px-2.5 py-1.5 border border-border/10">
              <code className="text-[11px] font-mono text-accent/80 flex-1">{proxyAddr}</code>
              <button
                type="button"
                onClick={copyProxy}
                className="p-1 rounded text-muted-foreground/50 hover:text-foreground transition-colors"
                aria-label="Copy proxy address"
              >
                {copied ? (
                  <Check className="w-3 h-3 text-green-400" />
                ) : (
                  <Copy className="w-3 h-3" />
                )}
              </button>
            </div>
            <p className="text-[9px] text-muted-foreground/40 mt-1 leading-relaxed">
              {t("browser.proxyManualHint")}
            </p>
          </div>

          <div>
            <span className="text-[10px] font-medium text-foreground/50 block mb-1.5">
              {t("browser.sslCert")}
            </span>
            <p className="text-[9px] text-muted-foreground/40 mb-2 leading-relaxed">
              {t(
                "browser.sslCertHint",
                "Install ZAP's root CA certificate to intercept HTTPS traffic without warnings."
              )}
            </p>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={handleDownloadCert}
                disabled={certDisabled}
                title={!isRunning ? t("security.setupNeedZapRunning") : undefined}
                className={cn(
                  "flex items-center gap-1.5 px-2.5 py-1 rounded text-[10px] font-medium transition-colors",
                  certDisabled
                    ? "bg-[var(--bg-hover)]/30 text-foreground/30 cursor-not-allowed"
                    : "bg-[var(--bg-hover)]/50 text-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)]"
                )}
              >
                {certLoading ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <Download className="w-3 h-3" />
                )}
                {t("browser.downloadCert")}
              </button>
              <button
                type="button"
                onClick={handleInstallCert}
                disabled={certDisabled}
                title={!isRunning ? t("security.setupNeedZapRunning") : undefined}
                className={cn(
                  "flex items-center gap-1.5 px-2.5 py-1 rounded text-[10px] font-medium transition-colors",
                  certDisabled
                    ? "bg-accent/5 text-accent/40 cursor-not-allowed"
                    : "bg-accent/15 text-accent hover:bg-accent/25"
                )}
              >
                {certLoading ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <ShieldCheck className="w-3 h-3" />
                )}
                {t("browser.installCert")}
              </button>
            </div>
            {certResult && (
              <div
                className={cn(
                  "mt-2 px-2.5 py-1.5 rounded-md text-[9px] font-mono break-all",
                  certResult.ok ? "bg-green-500/10 text-green-400" : "bg-red-500/10 text-red-400"
                )}
              >
                {certResult.msg}
              </div>
            )}
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
