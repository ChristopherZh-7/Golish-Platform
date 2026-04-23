import type { GolishTheme } from "../../types";

const palette = {
  bgPrimary: "#0a0a0f",
  bgSecondary: "#0f0f18",
  bgTertiary: "#161625",
  bgHover: "#1c1c30",

  textPrimary: "#e0e0e8",
  textSecondary: "#8888a8",
  textMuted: "#555570",

  accent: "#39ff14",
  accentDark: "#0a0a0f",

  secondary: "#b24bf3",

  success: "#39ff14",
  destructive: "#ff2d5b",

  borderSubtle: "rgba(57, 255, 20, 0.08)",
  borderMedium: "rgba(57, 255, 20, 0.15)",

  ring: "#555570",

  chartPurple: "#b24bf3",
  chartGreen: "#39ff14",
  chartYellow: "#ffe744",
  chartMagenta: "#ff2d5b",
  chartOrange: "#ff8c00",

  ansiBlack: "#1a1a2e",
  ansiBlue: "#4d6dff",
  ansiBrightBlack: "#555570",
  ansiBrightBlue: "#7b93ff",
  ansiBrightCyan: "#00ffff",
  ansiBrightGreen: "#7dff6a",
  ansiBrightMagenta: "#d580ff",
  ansiBrightRed: "#ff6688",
  ansiBrightWhite: "#f0f0f8",
  ansiBrightYellow: "#ffee80",
  ansiCyan: "#00e5e5",
  ansiDefaultBg: "#0a0a0f",
  ansiDefaultFg: "#e0e0e8",
  ansiGreen: "#39ff14",
  ansiMagenta: "#b24bf3",
  ansiRed: "#ff2d5b",
  ansiWhite: "#d0d0dd",
  ansiYellow: "#ffe744",
};

export const cyberpunkTheme: GolishTheme = {
  author: "Golish Team",
  license: "MIT",
  name: "Cyberpunk",
  schemaVersion: "1.0.0",
  version: "1.0.0",

  colors: {
    ansi: {
      black: palette.ansiBlack,
      blue: palette.ansiBlue,
      brightBlack: palette.ansiBrightBlack,
      brightBlue: palette.ansiBrightBlue,
      brightCyan: palette.ansiBrightCyan,
      brightGreen: palette.ansiBrightGreen,
      brightMagenta: palette.ansiBrightMagenta,
      brightRed: palette.ansiBrightRed,
      brightWhite: palette.ansiBrightWhite,
      brightYellow: palette.ansiBrightYellow,
      cyan: palette.ansiCyan,
      defaultBg: palette.ansiDefaultBg,
      defaultFg: palette.ansiDefaultFg,
      green: palette.ansiGreen,
      magenta: palette.ansiMagenta,
      red: palette.ansiRed,
      white: palette.ansiWhite,
      yellow: palette.ansiYellow,
    },

    ui: {
      accent: palette.accent,
      accentForeground: palette.accentDark,
      background: palette.bgPrimary,
      border: palette.borderSubtle,
      card: palette.bgSecondary,
      cardForeground: palette.textPrimary,

      chart: {
        c1: palette.chartPurple,
        c2: palette.chartGreen,
        c3: palette.chartYellow,
        c4: palette.chartMagenta,
        c5: palette.chartOrange,
      },

      destructive: palette.destructive,
      foreground: palette.textPrimary,
      input: palette.borderMedium,
      muted: palette.bgTertiary,
      mutedForeground: palette.textSecondary,
      popover: palette.bgSecondary,
      popoverForeground: palette.textPrimary,
      primary: palette.accent,
      primaryForeground: palette.accentDark,
      ring: palette.ring,
      secondary: palette.bgTertiary,
      secondaryForeground: palette.textPrimary,
      sidebar: palette.bgSecondary,
      sidebarAccent: palette.bgTertiary,
      sidebarAccentForeground: palette.textPrimary,
      sidebarBorder: palette.borderSubtle,
      sidebarForeground: palette.textPrimary,
      sidebarPrimary: palette.accent,
      sidebarPrimaryForeground: palette.accentDark,
      sidebarRing: palette.ring,
    },
  },

  effects: {
    plugins: [],
  },

  radii: {
    base: "0.375rem",
  },

  terminal: {
    cursorBlink: true,
    cursorStyle: "block",
    selectionBackground: palette.bgTertiary,
  },

  typography: {
    terminal: {
      fontFamily: "'JetBrains Mono', 'Fira Code', SF Mono, Menlo, monospace",
      fontSize: 14,
    },
    ui: {
      fontFamily:
        "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
    },
  },
};
