import type { GolishTheme } from "../../types";

const palette = {
  bgPrimary: "#0e0a0a",
  bgSecondary: "#161010",
  bgTertiary: "#201818",
  bgHover: "#2a1f1f",

  textPrimary: "#e8e0e0",
  textSecondary: "#a09090",
  textMuted: "#685858",

  accent: "#e05555",
  accentDark: "#0e0a0a",

  success: "#5cb870",
  destructive: "#ff4444",

  borderSubtle: "rgba(224, 85, 85, 0.08)",
  borderMedium: "rgba(224, 85, 85, 0.14)",

  ring: "#685858",

  chartPurple: "#b060c0",
  chartGreen: "#5cb870",
  chartYellow: "#e0a840",
  chartMagenta: "#d04080",
  chartOrange: "#e07030",

  ansiBlack: "#201818",
  ansiBlue: "#6080b0",
  ansiBrightBlack: "#685858",
  ansiBrightBlue: "#80a0d0",
  ansiBrightCyan: "#70c8c0",
  ansiBrightGreen: "#80d090",
  ansiBrightMagenta: "#d090c0",
  ansiBrightRed: "#ff7070",
  ansiBrightWhite: "#f0e8e8",
  ansiBrightYellow: "#f0c868",
  ansiCyan: "#50b0a8",
  ansiDefaultBg: "#0e0a0a",
  ansiDefaultFg: "#e8e0e0",
  ansiGreen: "#5cb870",
  ansiMagenta: "#b070a0",
  ansiRed: "#e05555",
  ansiWhite: "#c8b8b8",
  ansiYellow: "#d0a030",
};

export const redAlertTheme: GolishTheme = {
  author: "Golish Team",
  license: "MIT",
  name: "Red Alert",
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
      fontFamily: "SF Mono, Menlo, Monaco, 'JetBrains Mono', monospace",
      fontSize: 14,
    },
    ui: {
      fontFamily:
        "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
    },
  },
};
