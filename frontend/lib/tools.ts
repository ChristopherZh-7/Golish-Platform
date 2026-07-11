/**
 * Shared utilities for tool call display components.
 */

import type { LucideIcon } from "lucide-react";
import {
  FileCode,
  FileSearch,
  FolderOpen,
  Globe,
  Network,
  Pencil,
  Radar,
  Search,
  Terminal,
  Wrench,
} from "lucide-react";
import { stripAnsiForDisplay } from "./ansi";
import { safeStringify } from "./text";

/**
 * Risk level for a tool operation. Mirrors the canonical Rust enum
 * `golish-core/src/tools/risk.rs::RiskLevel` (via Tauri IPC). Defined
 * locally so the frontend does not depend on the generated `lib/generated/`
 * directory (which is being removed per M2.5).
 */
export type RiskLevel = "low" | "medium" | "high" | "critical";

const TOOL_COLORS: Record<string, string> = {
  run_command: "var(--ansi-green)",
  run_pty_cmd: "var(--ansi-green)",
  read_file: "var(--ansi-cyan)",
  write_file: "var(--ansi-yellow)",
  edit_file: "var(--ansi-yellow)",
  search_files: "var(--ansi-blue)",
  web_search: "var(--ansi-magenta)",
  web_fetch: "var(--ansi-magenta)",
  manage_targets: "var(--ansi-cyan)",
  record_finding: "#f59e0b",
  recon_map_assets: "var(--ansi-magenta)",
  recon_lookup_whois: "var(--ansi-magenta)",
  recon_discover_subsidiaries: "var(--ansi-magenta)",
  enum_crawl_same_origin_urls: "var(--ansi-magenta)",
  vuln_run_formulaic_sweep: "#f59e0b",
};

const TOOL_ICONS: Record<string, LucideIcon> = {
  run_command: Terminal,
  run_pty_cmd: Terminal,
  shell: Terminal,
  read_file: FileSearch,
  write_file: Pencil,
  edit_file: Pencil,
  apply_patch: FileCode,
  list_files: FolderOpen,
  search_files: Search,
  grep_file: Search,
  web_search: Globe,
  web_search_answer: Globe,
  web_fetch: Globe,
  manage_targets: Network,
  recon_map_assets: Radar,
  recon_lookup_whois: Globe,
  recon_discover_subsidiaries: Radar,
  enum_crawl_same_origin_urls: Globe,
  vuln_run_formulaic_sweep: Radar,
};

export function getToolColor(name: string): string {
  return TOOL_COLORS[name] || "var(--ansi-blue)";
}

export function getToolIcon(name: string): LucideIcon {
  return TOOL_ICONS[name] || Wrench;
}

/** Base properties shared by all tool call types */
interface BaseToolCall {
  name: string;
  executedByAgent?: boolean;
}

/** Check if a tool call is a terminal command executed by the agent */
export function isAgentTerminalCommand(tool: BaseToolCall): boolean {
  return (
    (tool.name === "run_pty_cmd" || tool.name === "run_command" || tool.name === "shell") &&
    tool.executedByAgent === true
  );
}

/** Check if a tool call is a visible terminal command (run_pty_cmd/run_command) */
export function isVisibleTerminalCommand(tool: BaseToolCall): boolean {
  return tool.name === "run_pty_cmd" || tool.name === "run_command";
}

/** Format tool name for display (e.g., "read_file" -> "Read File") */
export function formatToolName(name: string): string {
  return name
    .split(/[_\s-]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

const TOOL_LABELS_SHORT: Record<string, string> = {
  run_command: "Shell",
  run_pty_cmd: "Shell",
  wait_for_background_jobs: "Waiting",
  read_file: "Read",
  write_file: "Write",
  edit_file: "Edit",
  search_files: "Search",
  web_search: "Web",
  web_fetch: "Fetch",
  manage_targets: "Targets",
  record_finding: "Finding",
  recon_map_assets: "Survey",
  recon_lookup_whois: "WHOIS",
  recon_discover_subsidiaries: "Subsidiaries",
  eas_fingerprint_web_stack: "Web FP",
  enum_crawl_same_origin_urls: "Crawl URLs",
  vuln_run_formulaic_sweep: "Vuln Sweep",
};

const TOOL_LABELS_STANDARD: Record<string, string> = {
  run_command: "Shell Command",
  run_pty_cmd: "Shell Command",
  wait_for_background_jobs: "Waiting for background jobs",
  read_file: "Read File",
  write_file: "Write File",
  edit_file: "Edit File",
  search_files: "Search Files",
  web_search: "Web Search",
  web_fetch: "Fetch URL",
  manage_targets: "Manage Targets",
  record_finding: "Record Finding",
  credential_vault: "Credential Vault",
  pentest_run: "Pentest Run",
  pentest_list_tools: "List Tools",
  pentest_read_skill: "Read Skill",
  recon_map_assets: "Map Assets",
  recon_lookup_whois: "Lookup WHOIS",
  recon_discover_subsidiaries: "Discover Subsidiaries",
  eas_fingerprint_web_stack: "Fingerprint Web Stack",
  enum_crawl_same_origin_urls: "Crawl Same-Origin URLs",
  vuln_run_formulaic_sweep: "Formulaic Vuln Sweep",
};

export function getToolLabel(name: string, variant: "short" | "standard" = "standard"): string {
  const map = variant === "short" ? TOOL_LABELS_SHORT : TOOL_LABELS_STANDARD;
  return map[name] || formatToolName(name);
}

const TOOL_ACTION_LABELS: Record<string, string> = {
  run_command: "Running shell command",
  run_pty_cmd: "Running shell command",
  shell: "Running shell command",
  wait_for_background_jobs: "Waiting for background jobs",
  check_job: "Checking background job",
  kill_job: "Stopping background job",
  list_jobs: "Listing background jobs",
  stage_run: "Running specialist agents",
  submit_stage_deliverable: "Submitting stage deliverable",
  check_stage_asset_coverage: "Checking asset coverage",
  list_recent_evidence: "Reading recent evidence",
  list_in_scope_targets: "Reading in-scope targets",
  list_attack_surface_seeds: "Reading attack surface seeds",
  query_target_data: "Reading target data",
  manage_targets: "Updating targets",
  manage_organizations: "Updating organizations",
  pentest_list_tools: "Listing pentest tools",
  pentest_read_skill: "Reading tool skill",
  recon_map_assets: "Mapping assets",
  recon_lookup_whois: "Looking up WHOIS",
  recon_discover_subsidiaries: "Discovering subsidiaries",
  recon_enrich_assets: "Enriching assets",
  eas_fingerprint_web_stack: "Fingerprinting web services",
  enum_crawl_same_origin_urls: "Crawling same-origin URLs",
  vuln_run_formulaic_sweep: "Running formulaic vuln sweep",
  read_file: "Reading file",
  write_file: "Writing file",
  edit_file: "Editing file",
  search_files: "Searching files",
  grep_file: "Searching files",
  web_search: "Searching the web",
  web_search_answer: "Searching the web",
  web_fetch: "Fetching URL",
};

const EXTERNAL_TOOL_LABELS: Record<string, string> = {
  httpx: "HTTPX",
  naabu: "Naabu",
  nmap: "Nmap",
  masscan: "Masscan",
  whatweb: "WhatWeb",
  gowitness: "GoWitness",
  ffuf: "FFUF",
  feroxbuster: "Feroxbuster",
  gobuster: "Gobuster",
  nuclei: "Nuclei",
  dig: "dig",
  whois: "WHOIS",
};

function formatExternalToolName(name: string): string {
  const trimmed = name.trim();
  const known = EXTERNAL_TOOL_LABELS[trimmed.toLowerCase()];
  return known ?? formatToolName(trimmed);
}

export function getToolActionLabel(name: string, args?: Record<string, unknown>): string {
  if (name === "pentest_run") {
    return formatPentestRunActionLabel(args);
  }

  if (name.startsWith("sub_agent_")) {
    const agentName = formatToolName(name.replace(/^sub_agent_/, ""));
    return agentName ? `Starting ${agentName} agent` : "Starting sub-agent";
  }

  return TOOL_ACTION_LABELS[name] ?? `Using ${formatToolName(name)}`;
}

function formatPentestRunActionLabel(args?: Record<string, unknown>): string {
  const toolName = typeof args?.tool_name === "string" ? args.tool_name.trim().toLowerCase() : "";
  const rawArgs = typeof args?.args === "string" ? args.args.toLowerCase() : "";

  switch (toolName) {
    case "naabu":
    case "masscan":
      return "Scanning ports";
    case "nmap":
      if (/(^|\s)-sn(\s|$)/.test(rawArgs)) return "Checking host reachability";
      if (/(^|\s)-sv(\s|$)/.test(rawArgs) || rawArgs.includes("service")) {
        return "Probing services";
      }
      return "Running Nmap";
    case "httpx":
      return "Checking web services";
    case "whatweb":
      return "Fingerprinting web services";
    case "gowitness":
      return "Capturing screenshots";
    case "katana":
      return "Crawling same-origin URLs";
    case "nuclei":
      return "Checking vulnerabilities";
    case "ffuf":
    case "feroxbuster":
    case "gobuster":
      return "Discovering paths";
    case "dig":
      return rawArgs.includes("axfr") ? "Checking DNS zone transfer" : "Querying DNS";
    case "whois":
      return "Looking up WHOIS";
    default: {
      const wrappedTool =
        typeof args?.tool_name === "string" ? formatExternalToolName(args.tool_name) : null;
      return wrappedTool ? `Running ${wrappedTool}` : "Running pentest tool";
    }
  }
}

export function getToolPrimaryArg(name: string, args: Record<string, unknown>): string | null {
  if (name === "wait_for_background_jobs") {
    return formatWaitForBackgroundJobsSummary(args);
  }
  if ((name === "run_command" || name === "run_pty_cmd") && args.command)
    return formatCommandForDisplay(String(args.command));
  if (name === "enum_crawl_same_origin_urls") {
    return formatEnumCrawlSameOriginUrlsSummary(args);
  }
  if (name === "vuln_run_formulaic_sweep") {
    return formatVulnRunFormulaicSweepSummary(args);
  }
  // pentest_run wraps the real tool in `tool_name` + `args`; the card title now
  // carries the action ("Probing services"), so the secondary line stays compact
  // and avoids repeating the raw command prefix.
  if (typeof args.tool_name === "string") {
    return formatPentestRunSummary(args);
  }
  if (args.path) return String(args.path);
  if (args.file_path) return String(args.file_path);
  if (args.url) return String(args.url);
  if (args.query) return String(args.query);
  if (args.pattern) return String(args.pattern);
  return null;
}

function formatWaitForBackgroundJobsSummary(args: Record<string, unknown>): string {
  const timeoutSecs =
    typeof args.timeout_secs === "number" && Number.isFinite(args.timeout_secs)
      ? Math.max(0, Math.trunc(args.timeout_secs))
      : null;
  const pollMs =
    typeof args.poll_interval_ms === "number" && Number.isFinite(args.poll_interval_ms)
      ? Math.max(0, Math.trunc(args.poll_interval_ms))
      : null;

  const parts = [timeoutSecs == null ? "default wait up to 300s" : `wait up to ${timeoutSecs}s`];
  if (pollMs != null) parts.push(`poll ${pollMs}ms`);
  return parts.join(" | ");
}

function formatPentestRunSummary(args: Record<string, unknown>): string | null {
  const rawToolName = typeof args.tool_name === "string" ? args.tool_name.trim() : "";
  if (!rawToolName) return null;

  const toolLabel = formatExternalToolName(rawToolName);
  const rawArgs = args.args != null ? String(args.args) : "";
  const batchSummary = formatPentestInputSummary(args);
  const details = formatPentestRunDetailSummaries(rawToolName, rawArgs, Boolean(batchSummary));
  if (!batchSummary && details.length === 0) return null;

  return [toolLabel, batchSummary, ...details].filter(Boolean).join(" · ");
}

export function formatCommandForDisplay(command: string): string {
  return command.replace(/\\n/g, "\n").replace(/\\r/g, "").replace(/\\t/g, "    ");
}

const INPUT_FILE_PLACEHOLDER_RE =
  /\{\{(?:input_file|targets_file|hosts_file|urls_file)\}\}|\{input_file\}|\$GOLISH_INPUT_FILE/g;

function formatPentestCommandForDisplay(command: string, hasBatchInput: boolean): string {
  const displayedCommand = hasBatchInput
    ? command.replace(INPUT_FILE_PLACEHOLDER_RE, "[input file]")
    : command;
  return formatCommandForDisplay(displayedCommand);
}

function formatPentestRunDetailSummaries(
  toolName: string,
  rawArgs: string,
  hasBatchInput: boolean
): string[] {
  const lowerTool = toolName.trim().toLowerCase();
  const details: string[] = [];
  const ports = matchCliArgValue(rawArgs, "-p");
  const topPorts = matchCliArgValue(rawArgs, "-top-ports");

  if (ports) details.push(`ports ${compactInlineValue(ports)}`);
  if (topPorts) details.push(`top ${compactInlineValue(topPorts)} ports`);

  if (
    lowerTool === "nmap" ||
    lowerTool === "naabu" ||
    lowerTool === "masscan" ||
    lowerTool === "httpx" ||
    lowerTool === "whatweb" ||
    lowerTool === "gowitness"
  ) {
    return details;
  }

  if (hasBatchInput) return details;

  const cleaned = compactPentestArgsForDisplay(toolName, rawArgs);
  return cleaned ? [...details, cleaned] : details;
}

function matchCliArgValue(rawArgs: string, flag: string): string | null {
  const match = rawArgs.match(new RegExp(`(?:^|\\s)${escapeRegExp(flag)}(?:=|\\s+)([^\\s]+)`, "i"));
  return match?.[1] ?? null;
}

function compactPentestArgsForDisplay(toolName: string, rawArgs: string): string | null {
  let cleaned = formatPentestCommandForDisplay(rawArgs, false).replace(/\s+/g, " ").trim();
  if (!cleaned) return null;

  const toolPrefix = new RegExp(`^${escapeRegExp(toolName)}\\s+`, "i");
  cleaned = cleaned.replace(toolPrefix, "").trim();
  cleaned = cleaned
    .replace(INPUT_FILE_PLACEHOLDER_RE, "[input file]")
    .replace(/(?:^|\s)(?:-iL|-list|-l|-f|--input-file)(?:=|\s+)\[input file\]/gi, " ")
    .replace(/\s+/g, " ")
    .trim();

  return cleaned ? compactInlineValue(cleaned) : null;
}

function compactInlineValue(value: string): string {
  const maxLength = 72;
  const compacted = value.trim();
  if (compacted.length <= maxLength) return compacted;
  return `${compacted.slice(0, 44)}...${compacted.slice(-24)}`;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function formatPentestInputSummary(args: Record<string, unknown>): string | null {
  const inputLines = getPentestRunInputLines(args);
  return formatTargetListSummary(inputLines);
}

function formatEnumCrawlSameOriginUrlsSummary(args: Record<string, unknown>): string | null {
  const targetSummary = Array.isArray(args.target_urls)
    ? formatTargetListSummary(
        args.target_urls
          .map((line) => normalizeInputLine(line))
          .filter((line): line is string => line != null)
      )
    : null;
  const depth =
    typeof args.depth === "number" && Number.isFinite(args.depth)
      ? `depth ${Math.max(1, Math.trunc(args.depth))}`
      : null;
  return [targetSummary, depth].filter(Boolean).join(" · ") || null;
}

function formatVulnRunFormulaicSweepSummary(args: Record<string, unknown>): string | null {
  const targetSummary = Array.isArray(args.targets)
    ? formatTargetListSummary(
        args.targets
          .map((line) => normalizeInputLine(line))
          .filter((line): line is string => line != null)
      )
    : null;
  const techniques = Array.isArray(args.techniques)
    ? args.techniques
        .map((technique) => normalizeInputLine(technique))
        .filter((technique): technique is string => technique != null)
    : [];
  const techniqueSummary =
    techniques.length === 0
      ? null
      : techniques.length === 1
        ? techniques[0]
        : `${techniques.length} techniques`;
  return [targetSummary, techniqueSummary].filter(Boolean).join(" · ") || null;
}

function formatTargetListSummary(inputLines: string[]): string | null {
  if (inputLines.length === 0) return null;

  const noun = inputLines.length === 1 ? "target" : "targets";
  const first = compactTargetLabel(inputLines[0] ?? "");
  const last = compactTargetLabel(inputLines[inputLines.length - 1] ?? "");
  if (!first) return `batch ${inputLines.length} ${noun}`;
  if (inputLines.length === 1 || first === last)
    return `batch ${inputLines.length} ${noun} (${first})`;
  return `batch ${inputLines.length} ${noun} (${first} ... ${last})`;
}

export function getPentestRunInputLines(args: Record<string, unknown>): string[] {
  if (Array.isArray(args.input_lines)) {
    return args.input_lines
      .map((line) => normalizeInputLine(line))
      .filter((line): line is string => line != null);
  }

  if (typeof args.input_lines === "string") {
    return splitInputLines(args.input_lines);
  }

  return typeof args.stdin === "string" ? splitInputLines(args.stdin) : [];
}

function splitInputLines(input: string): string[] {
  return input
    .split(/\r?\n/)
    .map((line) => normalizeInputLine(line))
    .filter((line): line is string => line != null);
}

function normalizeInputLine(line: unknown): string | null {
  if (typeof line !== "string") return null;
  const trimmed = line.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function compactTargetLabel(value: string): string {
  const maxLength = 72;
  if (value.length <= maxLength) return value;
  return `${value.slice(0, 40)}...${value.slice(-24)}`;
}

/** Format result for display */
export function formatToolResult(result: unknown): string {
  if (typeof result === "string") {
    return result;
  }
  return safeStringify(result);
}

const FAILURE_STATUS_RE = /"status"\s*:\s*"(rejected|needs_fix|error|failed)"/i;
const FAILURE_STATUSES = new Set(["rejected", "needs_fix", "error", "failed"]);
const STDERR_FAILURE_RE = /(^|\s)(error|fatal|exception)([:\s]|$)/i;
const OUTPUT_FAILURE_RE = /\b(not installed|missing dependenc(?:y|ies)|command not found)\b/i;

function parseResultObject(result: unknown): Record<string, unknown> | null {
  if (result != null && typeof result === "object" && !Array.isArray(result)) {
    return result as Record<string, unknown>;
  }
  if (typeof result !== "string") return null;
  const trimmed = result.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(trimmed);
    return parsed != null && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function topLevelStatusIndicatesFailure(status: unknown): boolean {
  return typeof status === "string" && FAILURE_STATUSES.has(status.toLowerCase());
}

/**
 * Some tools return success at the transport layer while their payload is a
 * domain failure (e.g. `submit_stage_deliverable` needs_fix) or a shell-like
 * result with `exit_code: 0` but fatal stderr (`whatweb` can do this). UI status
 * indicators should use this helper before painting a green success icon.
 */
export function toolResultIndicatesFailure(result?: unknown): boolean {
  if (result == null || result === "") return false;

  const obj = parseResultObject(result);
  if (!obj) {
    const text = typeof result === "string" ? result : safeStringify(result);
    return FAILURE_STATUS_RE.test(text);
  }

  if (topLevelStatusIndicatesFailure(obj.status)) return true;

  const exitCode =
    typeof obj.exit_code === "number"
      ? obj.exit_code
      : typeof obj.exitCode === "number"
        ? obj.exitCode
        : null;
  if (exitCode != null && exitCode !== 0) return true;

  const stderr = [obj.stderr, obj.partial_stderr]
    .filter((value): value is string => typeof value === "string" && value.trim().length > 0)
    .join("\n");
  if (STDERR_FAILURE_RE.test(stripAnsiForDisplay(stderr))) return true;

  const outputText = [obj.stdout, obj.output, obj.error, obj.message]
    .filter((value): value is string => typeof value === "string" && value.trim().length > 0)
    .join("\n");
  return OUTPUT_FAILURE_RE.test(stripAnsiForDisplay(outputText));
}

/** Type guard to check if a result is a shell command result */
function isShellCommandResult(
  result: unknown
): result is { stdout: string; stderr: string; exit_code: number; command?: string } {
  return (
    typeof result === "object" && result !== null && "stdout" in result && "exit_code" in result
  );
}

/** Format shell command result for display (shows stdout/stderr, not raw JSON) */
export function formatShellCommandResult(result: unknown): string {
  if (!isShellCommandResult(result)) {
    return formatToolResult(result);
  }

  const parts: string[] = [];

  // Show stdout if present
  if (result.stdout?.trim()) {
    parts.push(result.stdout.trimEnd());
  }

  // Show stderr if present (and different from stdout)
  if (result.stderr?.trim() && result.stderr !== result.stdout) {
    if (parts.length > 0) parts.push(""); // Add blank line
    parts.push(result.stderr.trimEnd());
  }

  // If no output, show exit code
  if (parts.length === 0) {
    if (result.exit_code === 0) {
      return "(no output)";
    }
    return `Exit code: ${result.exit_code}`;
  }

  return parts.join("\n");
}

/** Type guard to check if a result is an edit_file result with diff */
export function isEditFileResult(result: unknown): result is { diff: string; path?: string } {
  return (
    typeof result === "object" &&
    result !== null &&
    "diff" in result &&
    typeof (result as { diff: unknown }).diff === "string"
  );
}

/** Read-only tools that pose minimal risk */
const READ_ONLY_TOOLS = [
  "read_file",
  "grep_file",
  "list_files",
  "indexer_search_code",
  "indexer_search_files",
  "indexer_analyze_file",
  "indexer_extract_symbols",
  "indexer_get_metrics",
  "indexer_detect_language",
  "debug_agent",
  "analyze_agent",
  "get_errors",
  "list_skills",
  "search_skills",
  "load_skill",
  "search_tools",
  "update_plan",
  "web_fetch",
];

/** Write operations that are recoverable */
const WRITE_TOOLS = ["write_file", "create_file", "edit_file", "apply_patch", "save_skill"];

/** Shell execution tools */
const SHELL_TOOLS = ["run_pty_cmd", "create_pty_session", "send_pty_input"];

/** Destructive operations */
const DESTRUCTIVE_TOOLS = ["delete_file", "execute_code"];

/** Tools that can modify files or execute code (dangerous operations) */
const DANGEROUS_TOOLS = [
  "write_file",
  "edit_file",
  "apply_patch",
  "run_pty_cmd",
  "shell",
  "execute_code",
  "delete_file",
];

/** Get the risk level for a tool based on its name */
export function getRiskLevel(toolName: string): RiskLevel {
  if (READ_ONLY_TOOLS.includes(toolName)) {
    return "low";
  }
  if (WRITE_TOOLS.includes(toolName)) {
    return "medium";
  }
  if (SHELL_TOOLS.includes(toolName)) {
    return "high";
  }
  if (DESTRUCTIVE_TOOLS.includes(toolName)) {
    return "critical";
  }
  // Sub-agents are medium risk
  if (toolName.startsWith("sub_agent_")) {
    return "medium";
  }
  // Default for unknown tools
  return "high";
}

/** Check if a tool is considered dangerous */
export function isDangerousTool(toolName: string, riskLevel?: RiskLevel): boolean {
  const level = riskLevel ?? getRiskLevel(toolName);
  return DANGEROUS_TOOLS.includes(toolName) || level === "high" || level === "critical";
}
