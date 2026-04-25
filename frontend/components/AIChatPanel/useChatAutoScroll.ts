import { useEffect, useRef } from "react";

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

  useEffect(() => {
    if (!userScrolledUpRef.current) {
      const container = messagesContainerRef.current;
      if (container) {
        container.scrollTop = container.scrollHeight;
      }
    }
  }, [messages]);

  return {
    messagesContainerRef,
    userScrolledUpRef,
    chatAtBottomRef,
  };
}
