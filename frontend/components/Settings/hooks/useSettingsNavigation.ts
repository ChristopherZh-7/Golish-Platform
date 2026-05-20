import { useCallback, useEffect, useRef, useState } from "react";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import { updateConfig as updatePentestConfig } from "@/lib/pentest/api";
import { type GolishSettings, getSettings, updateSettings } from "@/lib/settings";

export type SettingsSection =
  | "providers"
  | "intel"
  | "ai"
  | "terminal"
  | "editor"
  | "agent"
  | "mcp"
  | "codebases"
  | "network"
  | "notifications"
  | "appearance"
  | "advanced"
  | "pentest"
  | "vault";

/**
 * Debounce window between user-driven setting tweaks and the actual
 * `updateSettings` IPC. Coalescing rapid edits (e.g. dragging a slider,
 * toggling several checkboxes in succession) keeps the Settings UI
 * responsive and avoids stacking dozens of redundant Tauri command calls.
 */
const SAVE_DEBOUNCE_MS = 300;

export function useSettingsNavigation(initialSection?: string) {
  const [settings, setSettings] = useState<GolishSettings | null>(null);
  const [activeSection, setActiveSection] = useState<SettingsSection>(
    (initialSection as SettingsSection) || "pentest"
  );
  const [isLoading, setIsLoading] = useState(false);

  // Hold the latest pending settings so the debounced flush always writes
  // the most recent value, regardless of how many rapid edits happened.
  const pendingRef = useRef<GolishSettings | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (initialSection) {
      setActiveSection(initialSection as SettingsSection);
    }
  }, [initialSection]);

  const loadSettings = useCallback(() => {
    setIsLoading(true);
    getSettings()
      .then(setSettings)
      .catch((err) => {
        logger.error("Failed to load settings:", err);
        notify.error("Failed to load settings");
      })
      .finally(() => setIsLoading(false));
  }, []);

  const flushSave = useCallback(async () => {
    const snapshot = pendingRef.current;
    if (!snapshot) return;
    pendingRef.current = null;
    try {
      await updateSettings(snapshot);
      window.dispatchEvent(new CustomEvent("settings-updated", { detail: snapshot }));
    } catch (err) {
      logger.error("Failed to save settings:", err);
      notify.error("Failed to save settings");
    }
  }, []);

  const scheduleSave = useCallback(
    (next: GolishSettings) => {
      pendingRef.current = next;
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        void flushSave();
      }, SAVE_DEBOUNCE_MS);
    },
    [flushSave]
  );

  // On unmount, flush any pending change synchronously so we don't lose the
  // user's last edit when they close the dialog quickly.
  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
        void flushSave();
      }
    };
  }, [flushSave]);

  const updateSection = useCallback(
    <K extends keyof GolishSettings>(section: K, value: GolishSettings[K]) => {
      setSettings((prev) => {
        if (!prev) return null;
        const updated = { ...prev, [section]: value };
        scheduleSave(updated);
        return updated;
      });
    },
    [scheduleSave]
  );

  const handleNetworkChange = useCallback(
    (network: GolishSettings["network"]) => {
      updateSection("network", network);
      updatePentestConfig({
        proxy_url: network.proxy_url || "",
        github_token: network.github_token || "",
      }).catch((e) => console.error("[Settings] pentest config sync failed:", e));
    },
    [updateSection]
  );

  return {
    settings,
    activeSection,
    setActiveSection,
    isLoading,
    loadSettings,
    updateSection,
    handleNetworkChange,
  };
}
