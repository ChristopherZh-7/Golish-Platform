/* ── Types & constants for the project-overview activity feed ─────────
 *
 * Extracted from `ProjectOverview.tsx` to keep that file under the 800-line
 * budget enforced by `scripts/check_file_sizes.sh`. Anything that's purely
 * data — type aliases, lookup maps, step lists — lives here so the .tsx
 * files only own JSX.
 */

import type { Target as PentestTarget } from "@/lib/pentest/types";

declare global {
  interface Window {
    __PENDING_RECON__?: {
      sessionId: string;
      targets: string[];
      projectName: string;
      projectPath: string;
    };
  }
}

export interface TargetStore {
  targets: PentestTarget[];
  groups: string[];
}

export type ItemKind =
  | "tool_start"
  | "tool_done"
  | "tool_error"
  | "agent_thinking"
  | "agent_done"
  | "sub_agent_start"
  | "sub_agent_done"
  | "pipeline_start"
  | "pipeline_done"
  | "pipeline_error"
  | "info";

export interface ActivityItem {
  id: string;
  kind: ItemKind;
  label: string;
  detail?: string;
  ts: number;
  durationMs?: number;
}

export interface StepGroup {
  id: string;
  stepName: string;
  status: "pending" | "running" | "completed" | "failed";
  startTs: number;
  durationMs?: number;
  output?: string;
  children: ActivityItem[];
}

export type FeedEntry = { type: "item"; data: ActivityItem } | { type: "step"; data: StepGroup };

export interface PipelineProgress {
  status: "running" | "completed" | "failed";
  totalSteps: number;
  completedSteps: number;
  currentStepIndex: number;
  currentStepName: string;
  stepNames: string[];
}

/** Friendly labels for raw tool / step names emitted from the backend. */
export const TOOL_DISPLAY: Record<string, string> = {
  run_shell_command: "Shell",
  read_file: "Read File",
  write_file: "Write File",
  list_directory: "List Dir",
  web_search: "Web Search",
  web_fetch: "Web Fetch",
  nmap_scan: "Nmap",
  dns_lookup: "DNS Lookup",
  http_probe: "HTTP Probe",
  port_scan: "Port Scan",
  tech_fingerprint: "Fingerprint",
  whatweb: "WhatWeb",
  subfinder: "Subfinder",
  httpx: "HTTPX",
  initialize: "Initialize",
  tool_check: "Tool Check",
  tool_install: "Tool Install",
  brew_install_nmap: "Install nmap",
  brew_install_whatweb: "Install whatweb",
  brew_install_subfinder: "Install subfinder",
  brew_install_httpx: "Install httpx",
  summarize: "Summarize",
};

/** Per-step descriptive blurbs shown while a step is pending or running. */
export const STEP_DESCRIPTIONS: Record<string, string> = {
  initialize: "Setting up targets and preparing workspace",
  tool_check: "Checking which recon tools are installed",
  tool_install: "Installing missing tools via Homebrew",
  dns_lookup: "Resolving DNS records for targets",
  http_probe: "Probing HTTP/HTTPS services",
  port_scan: "Scanning for open ports (nmap)",
  tech_fingerprint: "Identifying web technologies",
  summarize: "Generating results summary",
};

/** Cap on the number of feed entries kept in memory. */
export const MAX_ENTRIES = 200;

/** Canonical recon-pipeline step order, used for the progress bar skeleton. */
export const RECON_STEPS = [
  "initialize",
  "tool_check",
  "tool_install",
  "dns_lookup",
  "http_probe",
  "port_scan",
  "tech_fingerprint",
  "summarize",
];
