import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft, ArrowRight, ExternalLink, Globe, Loader2,
  Lock, RotateCcw, Shield, ShieldOff, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { useStore } from "@/store";
import { useTranslation } from "react-i18next";

interface BrowserViewProps {
  initialUrl?: string;
  sessionId?: string;
}

export function BrowserView({ initialUrl = "", sessionId }: BrowserViewProps) {
  const { t } = useTranslation();
  const [url, setUrl] = useState(initialUrl);
  const [inputValue, setInputValue] = useState(initialUrl);
  const [loading, setLoading] = useState(false);
  const [isSecure, setIsSecure] = useState(false);
  const [history, setHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const webviewCreated = useRef(false);

  const [proxyEnabled, setProxyEnabled] = useState(false);
  const [proxyLoading, setProxyLoading] = useState(false);

  const activeSessionId = useStore((s) => s.activeSessionId);
  const isBrowserActive = !sessionId || activeSessionId === sessionId;

  useEffect(() => {
    invoke("pentest_get_system_proxy")
      .then((result) => {
        const proxy = result as [string, number] | null;
        if (proxy && proxy[0] === "127.0.0.1" && proxy[1] === 8090) {
          setProxyEnabled(true);
        }
      })
      .catch(() => {});
  }, []);

  const toggleProxy = useCallback(async () => {
    setProxyLoading(true);
    try {
      if (proxyEnabled) {
        await invoke("pentest_clear_system_proxy");
        setProxyEnabled(false);
      } else {
        await invoke("pentest_set_system_proxy", { host: "127.0.0.1", port: 8090 });
        setProxyEnabled(true);
      }
    } catch (e) {
      console.error("[BrowserView] proxy toggle error:", e);
    } finally {
      setProxyLoading(false);
    }
  }, [proxyEnabled]);

  const normalizeUrl = (input: string): string => {
    const trimmed = input.trim();
    if (!trimmed) return "";
    if (/^https?:\/\//i.test(trimmed)) return trimmed;
    if (/^localhost|^127\.|^\d+\.\d+\.\d+\.\d+/.test(trimmed)) return `http://${trimmed}`;
    if (trimmed.includes(".") && !trimmed.includes(" ")) return `https://${trimmed}`;
    return `https://www.google.com/search?q=${encodeURIComponent(trimmed)}`;
  };

  const getBounds = useCallback((): [number, number, number, number] | null => {
    const el = containerRef.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    if (rect.width < 10 || rect.height < 10) return null;
    return [rect.x, rect.y, rect.width, rect.height];
  }, []);

  const navigate = useCallback(async (targetUrl: string, addToHistory = true) => {
    const normalized = normalizeUrl(targetUrl);
    if (!normalized) return;
    setUrl(normalized);
    setInputValue(normalized);
    setLoading(true);
    setIsSecure(normalized.startsWith("https://"));

    if (addToHistory) {
      setHistory((prev) => {
        const newHistory = prev.slice(0, historyIndex + 1);
        newHistory.push(normalized);
        return newHistory;
      });
      setHistoryIndex((prev) => prev + 1);
    }

    const bounds = getBounds();
    if (!bounds) return;
    try {
      await invoke("pentest_browser_navigate", { url: normalized, bounds });
      webviewCreated.current = true;
    } catch (e) {
      console.error("[BrowserView] navigate error:", e);
    } finally {
      setTimeout(() => setLoading(false), 1500);
    }
  }, [historyIndex, getBounds]);

  const handleSubmit = useCallback((e: React.FormEvent) => {
    e.preventDefault();
    navigate(inputValue);
  }, [inputValue, navigate]);

  const goBack = useCallback(async () => {
    if (historyIndex > 0) {
      const newIndex = historyIndex - 1;
      setHistoryIndex(newIndex);
      navigate(history[newIndex], false);
    }
    try { await invoke("pentest_browser_go_back"); } catch { /* ignore */ }
  }, [historyIndex, history, navigate]);

  const goForward = useCallback(async () => {
    if (historyIndex < history.length - 1) {
      const newIndex = historyIndex + 1;
      setHistoryIndex(newIndex);
      navigate(history[newIndex], false);
    }
    try { await invoke("pentest_browser_go_forward"); } catch { /* ignore */ }
  }, [historyIndex, history, navigate]);

  const reload = useCallback(() => {
    if (url) navigate(url, false);
  }, [url, navigate]);

  const openExternal = useCallback(async () => {
    if (!url) return;
    const { open } = await import("@tauri-apps/plugin-shell");
    await open(url);
  }, [url]);

  // Hide/show webview based on whether this browser tab is active
  useEffect(() => {
    if (!webviewCreated.current) return;
    if (isBrowserActive) {
      requestAnimationFrame(() => {
        const bounds = getBounds();
        if (bounds) {
          invoke("pentest_browser_resize", { bounds }).catch(() => {});
          invoke("pentest_browser_show").catch(() => {});
        }
      });
    } else {
      invoke("pentest_browser_hide").catch(() => {});
    }
  }, [isBrowserActive, getBounds]);

  // Resize webview when container size changes
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => {
      if (!webviewCreated.current || !isBrowserActive) return;
      const bounds = getBounds();
      if (bounds) {
        invoke("pentest_browser_resize", { bounds }).catch(() => {});
      }
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [getBounds, isBrowserActive]);

  // Also handle window resize
  useEffect(() => {
    const handler = () => {
      if (!webviewCreated.current || !isBrowserActive) return;
      requestAnimationFrame(() => {
        const bounds = getBounds();
        if (bounds) {
          invoke("pentest_browser_resize", { bounds }).catch(() => {});
        }
      });
    };
    window.addEventListener("resize", handler);
    return () => window.removeEventListener("resize", handler);
  }, [getBounds, isBrowserActive]);

  // On mount: close any stale webview from previous HMR or tab reopen.
  // On unmount (tab close or tab switch): close the native webview.
  // Also listen for page unload (full refresh) to close the webview.
  useEffect(() => {
    invoke("pentest_browser_close").catch(() => {});
    webviewCreated.current = false;

    const handleUnload = () => {
      invoke("pentest_browser_close").catch(() => {});
    };
    window.addEventListener("beforeunload", handleUnload);

    return () => {
      window.removeEventListener("beforeunload", handleUnload);
      invoke("pentest_browser_close").catch(() => {});
      webviewCreated.current = false;
    };
  }, []);

  // Clear system proxy on app close if we set it
  useEffect(() => {
    const cleanup = () => {
      if (proxyEnabled) {
        invoke("pentest_clear_system_proxy").catch(() => {});
      }
    };
    window.addEventListener("beforeunload", cleanup);
    return () => window.removeEventListener("beforeunload", cleanup);
  }, [proxyEnabled]);

  const canGoBack = historyIndex > 0;
  const canGoForward = historyIndex < history.length - 1;

  return (
    <div className="h-full w-full flex flex-col bg-[var(--bg-primary)]">
      {/* URL bar */}
      <div className="h-10 flex-shrink-0 flex items-center gap-1.5 px-2 border-b border-border/10 bg-[var(--bg-secondary,var(--bg-primary))]">
        <NavButton icon={ArrowLeft} onClick={goBack} disabled={!canGoBack} title={t("browser.back")} />
        <NavButton icon={ArrowRight} onClick={goForward} disabled={!canGoForward} title={t("browser.forward")} />
        {loading ? (
          <NavButton icon={X} onClick={() => setLoading(false)} title={t("browser.stop")} />
        ) : (
          <NavButton icon={RotateCcw} onClick={reload} disabled={!url} title={t("browser.reload")} />
        )}
        <form onSubmit={handleSubmit} className="flex-1 mx-1">
          <div className="relative flex items-center">
            {isSecure ? (
              <Lock className="absolute left-2.5 w-3 h-3 text-green-400/60" />
            ) : (
              <Globe className="absolute left-2.5 w-3.5 h-3.5 text-muted-foreground/30" />
            )}
            <input ref={inputRef} value={inputValue} onChange={(e) => setInputValue(e.target.value)}
              placeholder={t("browser.urlPlaceholder")}
              className="w-full h-7 pl-8 pr-8 text-[11px] font-mono bg-[var(--bg-hover)]/40 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
              onFocus={(e) => e.target.select()}
              autoFocus={!url} />
            {loading && (
              <Loader2 className="absolute right-2.5 w-3 h-3 animate-spin text-accent/50" />
            )}
          </div>
        </form>
        <button
          type="button"
          onClick={toggleProxy}
          disabled={proxyLoading}
          title={proxyEnabled ? "ZAP Proxy ON (127.0.0.1:8090) — Click to disable" : "Enable ZAP Proxy (127.0.0.1:8090)"}
          className={cn(
            "p-1.5 rounded-md transition-all relative",
            proxyLoading && "opacity-50 cursor-wait",
            proxyEnabled
              ? "text-orange-400 bg-orange-400/10 hover:bg-orange-400/20"
              : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]"
          )}
        >
          {proxyEnabled ? <Shield className="w-3.5 h-3.5" /> : <ShieldOff className="w-3.5 h-3.5" />}
          {proxyEnabled && (
            <span className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-orange-400" />
          )}
        </button>
        <NavButton icon={ExternalLink} onClick={openExternal} disabled={!url} title={t("browser.openExternal")} />
      </div>

      {/* Content area — native webview renders on top of this div */}
      <div ref={containerRef} className="flex-1 relative min-h-0">
        {!url && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-4 text-muted-foreground/30">
            <Globe className="w-16 h-16" />
            <p className="text-[14px] font-medium">{t("browser.title")}</p>
            <p className="text-[12px] text-muted-foreground/20 max-w-xs text-center">
              {t("browser.hint")}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

function NavButton({ icon: Icon, onClick, disabled, title }: {
  icon: React.ComponentType<{ className?: string }>;
  onClick?: () => void;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <button type="button" onClick={onClick} disabled={disabled} title={title}
      className={cn(
        "p-1.5 rounded-md transition-colors",
        disabled ? "text-muted-foreground/15 cursor-default" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]"
      )}>
      <Icon className="w-3.5 h-3.5" />
    </button>
  );
}
