import { ChevronDown, ChevronRight, Clock } from "lucide-react";
import { useMemo } from "react";
import { Ansi } from "@/components/Ansi/Ansi";
import { CopyButton } from "@/components/Markdown/CopyButton";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { stripOscSequences } from "@/lib/ansi";
import { formatDurationLong } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { CommandBlock as CommandBlockType } from "@/store";

// Static style constants extracted to avoid recreation on each render
const codeStyle = {
  fontSize: "12px",
  lineHeight: 1.4,
  fontFamily: "SF Mono, Menlo, Monaco, JetBrains Mono, Consolas, monospace",
} as const;

// Drop C0/C1 control characters (except CR/LF/TAB) and common zero-width chars.
// Used for visibility checks and for command-echo detection, so that invisible
// PTY noise (cursor moves, OSC 133 residue, zero-width joiners, etc.) doesn't
// hide an empty line or break the leading-line equality test.
const INVISIBLE_RE = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f\u200b-\u200d\ufeff]/g;
// Match any CSI sequence: ESC [ <intermediate/params> <final-byte>.
// Final byte is any letter; parameters include digits / `;` / `?` / `:` / `<>=`.
// Windows ConPTY wraps the echoed command line in cursor-mode toggles
// (`\x1b[?25l`, `\x1b[K`, `\x1b[?25h`, …) — stripping ESC alone leaves the
// `[?25l[K…` debris that breaks the `visible === cmd` equality test below.
const CSI_RE = /\x1b\[[\d;?:<>= ]*[a-zA-Z]/g;

function stripInvisible(s: string): string {
  return s.replace(INVISIBLE_RE, "");
}

// Reduce a raw output line to the user-visible text it would render as, with
// every CSI escape (cursor moves, SGR colours, DEC private modes) and every
// C0/C1 control byte removed. Used to compare the leading line of `output`
// against the typed command — see `stripCommandEcho` below.
function visibleText(line: string): string {
  return line.replace(CSI_RE, "").replace(INVISIBLE_RE, "").trim();
}

// Strip a single leading line that is the shell's echo of the typed command.
// Tolerates leading blank/control-only lines and the case where extra text
// (e.g. `dir   Directory: ...`) was appended on the same line as the echo.
function stripCommandEcho(output: string, command: string): string {
  const cmd = command.trim();
  if (!cmd) return output;
  const lines = output.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const visible = visibleText(lines[i]);
    if (visible === "") continue;
    if (visible === cmd) {
      return lines.slice(i + 1).join("\n");
    }
    if (visible.startsWith(`${cmd} `) || visible.startsWith(`${cmd}\t`)) {
      // Operate on the CSI-stripped version of the line so we don't reinsert
      // cursor-mode debris when we keep the trailing text after `cmd`.
      const rawLine = lines[i].replace(CSI_RE, "");
      const idx = rawLine.indexOf(cmd);
      if (idx !== -1) {
        const remainder = rawLine.slice(idx + cmd.length).replace(/^[\s\u3000]+/, "");
        return [remainder, ...lines.slice(i + 1)].join("\n");
      }
    }
    return output;
  }
  return output;
}

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

  // Strip OSC sequences but keep ANSI color codes for rendering, then drop a
  // leading line that is just the shell echoing the typed command back. The
  // echo happens on Windows ConPTY and sometimes on POSIX shells when stdin
  // is in cooked mode.
  const cleanOutput = useMemo(() => {
    const stripped = stripOscSequences(block.output);
    return stripCommandEcho(stripped, block.command ?? "");
  }, [block.output, block.command]);

  // `hasOutput` must reflect *visible* output — otherwise lone CSI escapes
  // (cursor moves, DEC private modes, SGR colour resets) or bare control
  // characters keep the empty output panel expanded ("black box" right after
  // the first command of a new PowerShell session).
  const hasOutput = useMemo(() => {
    const visible = stripInvisible(cleanOutput.replace(CSI_RE, "")).trim();
    return visible.length > 0;
  }, [cleanOutput]);

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

      {/* Output is always fully expanded — no inner expand/collapse toggle. */}
      <CollapsibleContent>
        <div className="px-5 pb-2">
          <pre
            className="ansi-output whitespace-pre-wrap break-words m-0 text-muted-foreground"
            style={codeStyle}
          >
            <Ansi useClasses>{cleanOutput}</Ansi>
          </pre>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
