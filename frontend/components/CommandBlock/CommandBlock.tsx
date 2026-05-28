import { ChevronDown, ChevronRight, Clock } from "lucide-react";
import { useMemo } from "react";
import { Ansi } from "@/components/Ansi/Ansi";
import { CopyButton } from "@/components/Markdown/CopyButton";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { expandTerminalTabs, stripOscSequences } from "@/lib/ansi";
import { formatDurationLong } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { CommandBlock as CommandBlockType } from "@/store";

// Static style constants extracted to avoid recreation on each render
const codeStyle = {
  fontSize: "12px",
  lineHeight: 1.4,
  fontFamily: "SF Mono, Menlo, Monaco, JetBrains Mono, Consolas, monospace",
} as const;

interface CommandBlockProps {
  block: CommandBlockType;
  sessionId?: string;
  onToggleCollapse: (blockId: string) => void;
  source?: "manual" | "pipeline";
}

export function CommandBlock({
  block,
  sessionId: _sessionId,
  onToggleCollapse,
  source,
}: CommandBlockProps) {
  const isSuccess = block.exitCode === 0;

  // Strip OSC sequences but keep ANSI color codes for rendering
  const cleanOutput = useMemo(
    () => expandTerminalTabs(stripOscSequences(block.output)),
    [block.output]
  );
  const hasOutput = cleanOutput.trim().length > 0;

  // Content for copying (command + output)
  const copyContent = useMemo(() => {
    const command = `$ ${block.command || "(empty command)"}`;
    return hasOutput ? `${command}\n${cleanOutput}` : command;
  }, [block.command, cleanOutput, hasOutput]);

  return (
    <Collapsible
      open={hasOutput && !block.isCollapsed}
      onOpenChange={() => hasOutput && onToggleCollapse(block.id)}
      className="w-full group border-b border-border/10"
      data-testid="command-block"
    >
      {/* Header */}
      <div className="relative flex items-center">
        <CollapsibleTrigger
          className={cn(
            "flex items-center gap-2 px-5 py-3 w-full text-left select-none",
            hasOutput && "cursor-pointer"
          )}
          disabled={!hasOutput}
        >
          {/* Command */}
          <code className="flex-1 truncate text-foreground" style={codeStyle}>
            {source === "pipeline" && (
              <span className="inline-flex items-center text-[8px] px-1 py-px rounded bg-blue-500/15 text-blue-400 font-sans font-medium mr-1.5 align-middle leading-none">
                AUTO
              </span>
            )}
            <span className="text-[var(--ansi-green)]">$ </span>
            {block.command || "(empty command)"}
          </code>

          {/* Metadata */}
          <div className="flex items-center gap-3 text-xs text-muted-foreground flex-shrink-0">
            {block.durationMs !== null && (
              <span className="flex items-center gap-1">
                <Clock className="w-3 h-3" />
                {formatDurationLong(block.durationMs)}
              </span>
            )}
            {/* Show exit code only on failure */}
            {!isSuccess && block.exitCode !== null && (
              <span className="text-[var(--ansi-red)]">exit {block.exitCode}</span>
            )}
            {hasOutput && (
              <span className="flex items-center gap-0.5">
                {block.isCollapsed ? (
                  <ChevronRight className="w-3.5 h-3.5" />
                ) : (
                  <ChevronDown className="w-3.5 h-3.5" />
                )}
              </span>
            )}
          </div>
        </CollapsibleTrigger>
        {/* Copy button */}
        <CopyButton
          content={copyContent}
          className="absolute right-9 opacity-0 group-hover:opacity-100 transition-opacity"
          data-testid="command-block-copy-button"
        />
      </div>

      {/* Output */}
      <CollapsibleContent>
        <div className="px-5 pb-2 pt-1">
          <div className="relative overflow-x-auto overflow-y-hidden">
            <pre className="ansi-output m-0 whitespace-pre" style={codeStyle}>
              <Ansi useClasses>{cleanOutput}</Ansi>
            </pre>
          </div>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
