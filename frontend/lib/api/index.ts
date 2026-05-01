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

export { ApiError, getInflightCommands, invoke, listen } from "./client";

import * as ai from "./ai";
import * as context from "./context";
import * as files from "./files";
import * as git from "./git";
import * as mcp from "./mcp";
import * as methodology from "./methodology";
import * as notes from "./notes";
import * as pty from "./pty";
import * as research from "./research";
import * as settings from "./settings";
import * as shell from "./shell";
import * as targets from "./targets";
import * as vault from "./vault";
import * as vulnLinks from "./vuln-links";
import * as wordlist from "./wordlist";

export {
  pty,
  git,
  shell,
  files,
  ai,
  settings,
  mcp,
  context,
  wordlist,
  notes,
  methodology,
  targets,
  vault,
  vulnLinks,
  research,
};

export const api = {
  pty,
  git,
  shell,
  files,
  ai,
  settings,
  mcp,
  context,
  wordlist,
  notes,
  methodology,
  targets,
  vault,
  vulnLinks,
  research,
} as const;
