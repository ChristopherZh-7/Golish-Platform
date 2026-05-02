/**
 * Settings Service — domain-level API for application settings.
 *
 * Wraps transport calls with TTL-based caching and automatic
 * cache invalidation on writes.
 *
 * Components should import from here instead of calling invoke() directly.
 */

import type { GolishSettings, ProviderVisibility, TelemetryStats } from "../settings/types";
import { dedupInvoke, invoke } from "../transport";

const CACHE_TTL_MS = 5_000;

let cache: { data: GolishSettings; ts: number } | null = null;

export function invalidateCache(): void {
  cache = null;
}

export async function getSettings(): Promise<GolishSettings> {
  return dedupInvoke<GolishSettings>("get_settings");
}

export async function getSettingsCached(): Promise<GolishSettings> {
  if (cache && Date.now() - cache.ts < CACHE_TTL_MS) return cache.data;
  const data = await getSettings();
  cache = { data, ts: Date.now() };
  return data;
}

export async function updateSettings(settings: GolishSettings): Promise<void> {
  invalidateCache();
  await invoke("update_settings", { settings });
}

export async function getSetting<T = unknown>(key: string): Promise<T> {
  return invoke("get_setting", { key });
}

export async function setSetting(key: string, value: unknown): Promise<void> {
  invalidateCache();
  await invoke("set_setting", { key, value });
}

export async function resetSettings(): Promise<void> {
  invalidateCache();
  await invoke("reset_settings");
}

export async function reloadSettings(): Promise<void> {
  invalidateCache();
  await invoke("reload_settings");
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
