import { useEffect } from "react";
import { logger } from "@/lib/logger";
import { initSystemNotifications, listenForSettingsUpdates } from "../../lib/systemNotifications";
import { useStore } from "../../store";

/**
 * Initialize app-level window lifecycle listeners and keep focus/visibility
 * flags in the global store in sync with browser events.
 */
export function useWindowLifecycle(): void {
  useEffect(() => {
    const { setAppIsFocused, setAppIsVisible } = useStore.getState();

    initSystemNotifications(useStore).catch((error) => {
      logger.error("Failed to initialize system notifications:", error);
    });

    const unlistenSettings = listenForSettingsUpdates();

    const handleFocus = () => setAppIsFocused(true);
    const handleBlur = () => setAppIsFocused(false);
    const handleVisibilityChange = () => {
      setAppIsVisible(document.visibilityState === "visible");
    };

    window.addEventListener("focus", handleFocus);
    window.addEventListener("blur", handleBlur);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    setAppIsFocused(document.hasFocus());
    setAppIsVisible(document.visibilityState === "visible");

    return () => {
      unlistenSettings();
      window.removeEventListener("focus", handleFocus);
      window.removeEventListener("blur", handleBlur);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, []);
}
