import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Custom horizontal-scrollbar state for the chat conversation tab strip.
 *
 * Encapsulates:
 *  - the `tabsRef` element ref the strip must attach,
 *  - hover state for showing the scrollbar track,
 *  - a `scrollThumb` snapshot recomputed on scroll/resize,
 *  - mouse-wheel→horizontal-scroll redirection,
 *  - a thumb-drag handler that tracks scroll-left while the user drags.
 */
export interface ChatTabsScrollbarState {
  tabsRef: React.MutableRefObject<HTMLDivElement | null>;
  tabsHovered: boolean;
  setTabsHovered: React.Dispatch<React.SetStateAction<boolean>>;
  scrollThumb: { left: number; width: number; visible: boolean };
  handleThumbDragStart: (e: React.MouseEvent) => void;
}

export function useChatTabsScrollbar(conversationCount: number): ChatTabsScrollbarState {
  const tabsRef = useRef<HTMLDivElement | null>(null);
  const [tabsHovered, setTabsHovered] = useState(false);
  const [scrollThumb, setScrollThumb] = useState({ left: 0, width: 0, visible: false });
  const thumbDragRef = useRef<{ startX: number; startScroll: number } | null>(null);

  const updateScrollThumb = useCallback(() => {
    const el = tabsRef.current;
    if (!el) return;
    const hasOverflow = el.scrollWidth > el.clientWidth + 1;
    if (!hasOverflow) {
      setScrollThumb({ left: 0, width: 0, visible: false });
      return;
    }
    const ratio = el.clientWidth / el.scrollWidth;
    const thumbWidth = Math.max(ratio * 100, 10);
    const scrollRange = el.scrollWidth - el.clientWidth;
    const thumbLeft = scrollRange > 0 ? (el.scrollLeft / scrollRange) * (100 - thumbWidth) : 0;
    setScrollThumb({ left: thumbLeft, width: thumbWidth, visible: true });
  }, []);

  useEffect(() => {
    const el = tabsRef.current;
    if (!el) return;
    updateScrollThumb();
    el.addEventListener("scroll", updateScrollThumb, { passive: true });
    const observer = new ResizeObserver(updateScrollThumb);
    observer.observe(el);
    return () => {
      el.removeEventListener("scroll", updateScrollThumb);
      observer.disconnect();
    };
  }, [updateScrollThumb, conversationCount]);

  // Mouse wheel -> horizontal scroll
  useEffect(() => {
    const el = tabsRef.current;
    if (!el) return;
    const handler = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        e.preventDefault();
        el.scrollLeft += e.deltaY;
      }
    };
    el.addEventListener("wheel", handler, { passive: false });
    return () => el.removeEventListener("wheel", handler);
  }, []);

  const handleThumbDragStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const el = tabsRef.current;
    if (!el) return;
    thumbDragRef.current = { startX: e.clientX, startScroll: el.scrollLeft };
    const onMove = (ev: MouseEvent) => {
      if (!thumbDragRef.current || !tabsRef.current) return;
      const trackEl = tabsRef.current;
      const dx = ev.clientX - thumbDragRef.current.startX;
      const trackWidth = trackEl.clientWidth;
      const scrollRange = trackEl.scrollWidth - trackEl.clientWidth;
      trackEl.scrollLeft = thumbDragRef.current.startScroll + (dx / trackWidth) * scrollRange;
    };
    const onUp = () => {
      thumbDragRef.current = null;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, []);

  return {
    tabsRef,
    tabsHovered,
    setTabsHovered,
    scrollThumb,
    handleThumbDragStart,
  };
}
