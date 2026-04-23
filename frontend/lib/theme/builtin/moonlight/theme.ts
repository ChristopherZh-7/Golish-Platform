import type { GolishTheme } from "../../types";

const palette = {
  bgPrimary: "#0e0e18",
  bgSecondary: "#141422",
  bgTertiary: "#1c1c30",
  bgHover: "#24243c",

  textPrimary: "#e4e4f0",
  textSecondary: "#8888b0",
  textMuted: "#555578",

  accent: "#a98eff",
  accentDark: "#0e0e18",

  success: "#86e1a0",
  destructive: "#f77a8a",

  borderSubtle: "rgba(169, 142, 255, 0.08)",
  borderMedium: "rgba(169, 142, 255, 0.14)",

  ring: "#5555778",

  chartPurple: "#a98eff",
  chartGreen: "#86e1a0",
  chartYellow: "#f0d070",
  chartMagenta: "#f77a8a",
  chartOrange: "#f0a060",

  ansiBlack: "#1c1c30",
  ansiBlue: "#7090d0",
  ansiBrightBlack: "#5555788",
  ansiBrightBlue: "#90b0f0",
  ansiBrightCyan: "#80d8e8",
  ansiBrightGreen: "#a0f0b8",
  ansiBrightMagenta: "#c8a0ff",
  ansiBrightRed: "#ffa0a8",
  ansiBrightWhite: "#f0f0f8",
  ansiBrightYellow: "#f8e088",
  ansiCyan: "#60c0d0",
  ansiDefaultBg: "#0e0e18",
  ansiDefaultFg: "#e4e4f0",
  ansiGreen: "#86e1a0",
  ansiMagenta: "#b090e0",
  ansiRed: "#f77a8a",
  ansiWhite: "#c8c8d8",
  ansiYellow: "#f0d070",
};

export const moonlightTheme: GolishTheme = {
  author: "Golish Team",
  license: "MIT",
  name: "Moonlight",
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
    base: "0.5rem",
  },

  terminal: {
    cursorBlink: true,
    cursorStyle: "block",
    selectionBackground: palette.bgTertiary,
  },

  typography: {
    terminal: {
      fontFamily: "SF Mono, Menlo, Monaco, 'JetBrains Mono', monospace",
      fontSize: 14,
    },
    ui: {
      fontFamily:
        "'Source Sans 3', Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
    },
  },
};
