import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Terminal } from "@/components/Terminal/Terminal";
import { ThemeManager } from "@/lib/theme";
import type { SecurityTab } from "@/components/SecurityView/SecurityView";
import "@xterm/xterm/css/xterm.css";

const SecurityView = lazy(() =>
  import("@/components/SecurityView/SecurityView").then((m) => ({ default: m.SecurityView }))
);

const TAB_LABELS: Record<string, string> = {
  terminal: "Terminal",
  "security-history": "HTTP History", "security-sitemap": "Site Map",
  "security-scanner": "Scanner", "security-repeater": "Repeater",
  "security-findings": "Findings", "security-audit": "Audit Log",
  "security-passive": "Passive Scan", "security-vault": "Credential Vault",
};

interface DetachedViewProps {
  sessionId: string;
  tabType: string;
}

export function DetachedView({ sessionId, tabType }: DetachedViewProps) {
  const [title, setTitle] = useState(TAB_LABELS[tabType] || tabType);
  const initialized = useRef(false);

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;
    ThemeManager.initialize().then(() => ThemeManager.tryLoadPersistedTheme()).catch(() => {});
  }, []);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    const unlisten = currentWindow.onCloseRequested(async () => {
      try {
        await emit("detached-window-closed", { session_id: sessionId });
      } catch { /* ignore */ }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [sessionId]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<{ session_id: string; title: string }>("detached-title-update", (event) => {
      if (event.payload.session_id === sessionId) {
        setTitle(event.payload.title);
        getCurrentWindow().setTitle(event.payload.title).catch(() => {});
      }
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, [sessionId]);

  const isSecuritySubTab = tabType.startsWith("security-") && tabType !== "security-all";
  const securitySubTab = isSecuritySubTab ? tabType.replace("security-", "") as SecurityTab : null;

  if (tabType === "terminal") {
    return <DetachedTerminal sessionId={sessionId} title={title} />;
  }

  if (tabType === "security-all") {
    return <DetachedSecurity sessionId={sessionId} title="Security" securitySubTab={undefined} />;
  }

  if (isSecuritySubTab && securitySubTab) {
    return <DetachedSecurity sessionId={sessionId} title={title} securitySubTab={securitySubTab} />;
  }

  return (
    <div className="h-screen w-screen flex items-center justify-center bg-[var(--bg-primary,#1a1b26)]">
      <span className="text-[var(--text-secondary,#787c99)]">Detached: {tabType}</span>
    </div>
  );
}

const startWindowDrag = async (e: React.MouseEvent) => {
  e.preventDefault();
  try { await getCurrentWindow().startDragging(); } catch { /* ignore */ }
};
const stopPropagation = (e: React.MouseEvent) => { e.stopPropagation(); };

function DetachedTerminal({ sessionId, title }: { sessionId: string; title: string }) {
  const themeInit = useRef(false);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    if (themeInit.current) return;
    themeInit.current = true;
    ThemeManager.initialize().then(() => ThemeManager.tryLoadPersistedTheme()).catch(() => {}).finally(() => setReady(true));
  }, []);

  const handleClose = useCallback(async () => {
    try { await emit("detached-window-closed", { session_id: sessionId }); } catch { /* ignore */ }
    try { await invoke("close_detached_window", { sessionId }); } catch { /* ignore */ }
    try { await getCurrentWindow().destroy(); } catch { /* ignore */ }
  }, [sessionId]);

  if (!ready) {
    return <div style={{ width: "100vw", height: "100vh", background: "#1a1b26" }} />;
  }

  return (
    <div className="h-screen w-screen flex flex-col bg-background text-foreground overflow-hidden">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: drag region */}
      <div className="h-[31px] flex-shrink-0 flex items-center select-none border-b border-border/10" onMouseDown={startWindowDrag}>
        <div className="w-[70px] flex-shrink-0" />
        <span className="text-[11px] font-mono text-foreground/80 truncate">{title}</span>
        <span className="ml-2 text-[9px] px-1.5 py-0.5 rounded-full bg-accent/10 text-accent/60 font-medium flex-shrink-0">Detached</span>
        <div className="flex-1" />
        {/* biome-ignore lint/a11y/noStaticElementInteractions: prevent drag */}
        <div onMouseDown={stopPropagation}>
          <button
            type="button"
            onClick={handleClose}
            className="flex items-center gap-1 px-2 py-1 mr-2 rounded text-[10px] text-muted-foreground/60 hover:text-destructive hover:bg-destructive/10 transition-colors"
            title="Close window"
          >
            <svg width="8" height="8" viewBox="0 0 8 8" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M1 1L7 7M7 1L1 7" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
            </svg>
            Close
          </button>
        </div>
      </div>
      <div className="flex-1 min-h-0 p-1">
        <Terminal sessionId={sessionId} />
      </div>
    </div>
  );
}

function DetachedSecurity({ sessionId, title, securitySubTab }: { sessionId: string; title: string; securitySubTab?: SecurityTab }) {
  const [ready, setReady] = useState(false);
  const themeInitialized = useRef(false);

  useEffect(() => {
    if (themeInitialized.current) return;
    themeInitialized.current = true;
    (async () => {
      try {
        await ThemeManager.initialize();
        await ThemeManager.tryLoadPersistedTheme();
      } catch { /* ignore */ }
      await getCurrentWindow().setFocus().catch(() => {});
      setReady(true);
    })();
  }, []);

  const handleClose = useCallback(async () => {
    try { await emit("detached-window-closed", { session_id: sessionId }); } catch { /* ignore */ }
    try { await invoke("close_detached_window", { sessionId }); } catch { /* ignore */ }
    try { await getCurrentWindow().destroy(); } catch { /* ignore */ }
  }, [sessionId]);

  if (!ready) {
    return (
      <div style={{ width: "100vw", height: "100vh", display: "flex", alignItems: "center", justifyContent: "center", background: "var(--background, #1a1b26)", color: "var(--muted-foreground, #787c99)" }}>
        <span style={{ fontSize: 12 }}>Loading...</span>
      </div>
    );
  }

  return (
    <div className="h-screen w-screen flex flex-col bg-background text-foreground overflow-hidden">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: drag region */}
      <div className="h-[31px] flex-shrink-0 flex items-center select-none border-b border-border/10" onMouseDown={startWindowDrag}>
        <div className="w-[70px] flex-shrink-0" />
        <span className="text-[11px] font-medium text-foreground/80 truncate">{title}</span>
        <span className="ml-2 text-[9px] px-1.5 py-0.5 rounded-full bg-accent/10 text-accent/60 font-medium flex-shrink-0">Detached</span>
        <div className="flex-1" />
        {/* biome-ignore lint/a11y/noStaticElementInteractions: prevent drag */}
        <div className="flex items-center gap-1" onMouseDown={stopPropagation}>
          <button
            type="button"
            onClick={handleClose}
            className="flex items-center gap-1 px-2 py-1 rounded text-[10px] text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
            title="Dock back to main window"
          >
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" xmlns="http://www.w3.org/2000/svg">
              <rect x="0.5" y="0.5" width="9" height="9" rx="1" stroke="currentColor" strokeWidth="1" />
              <path d="M3 5H7M5 3V7" stroke="currentColor" strokeWidth="1" strokeLinecap="round" />
            </svg>
            Dock
          </button>
          <button
            type="button"
            onClick={handleClose}
            className="flex items-center gap-1 px-2 py-1 mr-2 rounded text-[10px] text-muted-foreground/60 hover:text-destructive hover:bg-destructive/10 transition-colors"
            title="Close window"
          >
            <svg width="8" height="8" viewBox="0 0 8 8" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M1 1L7 7M7 1L1 7" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
            </svg>
            Close
          </button>
        </div>
      </div>
      <div className="flex-1 min-h-0 overflow-auto">
        <Suspense fallback={<div className="h-full flex items-center justify-center"><span className="text-muted-foreground/30 text-sm">Loading panel...</span></div>}>
          <SecurityView standaloneTab={securitySubTab} />
        </Suspense>
      </div>
    </div>
  );
}
