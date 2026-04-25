import { invoke } from "@tauri-apps/api/core";
import type { GolishSettings, ProviderVisibility, TelemetryStats } from "./types";

// ── Settings Cache ──────────────────────

export const SETTINGS_CACHE_TTL_MS = 5000;

let settingsCache: GolishSettings | null = null;
let settingsCacheTime = 0;

export async function getSettingsCached(): Promise<GolishSettings> {
  const now = Date.now();
  if (settingsCache && now - settingsCacheTime < SETTINGS_CACHE_TTL_MS) {
    return settingsCache;
  }
  settingsCache = await getSettings();
  settingsCacheTime = now;
  return settingsCache;
}

export function invalidateSettingsCache(): void {
  settingsCache = null;
  settingsCacheTime = 0;
}

// ── API Functions ──────────────────────

export async function getSettings(): Promise<GolishSettings> {
  return invoke("get_settings");
}

export async function updateSettings(settings: GolishSettings): Promise<void> {
  await invoke("update_settings", { settings });
  invalidateSettingsCache();
}

export async function getSetting<T = unknown>(key: string): Promise<T> {
  return invoke("get_setting", { key });
}

export async function setSetting(key: string, value: unknown): Promise<void> {
  await invoke("set_setting", { key, value });
  invalidateSettingsCache();
}

export async function resetSettings(): Promise<void> {
  await invoke("reset_settings");
  invalidateSettingsCache();
}

export async function reloadSettings(): Promise<void> {
  await invoke("reload_settings");
  invalidateSettingsCache();
}

export async function settingsFileExists(): Promise<boolean> {
  return invoke("settings_file_exists");
}

export async function getSettingsPath(): Promise<string> {
  return invoke("get_settings_path");
}

export async function isLangfuseActive(): Promise<boolean> {
  return invoke("is_langfuse_active");
}

export async function getTelemetryStats(): Promise<TelemetryStats | null> {
  return invoke("get_telemetry_stats");
}

// ── Provider Visibility ──────────────────────

export function buildProviderVisibility(settings: GolishSettings): ProviderVisibility {
  return {
    vertex_ai: settings.ai.vertex_ai.show_in_selector,
    vertex_gemini: settings.ai.vertex_gemini?.show_in_selector ?? true,
    openrouter: settings.ai.openrouter.show_in_selector,
    openai: settings.ai.openai.show_in_selector,
    anthropic: settings.ai.anthropic.show_in_selector,
    ollama: settings.ai.ollama.show_in_selector,
    gemini: settings.ai.gemini.show_in_selector,
    groq: settings.ai.groq.show_in_selector,
    xai: settings.ai.xai.show_in_selector,
    zai_sdk: settings.ai.zai_sdk?.show_in_selector ?? true,
    nvidia: settings.ai.nvidia?.show_in_selector ?? true,
  };
}
