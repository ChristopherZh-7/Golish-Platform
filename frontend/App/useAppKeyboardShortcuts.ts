import { useEffect } from "react";
import {
  createKeyboardHandler,
  type KeyboardHandlerContext,
  useKeyboardHandlerContext,
} from "../hooks/useKeyboardHandlerContext";

type KeyboardCallbacks = Omit<KeyboardHandlerContext, "activeSessionId">;

/**
 * Wires the global keyboard shortcut handler.
 *
 * The current values are written to a stable ref on every render (matching
 * the existing refs pattern in App.tsx) so the actual `keydown` listener can
 * be installed exactly once with no React-state-driven re-subscriptions.
 */
export function useAppKeyboardShortcuts(callbacks: KeyboardCallbacks) {
  const keyboardContextRef = useKeyboardHandlerContext();

  // Keyboard shortcuts using refs pattern to avoid recreating the handler on every state change
  keyboardContextRef.current = {
    ...keyboardContextRef.current,
    ...callbacks,
  };

  // Set up the keyboard event listener once
  useEffect(() => {
    const handleKeyDown = createKeyboardHandler(keyboardContextRef);
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [keyboardContextRef]);
}
