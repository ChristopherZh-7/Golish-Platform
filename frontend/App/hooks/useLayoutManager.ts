import type React from "react";
import { useCallback, useEffect, useRef, useState } from "react";

type SplitDragGhost = { x: number; y: number; name: string };
type SplitDragState = { startX: number; startY: number; dragging: boolean; tabId: string | null };
const SPLIT_DRAG_INITIAL: SplitDragState = { startX: 0, startY: 0, dragging: false, tabId: null };

/**
 * Owns the local UI state for the optional right-side split column:
 *  - the list of tab IDs currently shown on the right
 *  - the active right tab + drag/ghost state for tab tear-off
 *  - the drop-zone hint flags
 *  - the persisted right-panel width
 *  - the imperative resize handle handler
 *
 * Setters for the slice consumed by `useAppLifecycle` (split-tab DOM events)
 * are exposed alongside the state so App can wire them up directly.
 */
export function useLayoutManager() {
  const [rightPanelTabs, setRightPanelTabs] = useState<string[]>([]);
  const [rightActiveTab, setRightActiveTab] = useState<string | null>(null);
  const [showSplitDropZone, setShowSplitDropZone] = useState(false);
  const [, setShowMergeDropZone] = useState(false);
  const [rightPanelWidth, setRightPanelWidth] = useState(() => {
    const saved = localStorage.getItem("golish-right-panel-width");
    return saved ? Number(saved) : 340;
  });
  const rightPanelWidthRef = useRef(rightPanelWidth);
  useEffect(() => {
    rightPanelWidthRef.current = rightPanelWidth;
  }, [rightPanelWidth]);
  const [splitDragGhost, setSplitDragGhost] = useState<SplitDragGhost | null>(null);
  const splitDragRef = useRef<SplitDragState>(SPLIT_DRAG_INITIAL);

  const closeRightTab = useCallback(
    (tabId?: string) => {
      const target = tabId || rightActiveTab;
      if (!target) return;
      setRightPanelTabs((prev) => {
        const next = prev.filter((id) => id !== target);
        if (next.length === 0) {
          setRightActiveTab(null);
        } else if (rightActiveTab === target) {
          setRightActiveTab(next[next.length - 1]);
        }
        return next;
      });
    },
    [rightActiveTab]
  );

  const handlePanelResizeStart = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = rightPanelWidthRef.current;
    const panel = document.querySelector<HTMLElement>("[data-right-panel]");

    document.documentElement.style.cursor = "col-resize";
    document.documentElement.style.userSelect = "none";
    if (panel) panel.style.transition = "none";

    const onMove = (ev: PointerEvent) => {
      const delta = startX - ev.clientX;
      const w = Math.max(220, Math.min(600, startWidth + delta));
      rightPanelWidthRef.current = w;
      if (panel) panel.style.width = `${w}px`;
    };

    const onUp = () => {
      document.documentElement.style.cursor = "";
      document.documentElement.style.userSelect = "";
      if (panel) panel.style.transition = "";
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
      setRightPanelWidth(rightPanelWidthRef.current);
      try {
        localStorage.setItem("golish-right-panel-width", String(rightPanelWidthRef.current));
      } catch {
        /* ignore */
      }
    };

    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
  }, []);

  return {
    rightPanelTabs,
    setRightPanelTabs,
    rightActiveTab,
    setRightActiveTab,
    showSplitDropZone,
    setShowSplitDropZone,
    setShowMergeDropZone,
    rightPanelWidth,
    splitDragGhost,
    setSplitDragGhost,
    splitDragRef,
    closeRightTab,
    handlePanelResizeStart,
  };
}
