import { useCallback, useEffect, useState } from "react";
import { listIndexedCodebases } from "@/lib/indexer";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import {
  type CodebaseConfig,
  getSettings,
  type GolishSettings,
  updateSettings,
} from "@/lib/settings";
import { updateConfig as updatePentestConfig } from "@/lib/pentest/api";

export type SettingsSection =
  | "providers"
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

export function useSettingsNavigation(initialSection?: string) {
  const [settings, setSettings] = useState<GolishSettings | null>(null);
  const [activeSection, setActiveSection] = useState<SettingsSection>(
    (initialSection as SettingsSection) || "pentest"
  );
  const [isLoading, setIsLoading] = useState(false);

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

  const saveSettings = useCallback(async (settingsToSave: GolishSettings) => {
    try {
      const currentCodebases = await listIndexedCodebases();
      const updatedCodebases: CodebaseConfig[] = currentCodebases.map((cb) => ({
        path: cb.path,
        memory_file: cb.memory_file,
      }));
      const finalSettings = { ...settingsToSave, codebases: updatedCodebases };
      await updateSettings(finalSettings);
      window.dispatchEvent(new CustomEvent("settings-updated", { detail: finalSettings }));
    } catch (err) {
      logger.error("Failed to save settings:", err);
      notify.error("Failed to save settings");
    }
  }, []);

  const updateSection = useCallback(
    <K extends keyof GolishSettings>(section: K, value: GolishSettings[K]) => {
      setSettings((prev) => {
        if (!prev) return null;
        const updated = { ...prev, [section]: value };
        saveSettings(updated);
        return updated;
      });
    },
    [saveSettings]
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
