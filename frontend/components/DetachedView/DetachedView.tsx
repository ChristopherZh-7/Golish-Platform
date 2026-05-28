import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@/components/Terminal/Terminal";
import { closeDetached } from "@/lib/api/window";
import { onCustomEvent, sendEvent } from "@/lib/events";
import { runTauriUnlistenFn, runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import { ThemeManager } from "@/lib/theme";
import "@xterm/xterm/css/xterm.css";

const TAB_LABELS: Record<string, string> = {
  terminal: "Terminal",
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
    ThemeManager.tryLoadPersistedTheme().catch(() => {});
  }, []);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    const unlisten = currentWindow.onCloseRequested(async () => {
      try {
        await sendEvent("detached-window-closed", { session_id: sessionId });
      } catch {
        /* ignore */
      }
    });
    return () => {
      runTauriUnlistenFromPromise(unlisten);
    };
  }, [sessionId]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    onCustomEvent<{ session_id: string; title: string }>("detached-title-update", (payload) => {
      if (payload.session_id === sessionId) {
        setTitle(payload.title);
        getCurrentWindow()
          .setTitle(payload.title)
          .catch(() => {});
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      runTauriUnlistenFn(unlisten);
    };
  }, [sessionId]);

  if (tabType === "terminal") {
    return <DetachedTerminal sessionId={sessionId} title={title} />;
  }

  return (
    <div className="h-screen w-screen flex items-center justify-center bg-[var(--bg-primary,#1a1b26)]">
      <span className="text-[var(--text-secondary,#787c99)]">Detached: {tabType}</span>
    </div>
  );
}

const startWindowDrag = async (e: React.MouseEvent) => {
  e.preventDefault();
  try {
    await getCurrentWindow().startDragging();
  } catch {
    /* ignore */
  }
};
const stopPropagation = (e: React.MouseEvent) => {
  e.stopPropagation();
};

async function closeDetachedWindow(sessionId: string) {
  try {
    await sendEvent("detached-window-closed", { session_id: sessionId });
  } catch {
    /* ignore */
  }
  try {
    await closeDetached(sessionId);
  } catch {
    /* ignore */
  }
  try {
    await getCurrentWindow().destroy();
  } catch {
    /* ignore */
  }
}

function DetachedTerminal({ sessionId, title }: { sessionId: string; title: string }) {
  const themeInit = useRef(false);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    if (themeInit.current) return;
    themeInit.current = true;
    ThemeManager.tryLoadPersistedTheme()
      .catch(() => {})
      .finally(() => setReady(true));
  }, []);

  const handleClose = useCallback(() => closeDetachedWindow(sessionId), [sessionId]);

  if (!ready) {
    return <div style={{ width: "100vw", height: "100vh", background: "#1a1b26" }} />;
  }

  return (
    <div className="h-screen w-screen flex flex-col bg-background text-foreground overflow-hidden">
      <div
        className="h-[31px] flex-shrink-0 flex items-center select-none border-b border-border/10"
        onMouseDown={startWindowDrag}
      >
        <div className="w-[70px] flex-shrink-0" />
        <span className="text-[11px] font-mono text-foreground/80 truncate">{title}</span>
        <span className="ml-2 text-[9px] px-1.5 py-0.5 rounded-full bg-accent/10 text-accent/60 font-medium flex-shrink-0">
          Detached
        </span>
        <div className="flex-1" />
        <div onMouseDown={stopPropagation}>
          <button
            type="button"
            onClick={handleClose}
            className="flex items-center gap-1 px-2 py-1 mr-2 rounded text-[10px] text-muted-foreground/60 hover:text-destructive hover:bg-destructive/10 transition-colors"
            title="Close window"
          >
            <svg
              aria-hidden="true"
              width="8"
              height="8"
              viewBox="0 0 8 8"
              fill="none"
              xmlns="http://www.w3.org/2000/svg"
            >
              <path
                d="M1 1L7 7M7 1L1 7"
                stroke="currentColor"
                strokeWidth="1.2"
                strokeLinecap="round"
              />
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
