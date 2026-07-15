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
import { memo, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Ansi } from "@/components/Ansi";
import { AttackCandidateReview } from "@/components/Engagement/AttackCandidateReview";
import { CandidateAttemptRows } from "@/components/Engagement/CandidateAttemptRows";
import { CleanupObligationList } from "@/components/Engagement/CleanupObligationList";
import { ReportReadModelView } from "@/components/Engagement/ReportReadModelView";
import { StageRunOrgRows } from "@/components/Engagement/StageRunOrgRows";
import { JsonView } from "@/components/JsonView/JsonView";
import { Markdown } from "@/components/Markdown";
import { ToolAiTraceSummary } from "@/components/ToolAiTraceSummary";
import { BackgroundJobsBadge } from "@/components/UnifiedInput/StatusBadges";
import { AnchorChip } from "@/components/ui/AnchorChip";
import { Badge } from "@/components/ui/badge";
import {
  collapseProgressBars,
  expandTerminalTabs,
  stripAllAnsi,
  stripOscSequences,
} from "@/lib/ansi";
import { safeStringify } from "@/lib/text";
import { formatDurationLong } from "@/lib/time";
import {
  formatCommandForDisplay,
  getToolColor,
  getToolLabel,
  toolResultIndicatesFailure,
} from "@/lib/tools";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, AiToolExecution } from "@/store";
import { useStore } from "@/store";

interface ToolCallDetailViewProps {
  sessionId: string;
}

const EMPTY_BG_JOBS: never[] = [];
const EMPTY_SUB_AGENT_LIST: ActiveSubAgent[] = [];
const LIVE_OUTPUT_RENDER_LIMIT = 20000;

export const DETAIL_RUNNING_SPINNER_CLASS = "h-4 w-4 shrink-0 animate-spin";
export const DETAIL_PENDING_OUTPUT_SPINNER_CLASS = "h-4 w-4 shrink-0 animate-spin text-accent";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isShellLikeToolForDetail(toolName: string, args: unknown): boolean {
  if (toolName === "run_pty_cmd" || toolName === "run_command" || toolName === "pentest_run") {
    return true;
  }
  if (!isRecord(args)) return false;
  return (
    typeof args.tool_name === "string" &&
    (args.background === true ||
      typeof args.args === "string" ||
      typeof args.timeout_secs === "number")
  );
}

function normalizeToolArgs(
  args: unknown
): { kind: "record"; value: Record<string, unknown> } | { kind: "raw"; value: string } {
  if (isRecord(args)) {
    return { kind: "record", value: args };
  }

  if (typeof args === "string") {
    try {
      const parsed = JSON.parse(args);
      if (isRecord(parsed)) {
        return { kind: "record", value: parsed };
      }
    } catch {
      // Fall through to raw display for partial provider fragments.
    }
    return { kind: "raw", value: args };
  }

  return { kind: "raw", value: JSON.stringify(args, null, 2) ?? String(args) };
}

function hasToolArgs(args: unknown): boolean {
  if (isRecord(args)) return Object.keys(args).length > 0;
  if (typeof args === "string") return args.trim().length > 0;
  return args !== null && args !== undefined;
}

export function isAttackCandidateStageRun(toolName: string, args: unknown): boolean {
  if (toolName !== "stage_run") return false;
  const normalized = normalizeToolArgs(args);
  if (normalized.kind !== "record") return false;
  const stage = normalized.value.stage ?? normalized.value.stage_id;
  return stage === "attack_candidate";
}

export function isCleanupStageRun(toolName: string, args: unknown): boolean {
  if (toolName !== "stage_run") return false;
  const normalized = normalizeToolArgs(args);
  if (normalized.kind !== "record") return false;
  const stage = normalized.value.stage ?? normalized.value.stage_id;
  return stage === "cleanup";
}

function normalizedRecord(value: unknown): Record<string, unknown> | null {
  const normalized = normalizeToolArgs(value);
  return normalized.kind === "record" ? normalized.value : null;
}

function nonEmptyString(record: Record<string, unknown> | null, ...keys: string[]): string | null {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

/**
 * Resolve the Reporting read-model owner only from the selected `stage_run`
 * invocation/result pair. If both sides expose identity they must agree; a
 * conflicting stage or operation fails closed instead of mounting a report for
 * the wrong engagement.
 */
export function getReportingStageRunOperationId(
  toolName: string,
  args: unknown,
  result: unknown
): string | null {
  if (toolName !== "stage_run") return null;

  const argsRecord = normalizedRecord(args);
  const resultRecord = normalizedRecord(result);
  const stages = [
    nonEmptyString(argsRecord, "stage", "stage_id"),
    nonEmptyString(resultRecord, "stage", "stage_id"),
  ].filter((stage): stage is string => stage !== null);
  if (stages.length === 0 || stages.some((stage) => stage !== "reporting")) return null;

  const operationIds = [
    nonEmptyString(argsRecord, "operationId", "operation_id"),
    nonEmptyString(resultRecord, "operationId", "operation_id"),
  ].filter((operationId): operationId is string => operationId !== null);
  if (operationIds.length === 0 || operationIds.some((value) => value !== operationIds[0])) {
    return null;
  }
  return operationIds[0];
}

/**
 * Render a possibly-ANSI string: plain text is returned untouched (so clean
 * structured output is never altered), while anything carrying escape codes is
 * routed through {@link Ansi} after stripping cursor/OSC noise so colours paint
 * instead of leaking as literal `\x1b[…m` sequences.
 */
function renderMaybeAnsi(value: string) {
  if (!value.includes("\x1b")) return value;
  return <Ansi>{expandTerminalTabs(stripOscSequences(value))}</Ansi>;
}

function limitLiveOutputForRender(text: string, isLive: boolean): string {
  if (!isLive || text.length <= LIVE_OUTPUT_RENDER_LIMIT) return text;
  return `... (showing latest output)\n${text.slice(-LIVE_OUTPUT_RENDER_LIMIT)}`;
}

/**
 * Render a plain object as a key/value table. String values are rendered in a
 * `whitespace-pre-wrap` <pre> so embedded real newlines/tabs display correctly
 * instead of being collapsed or — when the whole object is JSON.stringify'd —
 * shown as literal `\n` / `\t` escape sequences. String values are also routed
 * through {@link renderMaybeAnsi} so a field carrying terminal colour codes
 * paints instead of leaking raw escapes.
 */
function RecordTable({ value }: { value: Record<string, unknown> }) {
  const entries = Object.entries(value);
  if (entries.length === 0) return null;

  return (
    <div className="divide-y divide-border/15">
      {entries.map(([key, val]) => {
        // Objects/arrays → rich collapsible JsonView (tree + auto-table for arrays
        // of flat objects). Strings/scalars keep the ANSI/newline-aware key/value
        // rendering below (important for stdout-style fields).
        if (val !== null && typeof val === "object") {
          return (
            <div key={key} className="px-3 py-2">
              <span className="text-[10px] font-mono text-[var(--ansi-cyan)]/70">{key}</span>
              <div className="mt-1">
                <JsonView value={val} />
              </div>
            </div>
          );
        }
        const isString = typeof val === "string";
        const strValue = isString ? (val as string) : String(val);
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
              <pre className="mt-1 text-[11px] font-mono text-foreground/80 whitespace-pre-wrap break-words max-h-60 overflow-auto leading-relaxed">
                {isString ? renderMaybeAnsi(strValue) : strValue}
              </pre>
            ) : (
              <span className="text-[11px] font-mono text-foreground/80 truncate" title={strValue}>
                {isString ? renderMaybeAnsi(strValue) : strValue}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}

function ToolArgsTable({ args }: { args: unknown }) {
  const normalized = normalizeToolArgs(args);
  if (normalized.kind === "raw") {
    return (
      <pre className="px-3 py-2 text-[11px] font-mono text-foreground/80 whitespace-pre-wrap break-words max-h-48 overflow-auto leading-relaxed">
        {normalized.value}
      </pre>
    );
  }

  return <RecordTable value={normalized.value} />;
}

function ToolResultDisplay({ result }: { result: unknown }) {
  if (result === null || result === undefined) return null;

  // A result delivered as a JSON string renders its nested fields with literal
  // \n / \t otherwise; parse it so e.g. a `stdout` field shows real newlines via
  // the per-field RecordTable below instead of one escaped blob.
  let value: unknown = result;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      try {
        const parsed = JSON.parse(trimmed);
        if (parsed && typeof parsed === "object") value = parsed;
      } catch {
        // Not JSON — keep the original string.
      }
    }
  }

  const isMarkdownLike =
    typeof value === "string" &&
    (/^#{1,3}\s/m.test(value) || /\*\*/.test(value) || /^[-*]\s/m.test(value) || /```/.test(value));

  if (isMarkdownLike) {
    const md = value as string;
    return (
      <div className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-[480px] overflow-auto text-[12px] text-foreground leading-[1.65] [&_p]:mb-1.5 [&_p:last-child]:mb-0">
        {/* Markdown can't render ANSI; strip it only when present so clean
            markdown is never altered. */}
        <Markdown content={md.includes("\x1b") ? stripAllAnsi(md) : md} />
      </div>
    );
  }

  // Plain objects: render each field so multi-line string values keep their real
  // newlines/tabs. Stringifying the whole object would escape them into literal
  // `\n` / `\t` sequences, which renders as unreadable noise in a <pre>.
  if (isRecord(value)) {
    return (
      <div className="space-y-2">
        <ToolAiTraceSummary value={value} />
        <div className="rounded-md bg-muted/40 border border-border/20 max-h-[480px] overflow-auto">
          <RecordTable value={value} />
        </div>
      </div>
    );
  }

  // Top-level arrays (e.g. a list result) → collapsible JsonView (auto-table for
  // arrays of flat objects).
  if (Array.isArray(value)) {
    return (
      <div className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-[480px] overflow-auto">
        <JsonView value={value} />
      </div>
    );
  }

  const text = safeStringify(value, 8000);
  return (
    <pre className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-[480px] overflow-auto text-[11px] font-mono text-foreground/80 whitespace-pre-wrap break-words leading-relaxed">
      {typeof value === "string" ? renderMaybeAnsi(text) : text}
    </pre>
  );
}

function formatShellLikeOutput(result: unknown, streamingOutput?: string): string | null {
  if (streamingOutput) return streamingOutput;

  // Results sometimes arrive as a JSON string instead of a parsed object; parse
  // so embedded stdout newlines/tabs become real whitespace rather than literal
  // \n / \t escapes in the rendered panel.
  let value: unknown = result;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed.startsWith("{")) {
      try {
        value = JSON.parse(trimmed);
      } catch {
        // Not JSON — fall through to the plain-string branch.
      }
    }
  }
  if (!value || typeof value !== "object") {
    return typeof result === "string" && result.trim() ? result : null;
  }

  const r = value as Record<string, unknown>;
  const command = typeof r.command === "string" ? r.command : null;
  const stdout =
    typeof r.stdout === "string"
      ? r.stdout
      : typeof r.partial_stdout === "string"
        ? r.partial_stdout
        : "";
  const stderr =
    typeof r.stderr === "string"
      ? r.stderr
      : typeof r.partial_stderr === "string"
        ? r.partial_stderr
        : "";
  const output = typeof r.output === "string" ? r.output : "";
  const error =
    typeof r.error === "string" ? r.error : typeof r.message === "string" ? r.message : "";
  const exitCode = typeof r.exit_code === "number" ? r.exit_code : null;

  const header: string[] = [];
  if (command) header.push(`$ ${formatCommandForDisplay(command)}`);
  if (exitCode !== null && exitCode !== 0) header.push(`[exit ${exitCode}]`);

  const body: string[] = [];
  if (stdout) body.push(stdout);
  if (!stdout && output) body.push(output);
  if (stderr) body.push(body.length > 0 ? `stderr:\n${stderr}` : stderr);
  if (error) body.push(body.length > 0 ? `error: ${error}` : error);

  return [...header, ...body].join("\n\n") || null;
}

export const TOOL_DETAIL_STATUS_BADGE_STYLES: Record<AiToolExecution["status"], string> = {
  running: "border-[var(--ansi-blue)]/45 bg-[var(--ansi-blue)]/15 text-[var(--ansi-blue)]",
  backgrounded: "border-amber-300/45 bg-amber-400/15 text-amber-300",
  completed: "bg-[var(--success-dim)] text-[var(--success)]",
  error: "bg-destructive/10 text-destructive",
  interrupted: "bg-yellow-500/10 text-yellow-400",
};

function getStatusLabel(status: AiToolExecution["status"]): string {
  switch (status) {
    case "running":
      return "Running";
    case "backgrounded":
      return "Backgrounded";
    case "completed":
      return "Completed";
    case "error":
      return "Error";
    case "interrupted":
      return "Interrupted";
  }
}

const TOOL_INTENT_SOURCE_LABELS: Record<
  NonNullable<AiToolExecution["toolIntent"]>["source"],
  string
> = {
  native_tool_call: "Native tool call",
  textual_xml: "Recovered XML text",
  textual_json: "Recovered JSON text",
  recovered: "Recovered",
};

const TOOL_INTENT_DECISION_LABELS: Record<
  NonNullable<AiToolExecution["toolIntent"]>["decision"],
  string
> = {
  allow: "Allowed",
  require_approval: "Waiting for approval",
  require_human_answer: "Waiting for user",
  reject: "Rejected",
};

export function getShellOutputForDetail(
  result: unknown,
  streamingOutput: string | undefined,
  status: AiToolExecution["status"]
): { text: string | null; pending: boolean } {
  const shellOutput = formatShellLikeOutput(result, streamingOutput);
  const cleanedShellOutput = shellOutput ? normalizeLiveToolOutput(shellOutput) : null;
  const displayShellOutput =
    cleanedShellOutput && cleanedShellOutput.length > 8000
      ? `${cleanedShellOutput.slice(0, 8000)}\n... (truncated)`
      : cleanedShellOutput;
  const pending = (status === "running" || status === "backgrounded") && !displayShellOutput;
  const terminalNoOutput =
    (status === "completed" || status === "error" || status === "interrupted") &&
    result !== null &&
    result !== undefined &&
    !displayShellOutput;
  return {
    text:
      displayShellOutput ??
      (pending ? "Waiting for output..." : terminalNoOutput ? "No output." : null),
    pending,
  };
}

function normalizeLiveToolOutput(raw: string): string {
  return expandTerminalTabs(collapseProgressBars(stripOscSequences(raw))).trim();
}

export function getLiveOutputForDetail(
  streamingOutput: string | undefined,
  status: AiToolExecution["status"]
): { text: string | null; pending: boolean } {
  const cleanedOutput = streamingOutput ? normalizeLiveToolOutput(streamingOutput) : null;
  const pending = (status === "running" || status === "backgrounded") && !cleanedOutput;
  return {
    text: cleanedOutput ?? (pending ? "Waiting for output..." : null),
    pending,
  };
}

export function stageTeamAgentRequestIdsByWorker(
  agents: ReadonlyArray<{ parentRequestId: string }>
): Record<string, string> {
  const indexed: Record<string, string> = {};
  for (const agent of agents) {
    const match = agent.parentRequestId.match(/::(?:lead|worker):([^:]+)$/);
    if (match?.[1]) indexed[match[1]] = agent.parentRequestId;
  }
  return indexed;
}

export const ToolCallDetailView = memo(function ToolCallDetailView({
  sessionId,
}: ToolCallDetailViewProps) {
  const { t } = useTranslation();
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);
  const requestIds = useStore((s) => s.sessions[sessionId]?.toolDetailRequestIds);
  const targetRequestId = requestIds?.[0] ?? null;
  const backgroundJobs = useStore((s) => s.backgroundJobs[sessionId]) ?? EMPTY_BG_JOBS;
  const activeSubAgents = useStore((s) => s.activeSubAgents[sessionId] ?? EMPTY_SUB_AGENT_LIST);
  const stageTeamAgentRequestIds = useMemo(
    () => stageTeamAgentRequestIdsByWorker(activeSubAgents),
    [activeSubAgents]
  );

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

  // A `stage_run` tool is "just a tool, with a bit more state": its live per-org
  // fan-out renders here in the standard detail pane (设计 2026-06-13-stage-run,
  // superseded the bespoke StageRunCard/StageRunView). Only attach the rows to the
  // matching tool row — when the run's requestId is known and differs, skip.
  const stageRun = useStore((s) => {
    const session = s.sessions[sessionId];
    const sr = targetRequestId
      ? (session?.stageRuns?.[targetRequestId] ?? session?.stageRun ?? null)
      : (session?.stageRun ?? null);
    if (!sr) return null;
    if (sr.requestId && targetRequestId && sr.requestId !== targetRequestId) return null;
    return sr;
  });
  const candidateReviewHint = useStore((s) => s.sessions[sessionId]?.candidateReviewHint);

  const navigateBack = () => setDetailViewMode(sessionId, "timeline");

  // Drill from a `stage_run` per-org row into that org's specialist sub-agent:
  // push its `agentRequestId` onto the detail stack and switch to the sub-agent
  // pane (same pattern as clicking a sub-agent card). `SubAgentDetailView`'s back
  // nav pops the stack back to this `stage_run` tool row (the org list).
  const handleDrillIntoOrg = useCallback(
    (agentRequestId: string) => {
      const store = useStore.getState();
      store.setToolDetailRequestIds(sessionId, [...(requestIds ?? []), agentRequestId]);
      store.setDetailViewMode(sessionId, "sub-agent-detail");
    },
    [sessionId, requestIds]
  );

  const toolColor = useMemo(
    () => (execution ? getToolColor(execution.toolName) : undefined),
    [execution]
  );
  const toolLabel = useMemo(
    () => (execution ? getToolLabel(execution.toolName, "short") : ""),
    [execution]
  );

  if (!execution) {
    // The tool execution block can lag the Details click — `stage_run` is
    // loop-routed and long-running, so its requestId / timeline block may not
    // have landed when the user clicks. Show the live per-org rows if we already
    // have them, otherwise a loading state while the block resolves — instead of
    // a bare "no tool executions" message that reads as an unresponsive button.
    const stageRunReady = Boolean(stageRun && stageRun.rows.length > 0);
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
        {stageRunReady && stageRun ? (
          <div className="flex-1 overflow-y-auto px-4 py-3">
            <div className="text-[10px] font-semibold text-muted-foreground/70 uppercase tracking-wider mb-2">
              Company Controllers
            </div>
            <StageRunOrgRows
              rows={stageRun.rows}
              summary={stageRun.summary}
              stageLabel={stageRun.stageLabel}
              roleLabel={stageRun.roleLabel}
              coverageAxis={stageRun.coverageAxis}
              agentRequestIdsByWorker={stageTeamAgentRequestIds}
              onDrillIn={handleDrillIntoOrg}
            />
          </div>
        ) : (
          <div className="flex-1 flex items-center justify-center text-sm text-muted-foreground/60">
            {targetRequestId ? (
              <span className="flex items-center gap-2">
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                {t("ai.toolDetail.loading")}
              </span>
            ) : (
              t("ai.toolDetail.noToolExecutions")
            )}
          </div>
        )}
      </div>
    );
  }

  const displayStatus =
    execution.status === "completed" && toolResultIndicatesFailure(execution.result)
      ? "error"
      : execution.status;
  const isRunning = displayStatus === "running";
  const isBackgrounded = displayStatus === "backgrounded";
  const backgroundedToolCount = isBackgrounded ? 1 : 0;
  const isError = displayStatus === "error";
  const errorMessage = (() => {
    if (!isError) return null;
    if (typeof execution.result === "string") return execution.result;
    if (typeof execution.result === "object" && execution.result !== null) {
      const r = execution.result as Record<string, unknown>;
      const e = r.error || r.message;
      if (typeof e === "string") return e;
    }
    return null;
  })();

  const isShellCmd = isShellLikeToolForDetail(execution.toolName, execution.args);
  const intent = execution.toolIntent;
  const shellOutputState = isShellCmd
    ? getShellOutputForDetail(execution.result, execution.streamingOutput, displayStatus)
    : { text: null, pending: false };
  const shellOutputText = shellOutputState.text;
  const pendingShellOutput = shellOutputState.pending;
  const displayedShellOutputText = shellOutputText
    ? limitLiveOutputForRender(shellOutputText, isRunning || isBackgrounded)
    : null;
  const liveOutputState = !isShellCmd
    ? getLiveOutputForDetail(execution.streamingOutput, displayStatus)
    : { text: null, pending: false };
  const displayedLiveOutputText = liveOutputState.text
    ? limitLiveOutputForRender(liveOutputState.text, isRunning || isBackgrounded)
    : null;
  const reportingOperationId = getReportingStageRunOperationId(
    execution.toolName,
    execution.args,
    execution.result
  );

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
            TOOL_DETAIL_STATUS_BADGE_STYLES[displayStatus]
          )}
        >
          {isRunning && <Loader2 className={DETAIL_RUNNING_SPINNER_CLASS} />}
          {isBackgrounded && <Loader2 className={DETAIL_RUNNING_SPINNER_CLASS} />}
          {getStatusLabel(displayStatus)}
        </Badge>
        {execution.durationMs != null && (
          <span className="text-[11px] text-muted-foreground/70 tabular-nums flex items-center gap-1">
            <Clock className="w-3 h-3" />
            {formatDurationLong(execution.durationMs)}
          </span>
        )}
        <div className="ml-auto flex items-center justify-end">
          <BackgroundJobsBadge
            jobs={backgroundJobs}
            fallbackCount={backgroundedToolCount}
            reserveSpace
          />
        </div>
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

        {execution.toolName === "stage_run" && stageRun && stageRun.rows.length > 0 && (
          <div className="px-4 py-3 border-b border-border/20">
            <div className="text-[10px] font-semibold text-muted-foreground/70 uppercase tracking-wider mb-2">
              Company Controllers
            </div>
            <StageRunOrgRows
              rows={stageRun.rows}
              summary={stageRun.summary}
              stageLabel={stageRun.stageLabel}
              roleLabel={stageRun.roleLabel}
              coverageAxis={stageRun.coverageAxis}
              isActive={isRunning}
              agentRequestIdsByWorker={stageTeamAgentRequestIds}
              onDrillIn={handleDrillIntoOrg}
            />
          </div>
        )}

        {candidateReviewHint && isAttackCandidateStageRun(execution.toolName, execution.args) && (
          <div className="space-y-3 border-b border-border/20 px-4 py-3">
            <AttackCandidateReview
              operationId={candidateReviewHint.operationId}
              waveRunId={candidateReviewHint.waveRunId}
              refreshVersion={candidateReviewHint.refreshVersion}
            />
            <CandidateAttemptRows
              operationId={candidateReviewHint.operationId}
              waveRunId={candidateReviewHint.waveRunId}
              refreshVersion={candidateReviewHint.refreshVersion}
            />
          </div>
        )}

        {stageRun && isCleanupStageRun(execution.toolName, execution.args) && (
          <div className="space-y-3 border-b border-border/20 px-4 py-3">
            {stageRun.rows.map((row) =>
              row.operationId ? (
                <CleanupObligationList
                  key={`${row.operationId}:${row.id}`}
                  operationId={row.operationId}
                  organizationIdAtTime={row.id}
                />
              ) : null
            )}
          </div>
        )}

        {reportingOperationId && (
          <div className="border-b border-border/20 px-4 py-3">
            <ReportReadModelView operationId={reportingOperationId} />
          </div>
        )}

        {intent && (
          <div className="px-4 py-3 border-b border-border/20">
            <div className="text-[10px] font-semibold text-muted-foreground/70 uppercase tracking-wider mb-2">
              Intent
            </div>
            <div className="rounded-md bg-muted/30 border border-border/20 divide-y divide-border/15">
              <div className="px-3 py-1.5 flex items-baseline gap-3">
                <span className="w-28 text-[10px] font-mono text-[var(--ansi-cyan)]/70 flex-shrink-0">
                  Model wanted
                </span>
                <span className="text-[11px] font-mono text-foreground/80 truncate">
                  {intent.modelWanted}
                </span>
              </div>
              <div className="px-3 py-1.5 flex items-baseline gap-3">
                <span className="w-28 text-[10px] font-mono text-[var(--ansi-cyan)]/70 flex-shrink-0">
                  Source
                </span>
                <span className="text-[11px] text-foreground/80">
                  {TOOL_INTENT_SOURCE_LABELS[intent.source]}
                </span>
              </div>
              <div className="px-3 py-1.5 flex items-baseline gap-3">
                <span className="w-28 text-[10px] font-mono text-[var(--ansi-cyan)]/70 flex-shrink-0">
                  Golish decision
                </span>
                <span className="text-[11px] text-foreground/80">
                  {TOOL_INTENT_DECISION_LABELS[intent.decision]}
                </span>
              </div>
              {intent.reason && (
                <div className="px-3 py-2">
                  <div className="text-[10px] font-mono text-[var(--ansi-cyan)]/70 mb-1">
                    Reason
                  </div>
                  <p className="text-[11px] text-muted-foreground/80 leading-relaxed">
                    {intent.reason}
                  </p>
                </div>
              )}
            </div>
          </div>
        )}

        {hasToolArgs(execution.args) && (
          <div className="px-4 py-3 border-b border-border/20">
            <div className="text-[10px] font-semibold text-muted-foreground/70 uppercase tracking-wider mb-2">
              Input
            </div>
            <div className="rounded-md bg-muted/40 border border-border/20 overflow-hidden">
              <ToolArgsTable args={execution.args} />
            </div>
          </div>
        )}

        {isShellCmd && displayedShellOutputText && (
          <div className="px-4 py-3 border-b border-border/20">
            <div className="flex items-center gap-1.5 mb-2">
              {(pendingShellOutput || isRunning || isBackgrounded) && (
                <Loader2 className={DETAIL_PENDING_OUTPUT_SPINNER_CLASS} />
              )}
              <span className="text-[10px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                Output
              </span>
            </div>
            <pre className="ansi-output max-h-[480px] overflow-auto whitespace-pre-wrap rounded border border-border/15 bg-background/40 px-3 py-2 text-[11px] font-mono text-muted-foreground">
              <Ansi>{displayedShellOutputText}</Ansi>
            </pre>
          </div>
        )}

        {!isShellCmd && (isRunning || isBackgrounded) && displayedLiveOutputText && (
          <div className="px-4 py-3 border-b border-border/20">
            <div className="flex items-center gap-1.5 mb-2">
              <Loader2 className={DETAIL_PENDING_OUTPUT_SPINNER_CLASS} />
              <span className="text-[10px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                Output
              </span>
            </div>
            <pre className="ansi-output max-h-[480px] overflow-auto whitespace-pre-wrap rounded border border-border/15 bg-background/40 px-3 py-2 text-[11px] font-mono text-muted-foreground">
              <Ansi>{displayedLiveOutputText}</Ansi>
            </pre>
          </div>
        )}

        {!isShellCmd &&
          !isRunning &&
          !isBackgrounded &&
          execution.result !== undefined &&
          execution.result !== null && (
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

      {(isRunning || isBackgrounded) && (
        <div
          className={cn(
            "px-3 py-2 border-t border-[var(--border-subtle)] flex items-center gap-2 flex-shrink-0",
            isBackgrounded ? "bg-amber-400/10" : "bg-[var(--ansi-blue)]/10"
          )}
        >
          <Loader2
            className={cn(
              DETAIL_RUNNING_SPINNER_CLASS,
              isBackgrounded ? "text-amber-300" : "text-[var(--ansi-blue)]"
            )}
          />
          <span
            className={cn(
              "text-[11px]",
              isBackgrounded ? "text-amber-300" : "text-[var(--ansi-blue)]"
            )}
          >
            {isBackgrounded ? "Running in background" : t("ai.toolDetail.running")}
          </span>
        </div>
      )}
    </div>
  );
});
