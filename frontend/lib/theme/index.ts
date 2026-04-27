/**
 * Theme system exports
 * Centralized exports for the theme module
 */

export {
  deleteTheme,
  getThemeAssetPath,
  listThemes,
  readTheme,
  saveTheme,
  saveThemeAsset,
  type ThemeInfo,
} from "./api";
export { ThemeRegistry } from "./registry";
export { applyTheme, loadThemeFromFile, loadThemeFromUrl } from "./ThemeLoader";
export { ThemeManager } from "./ThemeManager";
export type {
  AnsiColors,
  CursorEffect,
  CursorStyle,
  GolishTheme,
  GolishThemeMetadata,
  TerminalSettings,
  TerminalTypography,
  ThemeColors,
  ThemeEffects,
  ThemePlugin,
  ThemeRadii,
  ThemeRegistryEntry,
  ThemeTypography,
  UIColors,
  UITypography,
} from "./types";
