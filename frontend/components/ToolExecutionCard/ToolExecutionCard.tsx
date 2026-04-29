import {
  ChevronDown,
  ChevronRight,
  ChevronsUpDown,
  Loader2,
} from "lucide-react";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { AnchorChip } from "@/components/ui/AnchorChip";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { StatusIcon } from "@/components/ui/StatusIcon";
import { stripAllAnsi } from "@/lib/ansi";
import { formatDurationShort } from "@/lib/time";
import { getToolColor, getToolIcon, getToolLabel, getToolPrimaryArg } from "@/lib/tools";
import { cn } from "@/lib/utils";
import type { AiToolExecution } from "@/store";

interface ToolExecutionCardProps {
  execution: AiToolExecution;
  compact?: boolean;
  highlighted?: boolean;
  /** Controlled open state (accordion mode). When provided, overrides internal state. */
  isOpen?: boolean;
  /** Called when the card's expand/collapse toggle is clicked (accordion mode). */
  onToggle?: () => void;
  /** Session ID — needed to look up the anchor chip (T#) from the store. */
  sessionId?: string | null;
}

function cleanTerminalOutput(raw: string): string {
  return stripAllAnsi(unescapeNewlines(raw)).trim();
}

function unescapeNewlines(s: string): string {
  return s
    .replace(/\\r\\n/g, "\n")
    .replace(/\\n/g, "\n")
    .replace(/\\r/g, "")
    .replace(/\\t/g, "\t");
}

function formatGenericResult(result: unknown): string | null {
  let obj: Record<string, unknown> | null = null;
  if (result != null && typeof result === "object") {
    obj = result as Record<string, unknown>;
  } else if (typeof result === "string") {
    try {
      const parsed = JSON.parse(result);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) obj = parsed;
    } catch {
      return unescapeNewlines(result);
    }
  }
  if (!obj) return null;

  const textKey = ["response", "output", "message", "content", "text"].find(
    (k) => typeof obj![k] === "string" && (obj![k] as string).length > 0
  );
  if (textKey) {
    const mainText = unescapeNewlines(obj[textKey] as string);
    const otherKeys = Object.keys(obj).filter((k) => k !== textKey);
    if (otherKeys.length === 0) return mainText;

    const meta = otherKeys
      .filter((k) => obj![k] != null && typeof obj![k] !== "object")
      .map((k) => `${k}: ${String(obj![k])}`)
      .join("  |  ");

    return meta ? `${meta}\n\n${mainText}` : mainText;
  }

  return JSON.stringify(obj, null, 2);
}

interface ShellResult {
  stdout?: string;
  stderr?: string;
  exitCode?: number;
}

function parseShellResult(result: unknown): ShellResult | null {
  let obj: Record<string, unknown> | null = null;

  if (result != null && typeof result === "object") {
    obj = result as Record<string, unknown>;
  } else if (typeof result === "string") {
    try {
      const parsed = JSON.parse(result);
      if (parsed && typeof parsed === "object") obj = parsed;
    } catch {
      return null;
    }
  }

  if (!obj) return null;

  if (obj.stdout !== undefined || obj.stderr !== undefined || obj.exit_code !== undefined) {
    return {
      stdout: typeof obj.stdout === "string" ? obj.stdout : undefined,
      stderr: typeof obj.stderr === "string" ? obj.stderr : undefined,
      exitCode: typeof obj.exit_code === "number" ? obj.exit_code : undefined,
    };
  }
  if (typeof obj.output === "string") {
    return { stdout: obj.output };
  }
  return null;
}


const PREVIEW_LIMIT = 2000;

function OutputBlock({ text, isShellCommand }: { text: string; isShellCommand: boolean }) {
  const isLong = text.length > PREVIEW_LIMIT;
  const [expanded, setExpanded] = useState(false);

  const display = expanded || !isLong ? text : `${text.slice(0, PREVIEW_LIMIT)}`;

  return (
    <div className="mt-1">
      <pre
        className={cn(
          "overflow-auto whitespace-pre-wrap rounded px-2 py-1.5 text-[10px] font-mono leading-relaxed",
          expanded ? "max-h-[80vh]" : "max-h-48",
          isShellCommand
            ? "bg-[var(--ansi-black)]/20 text-foreground/80"
            : "bg-muted px-2 py-1",
        )}
      >
        {display}
      </pre>
      {isLong && (
        <button
          type="button"
          onClick={() => setExpanded(!expanded)}
          className="mt-0.5 flex items-center gap-0.5 text-[10px] text-muted-foreground hover:text-foreground/80 transition-colors"
        >
          <ChevronsUpDown className="w-3 h-3" />
          {expanded ? "收起" : `展开全部 (${(text.length / 1000).toFixed(1)}k 字符)`}
        </button>
      )}
    </div>
  );
}

export const ToolExecutionCard = memo(function ToolExecutionCard({
  execution,
  compact = false,
  highlighted = false,
  isOpen: controlledOpen,
  onToggle,
  sessionId,
}: ToolExecutionCardProps) {
  const [internalOpen, setInternalOpen] = useState(highlighted);
  const isExpanded = controlledOpen !== undefined ? controlledOpen : internalOpen;
  const handleOpenChange = (open: boolean) => {
    if (onToggle) onToggle();
    else setInternalOpen(open);
  };
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (highlighted) {
      if (controlledOpen === undefined) setInternalOpen(true);
      cardRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }
  }, [highlighted, controlledOpen]);

  const toolColor = getToolColor(execution.toolName);
  const ToolIcon = getToolIcon(execution.toolName);
  const toolLabel = getToolLabel(execution.toolName);
  const primary = getToolPrimaryArg(execution.toolName, execution.args);

  const isShellCommand =
    execution.toolName === "run_command" || execution.toolName === "run_pty_cmd";

  const shellResult = useMemo(
    () => (isShellCommand ? parseShellResult(execution.result) : null),
    [isShellCommand, execution.result]
  );

  const resultText = useMemo(() => {
    if (execution.result == null) return null;
    if (shellResult) {
      const parts: string[] = [];
      if (shellResult.stdout) parts.push(cleanTerminalOutput(shellResult.stdout));
      if (shellResult.stderr) {
        const cleaned = cleanTerminalOutput(shellResult.stderr);
        if (cleaned) parts.push(`stderr: ${cleaned}`);
      }
      return parts.join("\n") || null;
    }
    return formatGenericResult(execution.result);
  }, [execution.result, shellResult]);

  const outputPreview = execution.streamingOutput || resultText;

  return (
    <div
      ref={cardRef}
      className={cn(
        "overflow-hidden transition-all",
        compact
          ? "rounded border-0 bg-transparent"
          : cn(
              "mt-1 mb-1.5 rounded-lg border bg-card",
              execution.status === "running"
                ? "border-l-2 animate-[pulse-border_2s_ease-in-out_infinite]"
                : "border border-border",
            ),
        highlighted && "ring-1 ring-accent/50 bg-accent/5",
      )}
      style={
        !compact && execution.status === "running"
          ? {
              borderLeftColor: toolColor,
              boxShadow: `inset 2px 0 8px -4px ${toolColor}40`,
            }
          : undefined
      }
    >
      <Collapsible open={isExpanded} onOpenChange={handleOpenChange}>
        <div className={cn("flex items-center gap-2", compact ? "px-1 py-1" : "px-3 py-2")}>
          <CollapsibleTrigger className="flex flex-1 items-center gap-2 hover:bg-accent/30 rounded -ml-1 pl-1 py-0.5 min-w-0">
            {isExpanded ? (
              <ChevronDown className={cn("text-muted-foreground flex-shrink-0", compact ? "h-3 w-3" : "h-3.5 w-3.5")} />
            ) : (
              <ChevronRight className={cn("text-muted-foreground flex-shrink-0", compact ? "h-3 w-3" : "h-3.5 w-3.5")} />
            )}
            <ToolIcon
              className={cn("flex-shrink-0", compact ? "h-3.5 w-3.5" : "h-4 w-4")}
              style={{ color: toolColor }}
            />
            <span className={cn("font-medium truncate", compact ? "text-xs" : "text-sm")}>{toolLabel}</span>
            <AnchorChip sessionId={sessionId} requestId={execution.requestId} />

            {execution.status === "running" && !compact && (
              <Badge
                variant="outline"
                className="ml-auto gap-1 flex items-center text-[10px] px-2 py-0.5 rounded-full bg-[var(--accent-dim)] text-accent"
              >
                <Loader2 className="w-3 h-3 animate-spin" />
                Running
              </Badge>
            )}
            {execution.status === "running" && compact && (
              <Loader2 className="w-3 h-3 animate-spin text-accent ml-auto flex-shrink-0" />
            )}
          </CollapsibleTrigger>
          <div className="flex items-center gap-2 flex-shrink-0">
            <StatusIcon status={execution.status} />
            {execution.durationMs !== undefined && (
              <span className={cn("text-muted-foreground", compact ? "text-[10px]" : "text-xs")}>
                {formatDurationShort(execution.durationMs)}
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

          {/* Exit code for shell commands */}
          {isShellCommand && shellResult?.exitCode != null && shellResult.exitCode !== 0 && (
            <div className="mt-1 flex items-center gap-1.5 text-[10px]">
              <span className="text-red-400 font-medium">exit {shellResult.exitCode}</span>
            </div>
          )}

          {/* Streaming output / result */}
          {outputPreview && (
            <OutputBlock
              text={outputPreview}
              isShellCommand={isShellCommand}
            />
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
