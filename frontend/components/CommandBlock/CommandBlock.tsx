import { ChevronDown, ChevronRight, Clock, Maximize2, Minimize2 } from "lucide-react";
import { useMemo, useState } from "react";
import { CopyButton } from "@/components/Markdown/CopyButton";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { stripOscSequences } from "@/lib/ansi";
import { formatDurationLong } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { CommandBlock as CommandBlockType } from "@/store";
import { StaticTerminalOutput } from "./StaticTerminalOutput";

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

export function CommandBlock({ block, sessionId, onToggleCollapse, source }: CommandBlockProps) {
  const isSuccess = block.exitCode === 0;
  const [outputExpanded, setOutputExpanded] = useState(false);

  // Strip OSC sequences but keep ANSI color codes for rendering
  const cleanOutput = useMemo(() => stripOscSequences(block.output), [block.output]);
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

      {/* Output with fixed height preview + expand */}
      <CollapsibleContent>
        <div className="px-5 pb-2">
          <div
            className={cn(
              "relative overflow-hidden transition-[max-height] duration-200",
              !outputExpanded && "max-h-[120px]",
            )}
          >
            <StaticTerminalOutput
              output={cleanOutput}
              sessionId={sessionId}
              workingDirectory={block.workingDirectory}
            />
            {!outputExpanded && cleanOutput.length > 200 && (
              <div className="absolute bottom-0 left-0 right-0 h-8 bg-gradient-to-t from-card to-transparent pointer-events-none" />
            )}
          </div>
          {cleanOutput.length > 200 && (
            <button
              type="button"
              className="flex items-center gap-1 mt-1 text-[10px] text-muted-foreground/50 hover:text-muted-foreground/70 transition-colors"
              onClick={() => setOutputExpanded(!outputExpanded)}
            >
              {outputExpanded ? (
                <><Minimize2 className="w-3 h-3" /> Collapse</>
              ) : (
                <><Maximize2 className="w-3 h-3" /> Expand</>
              )}
            </button>
          )}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
