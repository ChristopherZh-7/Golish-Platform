/**
 * Unified API Client — single entry point for all backend communication.
 *
 * Import from `@/lib/api` instead of calling `invoke()` directly.
 *
 * Example:
 *   import { api } from "@/lib/api";
 *   const branch = await api.git.getGitBranch("/path");
 *
 * Or import individual functions:
 *   import { getGitBranch } from "@/lib/api/git";
 */

export { ApiError, invoke, listen, getInflightCommands } from "./client";

import * as pty from "./pty";
import * as git from "./git";
import * as shell from "./shell";
import * as files from "./files";
import * as ai from "./ai";
import * as settings from "./settings";
import * as mcp from "./mcp";
import * as context from "./context";

export { pty, git, shell, files, ai, settings, mcp, context };

export const api = { pty, git, shell, files, ai, settings, mcp, context } as const;
