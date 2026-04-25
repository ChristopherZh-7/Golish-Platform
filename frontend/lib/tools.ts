/**
 * Shared utilities for tool call display components.
 */

import type { LucideIcon } from "lucide-react";
import { FileCode, FileSearch, FolderOpen, Globe, Network, Pencil, Search, Terminal, Wrench } from "lucide-react";

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
};

export function getToolColor(name: string): string {
  return TOOL_COLORS[name] || "var(--ansi-blue)";
}

export function getToolIcon(name: string): LucideIcon {
  return TOOL_ICONS[name] || Wrench;
}

/** Base properties shared by all tool call types */
export interface BaseToolCall {
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
    .split("_")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

const TOOL_LABELS_SHORT: Record<string, string> = {
  run_command: "Shell",
  run_pty_cmd: "Shell",
  read_file: "Read",
  write_file: "Write",
  edit_file: "Edit",
  search_files: "Search",
  web_search: "Web",
  web_fetch: "Fetch",
  manage_targets: "Targets",
  record_finding: "Finding",
};

const TOOL_LABELS_STANDARD: Record<string, string> = {
  run_command: "Shell Command",
  run_pty_cmd: "Shell Command",
  read_file: "Read File",
  write_file: "Write File",
  edit_file: "Edit File",
  search_files: "Search Files",
  web_search: "Web Search",
  web_fetch: "Fetch URL",
  manage_targets: "Manage Targets",
  record_finding: "Record Finding",
  credential_vault: "Credential Vault",
  js_collect: "JS Collect",
  run_pipeline: "Run Pipeline",
  flow_compose: "Flow Compose",
  pentest_run: "Pentest Run",
  pentest_list_tools: "List Tools",
  pentest_read_skill: "Read Skill",
};

export function getToolLabel(name: string, variant: "short" | "standard" = "standard"): string {
  const map = variant === "short" ? TOOL_LABELS_SHORT : TOOL_LABELS_STANDARD;
  return map[name] || formatToolName(name);
}

export function getToolPrimaryArg(name: string, args: Record<string, unknown>): string | null {
  if ((name === "run_command" || name === "run_pty_cmd") && args.command)
    return String(args.command);
  if (args.path) return String(args.path);
  if (args.file_path) return String(args.file_path);
  if (args.url) return String(args.url);
  if (args.query) return String(args.query);
  if (args.pattern) return String(args.pattern);
  return null;
}

/** Format result for display */
export function formatToolResult(result: unknown): string {
  if (typeof result === "string") {
    return result;
  }
  return JSON.stringify(result, null, 2);
}

/** Type guard to check if a result is a shell command result */
export function isShellCommandResult(
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

/** Risk level for tool operations */
export type RiskLevel = "low" | "medium" | "high" | "critical";

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
export const DANGEROUS_TOOLS = [
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
