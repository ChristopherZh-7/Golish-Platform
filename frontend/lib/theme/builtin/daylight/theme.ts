import type { GolishTheme } from "../../types";

const palette = {
  bgPrimary: "#f8f9fb",
  bgSecondary: "#ffffff",
  bgTertiary: "#f0f2f5",
  bgHover: "#e8ebf0",

  textPrimary: "#1a1c20",
  textSecondary: "#5c6370",
  textMuted: "#9ca3af",

  accent: "#3b82f6",
  accentDark: "#ffffff",

  success: "#22c55e",
  destructive: "#ef4444",

  borderSubtle: "rgba(0, 0, 0, 0.08)",
  borderMedium: "rgba(0, 0, 0, 0.14)",

  ring: "#9ca3af",

  chartPurple: "#8b5cf6",
  chartGreen: "#22c55e",
  chartYellow: "#eab308",
  chartMagenta: "#ec4899",
  chartOrange: "#f97316",

  ansiBlack: "#1a1c20",
  ansiBlue: "#2563eb",
  ansiBrightBlack: "#6b7280",
  ansiBrightBlue: "#3b82f6",
  ansiBrightCyan: "#06b6d4",
  ansiBrightGreen: "#22c55e",
  ansiBrightMagenta: "#a855f7",
  ansiBrightRed: "#ef4444",
  ansiBrightWhite: "#f9fafb",
  ansiBrightYellow: "#eab308",
  ansiCyan: "#0891b2",
  ansiDefaultBg: "#ffffff",
  ansiDefaultFg: "#1a1c20",
  ansiGreen: "#16a34a",
  ansiMagenta: "#9333ea",
  ansiRed: "#dc2626",
  ansiWhite: "#e5e7eb",
  ansiYellow: "#ca8a04",
};

export const daylightTheme: GolishTheme = {
  author: "Golish Team",
  license: "MIT",
  name: "Daylight",
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
    cursorStyle: "bar",
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
