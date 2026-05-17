/**
 * ToolCallDetailView
 *
 * Left-pane detail panel for an individual AI tool call. Mounted by
 * `PaneLeaf` whenever `detailViewMode === "tool-detail"` for the active
 * pane session. Reads the target `requestId` from
 * `sessions[sessionId].toolDetailRequestIds[0]` and looks up the
 * matching `ai_tool_execution` block in the session timeline.
 *
 * Mirrors the visual structure of `SubAgentDetailView` (header + scroll
 * body) so users get a consistent feel when toggling between sub-agent
 * and tool-call detail views.
 */
import { ArrowLeft, CheckCircle2, Clock, Loader2, Wrench, XCircle } from "lucide-react";
import { memo, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Markdown } from "@/components/Markdown";
import { AnchorChip } from "@/components/ui/AnchorChip";
import { Badge } from "@/components/ui/badge";
import { stripAllAnsi, stripBareSgrArtifacts } from "@/lib/ansi";
import { getToolColor, getToolLabel } from "@/lib/tools";
import { cn } from "@/lib/utils";
import type { AiToolExecution } from "@/store";
import { useStore } from "@/store";

/**
 * Two-pass terminal output cleanup for display:
 *   1. `stripAllAnsi`           — kills full ESC-prefixed sequences,
 *                                 collapses `\r` overwrites, drops C0
 *                                 control bytes, trims zsh `%`.
 *   2. `stripBareSgrArtifacts`  — chews trailing `[Nm` / `%` / `\r`
 *                                 fragments that lost their ESC byte
 *                                 somewhere upstream (JSON, IPC).
 *
 * The pair runs idempotent so calling twice is safe.
 */
function cleanTerminalText(value: string): string {
  return stripBareSgrArtifacts(stripAllAnsi(value));
}

/**
 * Recursively clean every string leaf in a tool-call `result` payload so
 * that `JSON.stringify(result, null, 2)` doesn't dump raw `\u001b[1m…`
 * sequences and stray `\r\n` literals into the UI. Object identity is
 * lost (we rebuild plain objects/arrays), but the caller only uses the
 * cleaned tree for display.
 */
function deepCleanForDisplay(value: unknown): unknown {
  if (typeof value === "string") return cleanTerminalText(value);
  if (Array.isArray(value)) return value.map(deepCleanForDisplay);
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      out[k] = deepCleanForDisplay(v);
    }
    return out;
  }
  return value;
}

/**
 * Take the output of `JSON.stringify(_, null, 2)` and turn the escaped
 * whitespace inside string values back into real characters, so that
 * `stdout: "line1\nline2"` renders as two physical lines inside the
 * `<pre>` (with `whitespace-pre-wrap`) instead of the ugly literal `\n`.
 *
 * Caveat: the resulting text is no longer valid JSON (string values
 * span multiple lines). That's deliberate — this output is for human
 * eyes only, never re-parsed.
 *
 * Global replace is safe because at this point the *only* place a `\n`
 * literal can appear is inside a string value: JSON's structural
 * separators (after `,`, after `{`, before `}`, etc.) are already real
 * newlines emitted by `JSON.stringify`'s 2-space indent.
 */
function humanizeJsonWhitespace(jsonStr: string): string {
  return jsonStr
    .replace(/\\r\\n/g, "\n")
    .replace(/\\n/g, "\n")
    .replace(/\\r/g, "")
    .replace(/\\t/g, "  ");
}

interface ToolCallDetailViewProps {
  sessionId: string;
}

function formatDurationShort(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const minutes = Math.floor(seconds / 60);
  const remSec = Math.round(seconds - minutes * 60);
  return `${minutes}m ${remSec}s`;
}

function ToolArgsTable({ args }: { args: Record<string, unknown> }) {
  const entries = Object.entries(args);
  if (entries.length === 0) return null;

  return (
    <div className="divide-y divide-border/15">
      {entries.map(([key, value]) => {
        const strValue = typeof value === "string" ? value : JSON.stringify(value, null, 2);
        const isLong = strValue.length > 120 || strValue.includes("\n");
        return (
          <div
            key={key}
            className={cn("px-3", isLong ? "py-2" : "py-1.5 flex items-baseline gap-3")}
          >
            <span className="text-[10px] font-mono text-[var(--ansi-cyan)]/70 flex-shrink-0">
              {key}
            </span>
            {isLong ? (
              <pre className="mt-1 text-[11px] font-mono text-foreground/80 whitespace-pre-wrap break-all max-h-40 overflow-auto leading-relaxed">
                {strValue}
              </pre>
            ) : (
              <span className="text-[11px] font-mono text-foreground/80 truncate" title={strValue}>
                {strValue}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}

function ToolResultDisplay({ result }: { result: unknown }) {
  if (result === null || result === undefined) return null;

  // Clean ANSI / control-byte / zsh-prompt noise out of every string leaf
  // before deciding how to render. Tool results commonly look like
  //   { stdout: "\u001b[1m…\r\n", exit_code: 0 }
  // and dumping that raw via JSON.stringify produces the ugly
  // "\u001b\\Usage…\r\n[1m%[0m" the user reported.
  const cleaned = typeof result === "string" ? cleanTerminalText(result) : deepCleanForDisplay(result);

  const strResult =
    typeof cleaned === "string"
      ? cleaned
      : humanizeJsonWhitespace(JSON.stringify(cleaned, null, 2));
  const isMarkdownLike =
    typeof cleaned === "string" &&
    (/^#{1,3}\s/m.test(cleaned) ||
      /\*\*/.test(cleaned) ||
      /^[-*]\s/m.test(cleaned) ||
      /```/.test(cleaned));

  if (isMarkdownLike) {
    return (
      <div className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-[480px] overflow-auto text-[12px] text-foreground leading-[1.65] [&_p]:mb-1.5 [&_p:last-child]:mb-0">
        <Markdown content={cleaned as string} />
      </div>
    );
  }

  return (
    <pre className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-[480px] overflow-auto text-[11px] font-mono text-foreground/80 whitespace-pre-wrap break-all leading-relaxed">
      {strResult.length > 8000 ? `${strResult.slice(0, 8000)}\n... (truncated)` : strResult}
    </pre>
  );
}

const STATUS_BADGE_STYLES: Record<AiToolExecution["status"], string> = {
  running: "bg-[var(--accent-dim)] text-accent",
  completed: "bg-[var(--success-dim)] text-[var(--success)]",
  error: "bg-destructive/10 text-destructive",
  interrupted: "bg-yellow-500/10 text-yellow-400",
};

function getStatusLabel(status: AiToolExecution["status"]): string {
  switch (status) {
    case "running":
      return "Running";
    case "completed":
      return "Completed";
    case "error":
      return "Error";
    case "interrupted":
      return "Interrupted";
  }
}

export const ToolCallDetailView = memo(function ToolCallDetailView({
  sessionId,
}: ToolCallDetailViewProps) {
  const { t } = useTranslation();
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);
  const requestIds = useStore((s) => s.sessions[sessionId]?.toolDetailRequestIds);
  const targetRequestId = requestIds?.[0] ?? null;

  const execution = useStore((s) => {
    if (!targetRequestId) return null;
    const timeline = s.timelines[sessionId] ?? [];
    for (let i = timeline.length - 1; i >= 0; i--) {
      const block = timeline[i];
      if (block.type === "ai_tool_execution" && block.data.requestId === targetRequestId) {
        return block.data;
      }
    }
    return null;
  });

  const navigateBack = () => setDetailViewMode(sessionId, "timeline");

  const toolColor = useMemo(
    () => (execution ? getToolColor(execution.toolName) : undefined),
    [execution]
  );
  const toolLabel = useMemo(
    () => (execution ? getToolLabel(execution.toolName, "short") : ""),
    [execution]
  );

  if (!execution) {
    return (
      <div className="h-full flex flex-col bg-card">
        <div className="flex items-center gap-3 px-3 py-2 border-b border-[var(--border-subtle)] flex-shrink-0">
          <button
            type="button"
            onClick={navigateBack}
            className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
          >
            <ArrowLeft className="w-3.5 h-3.5" />
            {t("ai.toolDetail.backToTerminal")}
          </button>
        </div>
        <div className="flex-1 flex items-center justify-center text-sm text-muted-foreground/60">
          {t("ai.toolDetail.noToolExecutions")}
        </div>
      </div>
    );
  }

  const isRunning = execution.status === "running";
  const isError = execution.status === "error";
  const errorMessage = (() => {
    if (!isError) return null;
    const raw = (() => {
      if (typeof execution.result === "string") return execution.result;
      if (typeof execution.result === "object" && execution.result !== null) {
        const r = execution.result as Record<string, unknown>;
        const e = r.error || r.message;
        if (typeof e === "string") return e;
      }
      return null;
    })();
    return raw ? cleanTerminalText(raw) : null;
  })();

  const isShellCmd = execution.toolName === "run_pty_cmd" || execution.toolName === "run_command";
  const shellOutput = (() => {
    if (!isShellCmd) return null;
    const raw = (() => {
      if (execution.streamingOutput) return execution.streamingOutput;
      if (!execution.result || typeof execution.result !== "object") return null;
      const r = execution.result as Record<string, unknown>;
      return (r.stdout as string) || (r.output as string) || null;
    })();
    return raw ? cleanTerminalText(raw) : null;
  })();

  return (
    <div className="h-full flex flex-col bg-card">
      <div className="flex items-center gap-3 px-3 py-2 border-b border-[var(--border-subtle)] flex-shrink-0">
        <button
          type="button"
          onClick={navigateBack}
          className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          <ArrowLeft className="w-3.5 h-3.5" />
          {t("ai.toolDetail.backToTerminal")}
        </button>
        <div className="w-px h-4 bg-[var(--border-subtle)]" />
        <Wrench className="w-4 h-4 flex-shrink-0" style={{ color: toolColor }} />
        <span className="text-sm font-medium truncate" title={execution.toolName}>
          {toolLabel}
        </span>
        <AnchorChip sessionId={sessionId} requestId={execution.requestId} />
        <Badge
          variant="outline"
          className={cn(
            "gap-1 flex items-center text-[10px] px-2 py-0.5",
            STATUS_BADGE_STYLES[execution.status]
          )}
        >
          {isRunning && <Loader2 className="w-3 h-3 animate-spin" />}
          {getStatusLabel(execution.status)}
        </Badge>
        {execution.durationMs != null && (
          <span className="text-[11px] text-muted-foreground/70 tabular-nums flex items-center gap-1">
            <Clock className="w-3 h-3" />
            {formatDurationShort(execution.durationMs)}
          </span>
        )}
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="px-4 py-3 border-b border-border/20 bg-accent/[0.04]">
          <div className="flex items-center gap-2 mb-1.5">
            <span className="text-[10px] font-mono text-muted-foreground/60 uppercase tracking-wider">
              Tool
            </span>
            <span className="text-[12px] font-mono text-foreground/90">{execution.toolName}</span>
          </div>
          <div className="flex items-center gap-2 text-[11px] text-muted-foreground/60">
            <span>Started: {new Date(execution.startedAt).toLocaleTimeString()}</span>
            {execution.completedAt && (
              <>
                <span>·</span>
                <span>Completed: {new Date(execution.completedAt).toLocaleTimeString()}</span>
              </>
            )}
          </div>
        </div>

        {Object.keys(execution.args ?? {}).length > 0 && (
          <div className="px-4 py-3 border-b border-border/20">
            <div className="text-[10px] font-semibold text-muted-foreground/70 uppercase tracking-wider mb-2">
              Input
            </div>
            <div className="rounded-md bg-muted/40 border border-border/20 overflow-hidden">
              <ToolArgsTable args={execution.args} />
            </div>
          </div>
        )}

        {isShellCmd && shellOutput && (
          <div className="px-4 py-3 border-b border-border/20">
            <div className="text-[10px] font-semibold text-muted-foreground/70 uppercase tracking-wider mb-2">
              Output
            </div>
            <pre className="max-h-[480px] overflow-auto whitespace-pre-wrap rounded bg-[var(--ansi-black)]/20 px-3 py-2 text-[11px] font-mono text-foreground/80">
              {shellOutput.length > 8000
                ? `${shellOutput.slice(0, 8000)}\n... (truncated)`
                : shellOutput}
            </pre>
          </div>
        )}

        {!isShellCmd && execution.result !== undefined && execution.result !== null && (
          <div className="px-4 py-3 border-b border-border/20">
            <div className="flex items-center gap-1.5 mb-2">
              {isError ? (
                <XCircle className="w-3 h-3 text-destructive" />
              ) : (
                <CheckCircle2 className="w-3 h-3 text-[var(--success)]" />
              )}
              <span className="text-[10px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                Output
              </span>
            </div>
            <ToolResultDisplay result={execution.result} />
          </div>
        )}

        {errorMessage && (
          <div className="mx-4 my-3 rounded-lg bg-destructive/10 border border-destructive/25 p-3.5">
            <div className="flex items-start gap-2">
              <XCircle className="w-3.5 h-3.5 text-destructive mt-0.5 flex-shrink-0" />
              <p className="text-[12.5px] text-destructive leading-[1.6] whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
                {errorMessage}
              </p>
            </div>
          </div>
        )}
      </div>

      {isRunning && (
        <div className="px-3 py-2 border-t border-[var(--border-subtle)] bg-accent/5 flex items-center gap-2 flex-shrink-0">
          <Loader2 className="w-3 h-3 text-accent animate-spin" />
          <span className="text-[11px] text-accent/80">{t("ai.toolDetail.running")}</span>
        </div>
      )}
    </div>
  );
});
