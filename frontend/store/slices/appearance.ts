/**
 * Appearance slice for the Zustand store.
 *
 * Manages display settings — per-element visibility toggles that let users
 * customise which UI chrome is shown (tab bar buttons, context bar, status bar, etc.).
 */

import type { SliceCreator } from "./types";

const DISPLAY_SETTINGS_KEY = "golish-display-settings-v4";

/**
 * Controls which UI elements are visible.
 * `true` = shown, `false` = hidden.
 *
 * Only flags backed by an actual UI consumer are kept here — defunct toggles
 * (file editor / history / settings / notification-bell buttons, status bar
 * row, model badge, git branch badge, parent `showTabBar` / `showStatusBar`)
 * were removed in 2026-05 because the underlying chrome has either moved
 * (model badge → AI Chat Panel) or been retired (status bar row).
 */
export interface DisplaySettings {
  /** Show the home tab in the tab bar. */
  showHomeTab: boolean;
  /** Show the terminal context bar (path + virtual env). */
  showTerminalContext: boolean;
  /** Show the working directory path badge in the context bar. */
  showWorkingDirectory: boolean;
  /** Show the full input mode toggle (Terminal / AI) instead of collapsing to a single icon. */
  showInputModeToggle: boolean;
  /** Show the context / token usage percentage badge. */
  showContextUsage: boolean;
  /** Show the MCP servers indicator badge. */
  showMcpBadge: boolean;
  /** Hide AI-specific status bar items (token usage, MCP) when in shell mode. */
  hideAiSettingsInShellMode: boolean;
  /** Global UI scale factor (0.75 – 1.5). Applied via CSS zoom on the app root. */
  uiScale: number;
}

export const defaultDisplaySettings: DisplaySettings = {
  showHomeTab: true,
  showTerminalContext: true,
  showWorkingDirectory: true,
  showInputModeToggle: true,
  showContextUsage: true,
  showMcpBadge: true,
  hideAiSettingsInShellMode: false,
  uiScale: 1.1,
};

function loadDisplaySettings(): DisplaySettings {
  try {
    const stored = localStorage.getItem(DISPLAY_SETTINGS_KEY);
    if (stored) {
      // Spread default first so unknown legacy keys are dropped silently.
      const parsed = JSON.parse(stored) as Partial<DisplaySettings>;
      return { ...defaultDisplaySettings, ...parsed };
    }
  } catch {
    // ignore parse errors
  }
  return defaultDisplaySettings;
}

function saveDisplaySettings(settings: DisplaySettings): void {
  try {
    localStorage.setItem(DISPLAY_SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    // ignore storage errors
  }
}

// State interface
export interface AppearanceState {
  displaySettings: DisplaySettings;
}

// Actions interface
export interface AppearanceActions {
  setDisplaySettings: (settings: DisplaySettings) => void;
}

// Combined slice interface
export interface AppearanceSlice extends AppearanceState, AppearanceActions {}

// Initial state
export const initialAppearanceState: AppearanceState = {
  displaySettings: loadDisplaySettings(),
};

/**
 * Creates the appearance slice.
 * Display settings are global (not per-session).
 */
export const createAppearanceSlice: SliceCreator<AppearanceSlice> = (set) => ({
  ...initialAppearanceState,

  setDisplaySettings: (settings) =>
    set((state) => {
      state.displaySettings = settings;
      saveDisplaySettings(settings);
    }),
});

// Selectors
export const selectDisplaySettings = <T extends AppearanceState>(state: T): DisplaySettings =>
  state.displaySettings;
