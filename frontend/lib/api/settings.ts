/**
 * Settings API — re-exports existing settings module through the unified client.
 *
 * This file re-exports from lib/settings/api.ts so callers can import from
 * either `@/lib/api/settings` or `@/lib/settings` — both work.
 */
export {
  getSettings,
  updateSettings,
  getSetting,
  setSetting,
  resetSettings,
  reloadSettings,
  settingsFileExists,
  getSettingsPath,
  isLangfuseActive,
  getTelemetryStats,
  getSettingsCached,
  invalidateSettingsCache,
  buildProviderVisibility,
  SETTINGS_CACHE_TTL_MS,
} from "../settings/api";
