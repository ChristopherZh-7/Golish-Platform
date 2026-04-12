import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  FileSearch,
  Globe,
  Loader2,
  Network,
  Pencil,
  Search,
  Terminal,
  Wrench,
  XCircle,
} from "lucide-react";
import { memo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import type { AiToolExecution } from "@/store";

interface ToolExecutionCardProps {
  execution: AiToolExecution;
}

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

const TOOL_ICONS: Record<string, typeof Terminal> = {
  run_command: Terminal,
  run_pty_cmd: Terminal,
  read_file: FileSearch,
  write_file: Pencil,
  edit_file: Pencil,
  search_files: Search,
  web_search: Globe,
  web_fetch: Globe,
  manage_targets: Network,
};

function getToolColor(name: string): string {
  return TOOL_COLORS[name] || "var(--ansi-blue)";
}

function getToolIcon(name: string): typeof Terminal {
  return TOOL_ICONS[name] || Wrench;
}

function getToolLabel(name: string): string {
  const labels: Record<string, string> = {
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
  };
  return labels[name] || name.replace(/_/g, " ");
}

function getPrimaryDisplay(name: string, args: Record<string, unknown>): string | null {
  if ((name === "run_command" || name === "run_pty_cmd") && args.command) {
    return String(args.command);
  }
  if (args.path) return String(args.path);
  if (args.file_path) return String(args.file_path);
  if (args.pattern) return String(args.pattern);
  if (args.query) return String(args.query);
  if (args.url) return String(args.url);
  return null;
}

function formatDuration(ms?: number): string {
  if (!ms) return "";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function StatusIcon({ status }: { status: "running" | "completed" | "error" }) {
  switch (status) {
    case "completed":
      return <CheckCircle2 className="w-4 h-4 text-[var(--ansi-green)]" />;
    case "running":
      return <Loader2 className="w-4 h-4 text-[var(--ansi-blue)] animate-spin" />;
    case "error":
      return <XCircle className="w-4 h-4 text-[var(--ansi-red)]" />;
  }
}

export const ToolExecutionCard = memo(function ToolExecutionCard({
  execution,
}: ToolExecutionCardProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  const toolColor = getToolColor(execution.toolName);
  const ToolIcon = getToolIcon(execution.toolName);
  const toolLabel = getToolLabel(execution.toolName);
  const primary = getPrimaryDisplay(execution.toolName, execution.args);

  const isShellCommand =
    execution.toolName === "run_command" || execution.toolName === "run_pty_cmd";

  const resultText =
    typeof execution.result === "string"
      ? execution.result
      : execution.result != null
        ? JSON.stringify(execution.result, null, 2)
        : null;

  const outputPreview = execution.streamingOutput || resultText;

  return (
    <div
      className={cn(
        "mt-1 mb-1.5 rounded-lg border bg-card overflow-hidden transition-all",
        execution.status === "running"
          ? "border-l-2 animate-[pulse-border_2s_ease-in-out_infinite]"
          : "border border-border",
      )}
      style={
        execution.status === "running"
          ? {
              borderLeftColor: toolColor,
              boxShadow: `inset 2px 0 8px -4px ${toolColor}40`,
            }
          : undefined
      }
    >
      <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
        <div className="flex items-center gap-2 px-3 py-2">
          <CollapsibleTrigger className="flex flex-1 items-center gap-2 hover:bg-accent/30 rounded -ml-1 pl-1 py-0.5 min-w-0">
            {isExpanded ? (
              <ChevronDown className="h-3.5 w-3.5 text-muted-foreground flex-shrink-0" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-muted-foreground flex-shrink-0" />
            )}
            <ToolIcon
              className="h-4 w-4 flex-shrink-0"
              style={{ color: toolColor }}
            />
            <span className="font-medium text-sm truncate">{toolLabel}</span>

            {execution.status === "running" && (
              <Badge
                variant="outline"
                className="ml-auto gap-1 flex items-center text-[10px] px-2 py-0.5 rounded-full bg-[var(--accent-dim)] text-accent"
              >
                <Loader2 className="w-3 h-3 animate-spin" />
                Running
              </Badge>
            )}
          </CollapsibleTrigger>
          <div className="flex items-center gap-2 flex-shrink-0">
            <StatusIcon status={execution.status} />
            {execution.durationMs !== undefined && (
              <span className="text-xs text-muted-foreground">
                {formatDuration(execution.durationMs)}
              </span>
            )}
          </div>
        </div>

        {/* Primary argument preview (always visible) */}
        {primary && !isExpanded && (
          <div className="px-3 pb-2 -mt-1">
            <div
              className={cn(
                "text-xs font-mono truncate px-2 py-1 rounded",
                isShellCommand
                  ? "bg-[var(--ansi-black)]/30 text-[var(--ansi-green)]/90"
                  : "bg-muted/30 text-muted-foreground",
              )}
            >
              {isShellCommand && (
                <span className="text-muted-foreground/50 mr-1">$</span>
              )}
              {primary}
            </div>
          </div>
        )}

        <CollapsibleContent className="px-3 pb-2">
          {/* Full command / primary arg */}
          {primary && (
            <div
              className={cn(
                "text-xs font-mono px-2 py-1.5 rounded mb-1.5",
                isShellCommand
                  ? "bg-[var(--ansi-black)]/30 text-[var(--ansi-green)]/90"
                  : "bg-muted/30 text-muted-foreground",
              )}
            >
              {isShellCommand && (
                <span className="text-muted-foreground/50 mr-1">$</span>
              )}
              <span className="whitespace-pre-wrap break-all">{primary}</span>
            </div>
          )}

          {/* Non-primary args */}
          {!isShellCommand && Object.keys(execution.args).length > 0 && (
            <details className="mb-1.5">
              <summary className="cursor-pointer select-none text-[10px] text-muted-foreground hover:text-foreground/80">
                Arguments
              </summary>
              <pre className="mt-0.5 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-muted px-2 py-1 text-[10px]">
                {JSON.stringify(execution.args, null, 2)}
              </pre>
            </details>
          )}

          {/* Streaming output / result */}
          {outputPreview && (
            <div className="mt-1">
              <pre
                className={cn(
                  "max-h-48 overflow-auto whitespace-pre-wrap rounded px-2 py-1.5 text-[10px] font-mono",
                  isShellCommand
                    ? "bg-[var(--ansi-black)]/20 text-foreground/80"
                    : "bg-muted px-2 py-1",
                )}
              >
                {outputPreview.length > 2000
                  ? `${outputPreview.slice(0, 2000)}\n... (truncated)`
                  : outputPreview}
              </pre>
            </div>
          )}

          {/* Error display */}
          {execution.status === "error" && resultText && (
            <div className="mt-1.5 rounded bg-[var(--ansi-red)]/10 px-2 py-1.5 text-xs text-[var(--ansi-red)]">
              <span className="font-medium">Error: </span>
              {resultText.length > 500
                ? `${resultText.slice(0, 500)}...`
                : resultText}
            </div>
          )}
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
});
