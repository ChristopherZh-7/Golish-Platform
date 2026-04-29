import type React from "react";
import { useCallback, useMemo } from "react";
import type { ActivityView } from "../../components/ActivityBar/ActivityBar";
import { useStore } from "../../store";

export interface ActivityViewControls {
  /**
   * Toggle a fullscreen activity view: opens it if currently closed (or showing a
   * different view), closes it if currently active.
   */
  toggleView: (view: NonNullable<ActivityView>) => void;
  /** Close any active activity overlay. */
  closeView: () => void;
  /**
   * If the active session is a non-terminal tab, switch focus to the first terminal
   * tab. Otherwise toggle the bottom terminal panel. Always closes any active overlay.
   *
   * Centralised here because three callers (App keyboard shortcuts, AppShell
   * ActivityBar button, AppShell CommandPalette) used to keep their own copies of
   * this 15-line block — they would silently drift apart on any future change.
   */
  toggleBottomTerminal: () => void;
  /** Close any overlay and focus the right-side AI chat input on the next frame. */
  focusAiChat: () => void;
}

/**
 * Stable handlers that drive the activity-view (left-side overlay) state machine.
 *
 * Pass the same `setActivityView` returned by `useAppRouting` here; the hook only
 * memoises pure dispatchers so multiple consumers (shortcuts, command palette,
 * activity bar) never need to reimplement the same logic.
 */
export function useActivityViewControls(
  setActivityView: React.Dispatch<React.SetStateAction<ActivityView>>
): ActivityViewControls {
  const toggleView = useCallback(
    (view: NonNullable<ActivityView>) =>
      setActivityView((v) => (v === view ? null : view)),
    [setActivityView]
  );

  const closeView = useCallback(() => setActivityView(null), [setActivityView]);

  const toggleBottomTerminal = useCallback(() => {
    setActivityView(null);
    const s = useStore.getState();
    const currentTabType = s.activeSessionId
      ? (s.sessions[s.activeSessionId]?.tabType ?? "terminal")
      : "terminal";
    if (currentTabType !== "terminal") {
      const termTab = s.tabOrder.find(
        (id) => (s.sessions[id]?.tabType ?? "terminal") === "terminal"
      );
      if (termTab) {
        s.setActiveSession(termTab);
        return;
      }
    }
    useStore.getState().toggleBottomTerminal();
  }, [setActivityView]);

  const focusAiChat = useCallback(() => {
    setActivityView(null);
    requestAnimationFrame(() => {
      document.querySelector<HTMLTextAreaElement>("[data-ai-chat-input]")?.focus();
    });
  }, [setActivityView]);

  return useMemo(
    () => ({ toggleView, closeView, toggleBottomTerminal, focusAiChat }),
    [toggleView, closeView, toggleBottomTerminal, focusAiChat]
  );
}
