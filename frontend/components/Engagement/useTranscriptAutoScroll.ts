import {
  type MutableRefObject,
  type UIEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  type WheelEvent,
} from "react";
import { isNearScrollBottom } from "@/lib/scroll-stickiness";

export interface TranscriptAutoScrollState<
  Viewport extends HTMLElement = HTMLDivElement,
  Content extends HTMLElement = HTMLDivElement,
> {
  viewportRef: MutableRefObject<Viewport | null>;
  contentRef: MutableRefObject<Content | null>;
  onViewportScroll: (event: UIEvent<Viewport>) => void;
  onViewportWheel: (event: WheelEvent<Viewport>) => void;
  syncViewportPosition: (viewport: Viewport) => void;
}

function transcriptScrollTarget(viewport: HTMLElement, content: HTMLElement): number {
  const viewportRect = viewport.getBoundingClientRect();
  const contentRect = content.getBoundingClientRect();
  const contentTop = contentRect.top - viewportRect.top + viewport.scrollTop;
  const maximum = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
  return Math.max(0, Math.min(maximum, contentTop + content.scrollHeight - viewport.clientHeight));
}

/**
 * Follow a growing transcript until the user scrolls upward. Scrolling back
 * near the bottom restores follow mode; content below the transcript wrapper
 * (for example Evidence) is deliberately excluded from the follow target.
 */
export function useTranscriptAutoScroll<
  Viewport extends HTMLElement = HTMLDivElement,
  Content extends HTMLElement = HTMLDivElement,
>(followIdentity: string | null = null): TranscriptAutoScrollState<Viewport, Content> {
  const viewportRef = useRef<Viewport | null>(null);
  const contentRef = useRef<Content | null>(null);
  const followingRef = useRef(true);
  const previousScrollTopRef = useRef(0);
  const previousScrollHeightRef = useRef(0);
  const scrollFrameRef = useRef<number | null>(null);

  const previousFollowIdentityRef = useRef(followIdentity);

  const syncViewportPosition = useCallback((viewport: Viewport) => {
    const content = contentRef.current;
    const viewportScrollHeight = viewport.scrollHeight;
    const target = content
      ? transcriptScrollTarget(viewport, content)
      : Math.max(0, viewportScrollHeight - viewport.clientHeight);
    const metrics = {
      scrollTop: viewport.scrollTop,
      scrollHeight: target + viewport.clientHeight,
      clientHeight: viewport.clientHeight,
    };
    const movedUp = metrics.scrollTop < previousScrollTopRef.current - 1;
    const contentShrank = viewportScrollHeight < previousScrollHeightRef.current;
    const belowTranscript = metrics.scrollTop > target + 1;
    const wasBelowTranscript = previousScrollTopRef.current > target + 1;
    const nearTranscriptBottom = isNearScrollBottom(metrics);

    // Content growth and programmatic scroll anchoring can emit `scroll`
    // before the follow frame runs. Reading supplementary content below the
    // transcript is also an intentional pause, while returning from that
    // content to the transcript boundary restores follow mode.
    if (belowTranscript && !contentShrank) {
      followingRef.current = false;
    } else if (movedUp && !contentShrank) {
      followingRef.current = wasBelowTranscript && nearTranscriptBottom;
    } else if (!movedUp && nearTranscriptBottom) {
      followingRef.current = true;
    }
    previousScrollTopRef.current = viewport.scrollTop;
    previousScrollHeightRef.current = viewportScrollHeight;
  }, []);

  const scheduleFollow = useCallback(() => {
    if (!followingRef.current || scrollFrameRef.current != null) return;
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = null;
      if (!followingRef.current) return;
      const viewport = viewportRef.current;
      const content = contentRef.current;
      if (!viewport || !content) return;
      viewport.scrollTop = transcriptScrollTarget(viewport, content);
      // The browser clamps scrollTop. Store the actual value so its resulting
      // scroll event is never mistaken for a user scrolling upward.
      previousScrollTopRef.current = viewport.scrollTop;
      previousScrollHeightRef.current = viewport.scrollHeight;
    });
  }, []);

  const onViewportScroll = useCallback(
    (event: UIEvent<Viewport>) => syncViewportPosition(event.currentTarget),
    [syncViewportPosition]
  );

  const onViewportWheel = useCallback((event: WheelEvent<Viewport>) => {
    if (event.deltaY < 0) followingRef.current = false;
  }, []);

  useLayoutEffect(() => {
    if (previousFollowIdentityRef.current === followIdentity) return;
    previousFollowIdentityRef.current = followIdentity;
    followingRef.current = true;
    previousScrollTopRef.current = 0;
    previousScrollHeightRef.current = 0;
    if (scrollFrameRef.current != null) {
      cancelAnimationFrame(scrollFrameRef.current);
      scrollFrameRef.current = null;
    }
  }, [followIdentity]);

  // A running tool updates its card without adding a transcript entry. Run
  // after every committed render; ResizeObserver below covers later layout-only
  // growth such as wrapping and streamed tool output.
  useLayoutEffect(() => {
    scheduleFollow();
  });

  useEffect(() => {
    if (typeof ResizeObserver === "undefined" || !contentRef.current) return;
    const observer = new ResizeObserver(scheduleFollow);
    observer.observe(contentRef.current);
    return () => observer.disconnect();
  }, [followIdentity, scheduleFollow]);

  useEffect(
    () => () => {
      if (scrollFrameRef.current != null) cancelAnimationFrame(scrollFrameRef.current);
    },
    []
  );

  return {
    viewportRef,
    contentRef,
    onViewportScroll,
    onViewportWheel,
    syncViewportPosition,
  };
}
