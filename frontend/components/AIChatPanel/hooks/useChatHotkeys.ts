import { useCallback, type KeyboardEvent, type RefObject } from "react";

interface UseChatHotkeysOptions {
  textareaRef: RefObject<HTMLTextAreaElement>;
  onSend: () => void;
}

/**
 * Keyboard / textarea side-effects for the chat input.
 *
 *  - `handleKeyDown`: submit on Enter, allow Shift+Enter for newlines.
 *  - `handleTextareaInput`: auto-grow the textarea up to 160px.
 *
 * Kept as its own hook so the panel can drop ~15 lines of UI plumbing
 * and the input row is testable in isolation.
 */
export function useChatHotkeys({ textareaRef, onSend }: UseChatHotkeysOptions) {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        onSend();
      }
    },
    [onSend]
  );

  const handleTextareaInput = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [textareaRef]);

  return { handleKeyDown, handleTextareaInput };
}
