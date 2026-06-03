import { useEffect, useLayoutEffect, useRef } from "react";

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

export function useChatAutoScroll<T>(messages: readonly T[]): ChatAutoScrollState {
  const messagesContainerRef = useRef<HTMLDivElement | null>(null);
  const chatAtBottomRef = useRef(true);
  const userScrolledUpRef = useRef(false);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);

  useEffect(() => {
    const container = messagesContainerRef.current;
    if (!container) return;

    const isAtBottom = () => {
      const { scrollTop, scrollHeight, clientHeight } = container;
      return scrollHeight - scrollTop - clientHeight < 80;
    };

    const handleWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) {
        userScrolledUpRef.current = true;
      } else if (e.deltaY > 0) {
        requestAnimationFrame(() => {
          if (isAtBottom()) userScrolledUpRef.current = false;
        });
      }
    };

    const handleScroll = () => {
      chatAtBottomRef.current = isAtBottom();
    };

    container.addEventListener("wheel", handleWheel, { passive: true });
    container.addEventListener("scroll", handleScroll, { passive: true });
    return () => {
      container.removeEventListener("wheel", handleWheel);
      container.removeEventListener("scroll", handleScroll);
    };
  }, []);

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
    const container = messagesContainerRef.current;
    if (!container) return;

    if (!resizeObserverRef.current) {
      resizeObserverRef.current = new ResizeObserver(() => {
        if (userScrolledUpRef.current) return;
        const el = messagesContainerRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });
    }
    const ro = resizeObserverRef.current;
    ro.disconnect();
    ro.observe(container.firstElementChild ?? container);

    if (userScrolledUpRef.current) return;
    container.scrollTop = container.scrollHeight;
  }, [messages]);

  // Tear down the observer on unmount.
  useEffect(() => {
    return () => {
      resizeObserverRef.current?.disconnect();
      resizeObserverRef.current = null;
    };
  }, []);

  return {
    messagesContainerRef,
    userScrolledUpRef,
    chatAtBottomRef,
  };
}
