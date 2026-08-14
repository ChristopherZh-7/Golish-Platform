import { useCallback, useEffect, useLayoutEffect, useRef } from "react";

/**
 * Auto-scroll the chat messages container to the bottom on new messages,
 * unless the user has explicitly scrolled up.
 *
 * Wheel events are the **only** signal that toggles the user-scrolled-up
 * flag — programmatic `scrollTop` assignments must NOT accidentally
 * re-enable auto-scroll.
 */
export interface ChatAutoScrollState {
  messagesContainerRef: React.MutableRefObject<HTMLDivElement | null>;
  /**
   * `true` when the user has wheeled away from the bottom of the chat;
   * exposed so callers (e.g. submit handlers) can reset it on new sends.
   */
  userScrolledUpRef: React.MutableRefObject<boolean>;
  /** `true` when the latest scroll position is within 80px of the bottom. */
  chatAtBottomRef: React.MutableRefObject<boolean>;
}

export interface ChatAutoScrollOptions {
  /** The heavy ChatPanel DOM is absent while a full-width detail owns the UI. */
  active?: boolean;
  /** Resets/restores viewport intent independently for each conversation. */
  scrollKey?: string;
}

interface ChatScrollSnapshot {
  scrollTop: number;
  userScrolledUp: boolean;
  atBottom: boolean;
}

function isChatBottom(container: HTMLDivElement): boolean {
  const { scrollTop, scrollHeight, clientHeight } = container;
  return scrollHeight - scrollTop - clientHeight < 80;
}

export function useChatAutoScroll<T>(
  messages: readonly T[],
  { active = true, scrollKey = "default" }: ChatAutoScrollOptions = {}
): ChatAutoScrollState {
  const messagesContainerRef = useRef<HTMLDivElement | null>(null);
  const chatAtBottomRef = useRef(true);
  const userScrolledUpRef = useRef(false);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const scrollFrameRef = useRef<number | null>(null);
  const boundContainerRef = useRef<HTMLDivElement | null>(null);
  const boundScrollKeyRef = useRef<string | null>(null);
  const snapshotsRef = useRef(new Map<string, ChatScrollSnapshot>());

  const rememberPosition = useCallback((container: HTMLDivElement, key: string) => {
    const atBottom = isChatBottom(container);
    chatAtBottomRef.current = atBottom;
    snapshotsRef.current.set(key, {
      scrollTop: container.scrollTop,
      userScrolledUp: userScrolledUpRef.current,
      atBottom,
    });
  }, []);

  const schedulePosition = useCallback((position: "bottom" | number, force = false) => {
    if (!force && userScrolledUpRef.current) return;
    if (scrollFrameRef.current != null) {
      if (!force) return;
      cancelAnimationFrame(scrollFrameRef.current);
    }
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = null;
      if (!force && userScrolledUpRef.current) return;
      const container = messagesContainerRef.current;
      if (!container) return;
      container.scrollTop = position === "bottom" ? container.scrollHeight : position;
      chatAtBottomRef.current = isChatBottom(container);
      snapshotsRef.current.set(boundScrollKeyRef.current ?? "default", {
        scrollTop: container.scrollTop,
        userScrolledUp: userScrolledUpRef.current,
        atBottom: chatAtBottomRef.current,
      });
    });
  }, []);

  const scheduleScrollToBottom = useCallback(() => {
    schedulePosition("bottom");
  }, [schedulePosition]);

  useEffect(() => {
    if (!active) return;
    const container = messagesContainerRef.current;
    if (!container) return;

    const handleWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) {
        userScrolledUpRef.current = true;
      } else if (e.deltaY > 0) {
        requestAnimationFrame(() => {
          if (isChatBottom(container)) userScrolledUpRef.current = false;
        });
      }
    };

    const handleScroll = () => {
      rememberPosition(container, scrollKey);
    };

    container.addEventListener("wheel", handleWheel, { passive: true });
    container.addEventListener("scroll", handleScroll, { passive: true });
    return () => {
      rememberPosition(container, scrollKey);
      container.removeEventListener("wheel", handleWheel);
      container.removeEventListener("scroll", handleScroll);
    };
  }, [active, rememberPosition, scrollKey]);

  // Keep the chat pinned to the bottom whenever the scrollable *content* grows,
  // not only when the `messages` array changes. The preparing indicator,
  // workflow card, and ask-human prompt mount from non-message state after the
  // array has already settled; a ResizeObserver on the content wrapper is what
  // catches those height changes (otherwise they land below the fold until the
  // user scrolls down manually). The user-scrolled-up guard still wins.
  //
  // The observer is created lazily on the first layout pass and re-pointed at
  // the live content wrapper on every messages change, since React swaps that
  // node when the empty state gives way to the first message. Running this in a
  // layout effect (re-runs on every messages change, incl. streaming chunks
  // that swap the array reference) keeps content arriving at the bottom without
  // a paint flicker.
  useLayoutEffect(() => {
    if (!active) {
      const previous = boundContainerRef.current;
      const previousKey = boundScrollKeyRef.current;
      if (previous && previousKey) rememberPosition(previous, previousKey);
      resizeObserverRef.current?.disconnect();
      boundContainerRef.current = null;
      boundScrollKeyRef.current = null;
      if (scrollFrameRef.current != null) {
        cancelAnimationFrame(scrollFrameRef.current);
        scrollFrameRef.current = null;
      }
      return;
    }
    const container = messagesContainerRef.current;
    if (!container) return;
    const viewportChanged =
      boundContainerRef.current !== container || boundScrollKeyRef.current !== scrollKey;
    boundContainerRef.current = container;
    boundScrollKeyRef.current = scrollKey;

    if (!resizeObserverRef.current) {
      resizeObserverRef.current = new ResizeObserver(() => {
        scheduleScrollToBottom();
      });
    }
    const ro = resizeObserverRef.current;
    ro.disconnect();
    ro.observe(container.firstElementChild ?? container);

    if (viewportChanged) {
      const snapshot = snapshotsRef.current.get(scrollKey);
      userScrolledUpRef.current = snapshot?.userScrolledUp ?? false;
      chatAtBottomRef.current = snapshot?.atBottom ?? true;
      schedulePosition(snapshot?.userScrolledUp ? snapshot.scrollTop : "bottom", true);
    } else {
      scheduleScrollToBottom();
    }
  }, [active, messages, rememberPosition, schedulePosition, scheduleScrollToBottom, scrollKey]);

  // Tear down the observer on unmount.
  useEffect(() => {
    return () => {
      resizeObserverRef.current?.disconnect();
      resizeObserverRef.current = null;
      if (scrollFrameRef.current != null) {
        cancelAnimationFrame(scrollFrameRef.current);
        scrollFrameRef.current = null;
      }
    };
  }, []);

  return {
    messagesContainerRef,
    userScrolledUpRef,
    chatAtBottomRef,
  };
}
