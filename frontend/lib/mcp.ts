/**
 * Compat re-export — actual implementation lives at `@/lib/api/mcp`.
 *
 * This file is kept for backward compatibility with existing imports
 * (e.g. `import { listServers } from "@/lib/mcp"`). New code should
 * import from `@/lib/api/mcp` or use `api.mcp.*` from `@/lib/api`.
 *
 * See `docs/development.md#adding-a-new-tauri-command-ipc`.
 */

export * from "@/lib/api/mcp";
