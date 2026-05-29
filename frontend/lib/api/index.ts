/**
 * Unified API Client — single entry point for all backend communication.
 *
 * Import from `@/lib/api` instead of calling `invoke()` directly.
 *
 * Example:
 *   import { api } from "@/lib/api";
 *   const session = await api.pty.ptyCreate("/path");
 *
 * Or import individual functions:
 *   import { ptyCreate } from "@/lib/api/pty";
 */

export { ApiError, getInflightCommands, invoke } from "./client";

import * as ai from "./ai";
import * as assetIntel from "./asset-intel";
import * as auditLog from "./audit-log";
import * as context from "./context";
import * as conversationDb from "./conversation-db";
import * as fileEditor from "./file-editor";
import * as files from "./files";
import * as findings from "./findings";
import * as history from "./history";
import * as indexer from "./indexer";
import * as integrations from "./integrations";
import * as intel from "./intel";
import * as mcp from "./mcp";
import * as methodology from "./methodology";
import * as modelRegistry from "./model-registry";
import * as notes from "./notes";
import * as organizations from "./organizations";
import * as outputParser from "./output-parser";
import * as pentestBrowser from "./pentest-browser";
import * as pipeline from "./pipeline";
import * as projects from "./projects";
import * as pty from "./pty";
import * as security from "./security";
import * as securityAnalysis from "./security-analysis";
import * as settings from "./settings";
import * as shell from "./shell";
import * as sidecar from "./sidecar";
import * as targets from "./targets";
import * as vault from "./vault";
import * as vulnIntel from "./vuln-intel";
import * as detachedWindow from "./window";
import * as wordlist from "./wordlist";

export {
  pty,
  assetIntel,
  shell,
  files,
  fileEditor,
  ai,
  settings,
  mcp,
  context,
  indexer,
  integrations,
  intel,
  sidecar,
  security,
  securityAnalysis,
  vulnIntel,
  conversationDb,
  findings,
  history,
  auditLog,
  modelRegistry,
  pentestBrowser,
  pipeline,
  projects,
  outputParser,
  detachedWindow,
  wordlist,
  notes,
  organizations,
  methodology,
  targets,
  vault,
};

export const api = {
  pty,
  assetIntel,
  shell,
  files,
  fileEditor,
  ai,
  settings,
  mcp,
  context,
  indexer,
  integrations,
  intel,
  sidecar,
  security,
  securityAnalysis,
  vulnIntel,
  conversationDb,
  findings,
  history,
  auditLog,
  modelRegistry,
  pentestBrowser,
  pipeline,
  projects,
  outputParser,
  detachedWindow,
  wordlist,
  notes,
  organizations,
  methodology,
  targets,
  vault,
} as const;
