import type { GolishTheme } from "../../types";

const palette = {
  bgPrimary: "#070d15",
  bgSecondary: "#0c1520",
  bgTertiary: "#132030",
  bgHover: "#1a2940",

  textPrimary: "#dce8f0",
  textSecondary: "#7a9ab5",
  textMuted: "#4a6a85",

  accent: "#4dd0e1",
  accentDark: "#070d15",

  success: "#26c6a0",
  destructive: "#ef6c6c",

  borderSubtle: "rgba(77, 208, 225, 0.08)",
  borderMedium: "rgba(77, 208, 225, 0.14)",

  ring: "#4a6a85",

  chartPurple: "#7c4dff",
  chartGreen: "#26c6a0",
  chartYellow: "#ffca28",
  chartMagenta: "#ec407a",
  chartOrange: "#ff7043",

  ansiBlack: "#132030",
  ansiBlue: "#5c8abf",
  ansiBrightBlack: "#4a6a85",
  ansiBrightBlue: "#82b1ff",
  ansiBrightCyan: "#80deea",
  ansiBrightGreen: "#69f0ae",
  ansiBrightMagenta: "#b388ff",
  ansiBrightRed: "#ff8a80",
  ansiBrightWhite: "#eceff1",
  ansiBrightYellow: "#ffe57f",
  ansiCyan: "#4dd0e1",
  ansiDefaultBg: "#070d15",
  ansiDefaultFg: "#dce8f0",
  ansiGreen: "#26c6a0",
  ansiMagenta: "#9575cd",
  ansiRed: "#ef6c6c",
  ansiWhite: "#b0bec5",
  ansiYellow: "#ffd54f",
};

export const deepOceanTheme: GolishTheme = {
  author: "Golish Team",
  license: "MIT",
  name: "Deep Ocean",
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
