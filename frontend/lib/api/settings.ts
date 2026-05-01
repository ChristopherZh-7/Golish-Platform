/**
 * Settings API — re-exports existing settings module through the unified client.
 *
 * This file re-exports from lib/settings/api.ts so callers can import from
 * either `@/lib/api/settings` or `@/lib/settings` — both work.
 */
export {
  buildProviderVisibility,
  getSetting,
  getSettings,
  getSettingsCached,
  getSettingsPath,
  getTelemetryStats,
  invalidateSettingsCache,
  isLangfuseActive,
  reloadSettings,
  resetSettings,
  SETTINGS_CACHE_TTL_MS,
  setSetting,
  settingsFileExists,
  updateSettings,
} from "../settings/api";
