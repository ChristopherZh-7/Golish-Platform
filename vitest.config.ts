import path from "node:path";
import react from "@vitejs/plugin-react-swc";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    globals: true,
    environment: "happy-dom",
    setupFiles: ["./frontend/test/setup.ts"],
    include: ["frontend/**/*.{test,spec}.{js,ts,jsx,tsx}"],
    coverage: {
      provider: "v8",
      reporter: ["text", "json", "html"],
      include: ["frontend/**/*.{ts,tsx}"],
      exclude: ["frontend/test/**", "frontend/**/*.d.ts"],
    },
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./frontend"),
      // Mock Tauri APIs. The alias for `@tauri-apps/api/event` swaps
      // the real Tauri module for our in-memory helper so tests don't
      // try to call the IPC bridge that doesn't exist in jsdom /
      // happy-dom. The helper file must therefore expose every named
      // export the production code imports from the original module
      // (`listen`, `emit`, `once`, `TauriEvent`, …).
      "@tauri-apps/api/event": path.resolve(
        __dirname,
        "./frontend/test/mocks/event-bus-helpers.ts"
      ),
      "@tauri-apps/api/core": path.resolve(__dirname, "./frontend/test/mocks/tauri-core.ts"),
    },
  },
});
