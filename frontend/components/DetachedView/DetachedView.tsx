import { useCallback, useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Terminal } from "@/components/Terminal/Terminal";
import { ThemeManager } from "@/lib/theme";
import "@xterm/xterm/css/xterm.css";

interface DetachedViewProps {
  sessionId: string;
  tabType: string;
}

export function DetachedView({ sessionId, tabType }: DetachedViewProps) {
  const [title, setTitle] = useState(tabType === "terminal" ? "Terminal" : tabType);
  const initialized = useRef(false);

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;
    ThemeManager.initialize().catch(() => {});
  }, []);

  // Notify main window when this detached window is closed
  useEffect(() => {
    const currentWindow = getCurrentWindow();
    const unlisten = currentWindow.onCloseRequested(async () => {
      try {
        await emit("detached-window-closed", { session_id: sessionId });
      } catch {
        // ignore
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [sessionId]);

  // Listen for title update events from main window
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

  const handleDragStart = useCallback(async (e: React.MouseEvent) => {
    e.preventDefault();
    try {
      await getCurrentWindow().startDragging();
    } catch { /* ignore */ }
  }, []);

  if (tabType === "terminal") {
    return (
      <div className="h-screen w-screen flex flex-col bg-[var(--bg-primary,#1a1b26)] overflow-hidden">
        {/* Minimal title bar */}
        {/* biome-ignore lint/a11y/noStaticElementInteractions: drag region */}
        <div
          className="h-8 flex-shrink-0 flex items-center px-3 gap-2 bg-[var(--bg-secondary,#16161e)] border-b border-[var(--border-subtle,#2a2b3d33)] select-none cursor-grab active:cursor-grabbing"
          onMouseDown={handleDragStart}
        >
          <span className="text-[11px] font-mono text-[var(--text-secondary,#787c99)] truncate">{title}</span>
          <div className="flex-1" />
          <span className="text-[9px] text-[var(--text-tertiary,#565869)] opacity-60">Detached</span>
        </div>
        {/* Terminal fills remaining space */}
        <div className="flex-1 min-h-0 p-1">
          <Terminal sessionId={sessionId} />
        </div>
      </div>
    );
  }

  // Non-terminal tabs: show a placeholder for now
  return (
    <div className="h-screen w-screen flex items-center justify-center bg-[var(--bg-primary,#1a1b26)]">
      <span className="text-[var(--text-secondary,#787c99)]">Detached: {tabType}</span>
    </div>
  );
}
