import type React from "react";
import { useCallback, useRef } from "react";

interface SplitDragRef {
  startX: number;
  startY: number;
  dragging: boolean;
  tabId: string | null;
}

const INITIAL: SplitDragRef = { startX: 0, startY: 0, dragging: false, tabId: null };

interface UseSplitTabDragOptions {
  setShowMergeDropZone: React.Dispatch<React.SetStateAction<boolean>>;
  setSplitDragGhost: React.Dispatch<
    React.SetStateAction<{ x: number; y: number; name: string } | null>
  >;
  closeRightTab: (tabId?: string) => void;
}

export function useSplitTabDrag(opts: UseSplitTabDragOptions) {
  const { setShowMergeDropZone, setSplitDragGhost, closeRightTab } = opts;
  const dragRef = useRef<SplitDragRef>({ ...INITIAL });

  const onPointerDown = useCallback(
    (e: React.PointerEvent, tabId: string) => {
      if ((e.target as HTMLElement).closest("button")) return;
      e.preventDefault();
      dragRef.current = { startX: e.clientX, startY: e.clientY, dragging: false, tabId };
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
    },
    [],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent, tabName: string) => {
      const ref = dragRef.current;
      if (!ref.startX && !ref.startY) return;
      const dx = e.clientX - ref.startX;
      const dist = Math.sqrt(dx * dx + (e.clientY - ref.startY) ** 2);
      if (!ref.dragging && dist > 8) {
        ref.dragging = true;
        document.documentElement.classList.add("tab-dragging");
      }
      if (ref.dragging) {
        setShowMergeDropZone(dx < -40);
        setSplitDragGhost({ x: e.clientX, y: e.clientY, name: tabName });
      }
    },
    [setShowMergeDropZone, setSplitDragGhost],
  );

  const onPointerUp = useCallback(
    (e: React.PointerEvent) => {
      const ref = dragRef.current;
      if (ref.dragging && e.clientX - ref.startX < -40) closeRightTab(ref.tabId ?? undefined);
      dragRef.current = { ...INITIAL };
      setShowMergeDropZone(false);
      setSplitDragGhost(null);
      document.documentElement.classList.remove("tab-dragging");
    },
    [closeRightTab, setShowMergeDropZone, setSplitDragGhost],
  );

  return { dragRef, onPointerDown, onPointerMove, onPointerUp };
}
