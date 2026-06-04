import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react-swc";
import { defineConfig } from "vite";
import type { PluginOption } from "vite";

// @ts-expect-error process is a nodejs global
// Use 127.0.0.1 explicitly to avoid IPv6 localhost issues in Node 18+
const host = process.env.TAURI_DEV_HOST || "127.0.0.1";

// Injects the standalone React DevTools bridge script in development mode.
// Run `pnpm devtools` in a separate terminal to open the DevTools UI,
// then start the Tauri dev server. The app will connect automatically.
const enableDevTools = process.env.REACT_DEVTOOLS === "1";

const reactDevToolsPlugin = (): PluginOption => ({
  name: "react-devtools",
  apply: "serve",
  transformIndexHtml(html) {
    if (!enableDevTools) return html;
    return html.replace(
      "<head>",
      '<head><script src="http://localhost:8097"></script>',
    );
  },
});

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [
    react(),
    tailwindcss(),
    reactDevToolsPlugin(),
  ],

  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./frontend"),
      "@codemirror/state": path.resolve(__dirname, "node_modules/@codemirror/state"),
      "@codemirror/view": path.resolve(__dirname, "node_modules/@codemirror/view"),
      "@codemirror/language": path.resolve(__dirname, "node_modules/@codemirror/language"),
    },
    dedupe: [
      "react",
      "react-dom",
      "@codemirror/state",
      "@codemirror/view",
      "@codemirror/language",
    ],
  },

  // Pre-bundle the heavy dependencies up front so the dev server doesn't
  // transform their (thousands of) internal modules on demand during the first
  // cold load — and, more importantly, so Vite doesn't *discover* a lazily
  // imported heavy dep mid-session and trigger a full-page reload (the classic
  // "long white screen, then it reloads" dev symptom). These are the libraries
  // that dominate the bundle: xterm, codemirror, milkdown, cytoscape, the
  // markdown/syntax-highlighter stack, and the command palette.
  optimizeDeps: {
    include: [
      "react",
      "react-dom",
      "react-dom/client",
      "zustand",
      "immer",
      "i18next",
      "react-i18next",
      "@xterm/xterm",
      "@xterm/addon-fit",
      "@xterm/addon-web-links",
      "@xterm/addon-webgl",
      "@xterm/addon-serialize",
      "react-markdown",
      "remark-gfm",
      "react-syntax-highlighter",
      "cytoscape",
      "@codemirror/state",
      "@codemirror/view",
      "@codemirror/language",
      "@uiw/react-codemirror",
      "@milkdown/crepe",
      "@milkdown/kit",
      "@milkdown/react",
      "cmdk",
    ],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `backend`
      ignored: ["**/backend/**"],
    },
  },

  // Build optimizations: manual chunk splitting for better caching
  // Each chunk loads independently, so unchanged vendor code stays cached
  build: {
    rollupOptions: {
      output: {
        manualChunks(id: string) {
          if (id.includes("node_modules")) {
            if (id.includes("react-dom") || id.includes("react/jsx-runtime") || id.match(/\/react\//)) {
              return "react-vendor";
            }
            if (id.includes("zustand") || id.includes("immer")) {
              return "state";
            }
            if (id.includes("@xterm/")) {
              return "xterm";
            }
            if (id.includes("react-markdown") || id.includes("react-syntax-highlighter") || id.includes("remark-gfm")) {
              return "markdown";
            }
            if (id.includes("@radix-ui/")) {
              return "radix";
            }
            if (id.includes("@codemirror/")) {
              return "codemirror";
            }
            if (id.includes("cytoscape")) {
              return "cytoscape";
            }
            if (id.includes("@milkdown/")) {
              return "milkdown";
            }
            if (id.includes("cmdk")) {
              return "cmdk";
            }
          }
        },
      },
    },
  },
}));
