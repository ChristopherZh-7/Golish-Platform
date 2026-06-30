/**
 * SubAgentDetailView
 *
 * 以时间线（对话流）样式展示单个 sub-agent 的完整执行过程：
 * 文字输出和工具调用按时间顺序交错显示，类似右侧 ChatPanel 的 primary agent 消息流。
 */
import {
  AlertTriangle,
  ArrowLeft,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  Copy,
  Loader2,
  Terminal,
  Wand2,
  Wrench,
  XCircle,
} from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState, type WheelEvent } from "react";
import { useTranslation } from "react-i18next";
import { ThinkingBlock } from "@/components/AIChatPanel/ThinkingBlock";
import { Ansi } from "@/components/Ansi";
import {
  StageAssetCoverageBlock,
  type StageAssetCoverageWorkItem,
} from "@/components/Engagement/StageAssetCoveragePanel";
import { JsonView } from "@/components/JsonView/JsonView";
import { Markdown } from "@/components/Markdown";
import { ToolAiTraceSummary } from "@/components/ToolAiTraceSummary";
import { BackgroundJobsBadge } from "@/components/UnifiedInput/StatusBadges";
import { AnchorChip } from "@/components/ui/AnchorChip";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { StatusIcon } from "@/components/ui/StatusIcon";
import {
  collapseProgressBars,
  expandTerminalTabs,
  stripAllAnsi,
  stripOscSequences,
} from "@/lib/ansi";
import { copyToClipboard } from "@/lib/clipboard";
import { shouldStickToBottomAfterScroll } from "@/lib/scroll-stickiness";
import { getAgentColor, getAgentIcon } from "@/lib/sub-agent-theme";
import { safeStringify } from "@/lib/text";
import { formatDurationShort } from "@/lib/time";
import {
  getPentestRunInputLines,
  getToolActionLabel,
  getToolPrimaryArg,
  toolResultIndicatesFailure,
} from "@/lib/tools";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, SubAgentEntry, SubAgentToolCall } from "@/store";
import { useStore } from "@/store";
import type { SessionStageRun } from "@/store/types/session";

export const SUB_AGENT_DETAIL_RUNNING_SPINNER_CLASS = "h-4 w-4 shrink-0 animate-spin";
export const SUB_AGENT_DETAIL_PENDING_OUTPUT_SPINNER_CLASS =
  "h-4 w-4 shrink-0 animate-spin text-[var(--ansi-blue)]/80";
const SUB_AGENT_DETAIL_NARRATIVE_BLOCK_CLASS = "bg-[var(--bg-hover)]/25 px-4 py-2.5";
const SUB_AGENT_DETAIL_NARRATIVE_COMPACT_TOP_CLASS = "bg-[var(--bg-hover)]/25 px-4 pb-2.5 pt-0.5";

type ShellOutputField = { key: string; value: string };
type SubAgentToolStatus = SubAgentToolCall["status"];
type DetailToolStatus = SubAgentToolStatus | "interrupted";
type SubAgentHeaderStatus = ActiveSubAgent["status"] | "backgrounded";
type SubAgentToolCallVisualRelation = "after_narrative" | "stacked" | "standalone";
type SubAgentDetailTab = "run" | "coverage";

interface DisplayStatusOptions {
  parentStageStopped?: boolean;
}

interface StageRefinerDirectiveSummary {
  rootCause: string;
  stageLabel: string | null;
  repairKindLabel: string | null;
  gapCount: number | null;
  actionCount: number | null;
  allowedTools: string[];
  forbiddenTools: string[];
  batchFirst: boolean;
}

export function SubAgentShellOutputText({ text }: { text: string }) {
  return <Ansi>{text}</Ansi>;
}

function normalizeSubAgentShellText(raw: string): string {
  return expandTerminalTabs(collapseProgressBars(stripOscSequences(raw))).trim();
}

function normalizeSubAgentShellFieldText(raw: string): string {
  return collapseProgressBars(stripAllAnsi(raw)).trim();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isLiveToolStatus(status: string | undefined): boolean {
  return status === "running" || status === "backgrounded";
}

const TARGET_ARG_KEYS = [
  "target",
  "target_url",
  "targetUrl",
  "url",
  "base_url",
  "baseUrl",
  "host",
  "hostname",
  "domain",
  "ip",
  "address",
  "asset",
  "asset_value",
  "assetValue",
  "value",
];

const COMMAND_ARG_KEYS = ["command", "args", "query"];

function cleanAssetSubject(subject: string): string {
  return subject
    .trim()
    .replace(/^['"`]+|['"`),;]+$/g, "")
    .replace(/[),;]+$/g, "");
}

function uniqueAssetSubjects(subjects: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const subject of subjects) {
    const cleaned = cleanAssetSubject(subject);
    if (!cleaned) continue;
    const key = cleaned.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(cleaned);
  }
  return out;
}

export function extractAssetSubjectsFromText(text: string): string[] {
  const normalized = text.replace(/\\n/g, " ").replace(/\s+/g, " ").trim();
  if (!normalized) return [];

  const subjects: string[] = [];
  const withoutUrls = normalized.replace(/https?:\/\/[^\s"'<>]+/gi, (match) => {
    subjects.push(match);
    return " ";
  });

  for (const match of withoutUrls.matchAll(
    /\b(?:\d{1,3}\.){3}\d{1,3}(?::\d{1,5})?(?:\/\d{1,2})?\b/g
  )) {
    subjects.push(match[0]);
  }

  for (const match of withoutUrls.matchAll(
    /\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,}(?::\d{1,5})?\b/gi
  )) {
    subjects.push(match[0]);
  }

  return uniqueAssetSubjects(subjects);
}

export function extractAssetSubjectFromText(text: string): string | null {
  return extractAssetSubjectsFromText(text)[0] ?? null;
}

function firstStringArg(args: Record<string, unknown>, keys: readonly string[]): string | null {
  for (const key of keys) {
    const value = args[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

function nestedArgsRecord(args: Record<string, unknown>): Record<string, unknown> | null {
  const nested = args.args;
  return isRecord(nested) ? nested : null;
}

function displayToolNameForAssetWork(tool: Pick<SubAgentToolCall, "name" | "args">): string {
  const wrappedName = tool.args.tool_name;
  return typeof wrappedName === "string" && wrappedName.trim() ? wrappedName.trim() : tool.name;
}

function assetSubjectsFromCandidate(value: string): string[] {
  const parsed = extractAssetSubjectsFromText(value);
  return parsed.length > 0 ? parsed : [cleanAssetSubject(value)];
}

function batchAssetSubjectsFromToolArgs(args: Record<string, unknown>): string[] {
  return uniqueAssetSubjects(getPentestRunInputLines(args).flatMap(assetSubjectsFromCandidate));
}

export function extractAssetSubjectsFromToolCall(
  tool: Pick<SubAgentToolCall, "name" | "args">
): string[] {
  const directTarget = firstStringArg(tool.args, TARGET_ARG_KEYS);
  if (directTarget) return assetSubjectsFromCandidate(directTarget);

  const nested = nestedArgsRecord(tool.args);
  const nestedTarget = nested ? firstStringArg(nested, TARGET_ARG_KEYS) : null;
  if (nestedTarget) return assetSubjectsFromCandidate(nestedTarget);

  const batchSubjects = batchAssetSubjectsFromToolArgs(tool.args);
  if (batchSubjects.length > 0) return batchSubjects;

  const commandLike = [
    firstStringArg(tool.args, COMMAND_ARG_KEYS),
    nested ? firstStringArg(nested, COMMAND_ARG_KEYS) : null,
    getToolPrimaryArg(tool.name, tool.args),
  ].find((value): value is string => Boolean(value));

  return commandLike ? extractAssetSubjectsFromText(commandLike) : [];
}

export function extractAssetSubjectFromToolCall(
  tool: Pick<SubAgentToolCall, "name" | "args">
): string | null {
  return extractAssetSubjectsFromToolCall(tool)[0] ?? null;
}

function latestOutputPreview(
  tool: Pick<SubAgentToolCall, "streamingOutput" | "result">
): string | null {
  const raw =
    tool.streamingOutput ??
    (isRecord(tool.result) && typeof tool.result.partial_stdout === "string"
      ? tool.result.partial_stdout
      : null);
  if (!raw) return null;
  const lines = stripAllAnsi(raw)
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  const latest = lines[lines.length - 1];
  return latest ? latest.slice(0, 160) : null;
}

function uniqueTechniqueLabels(labels: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const label of labels) {
    const normalized = label.toUpperCase();
    if (!normalized || seen.has(normalized)) continue;
    seen.add(normalized);
    out.push(normalized);
  }
  return out;
}

export function inferCoverageTechniquesFromToolCall(
  tool: Pick<SubAgentToolCall, "name" | "args">
): string[] {
  const displayName = displayToolNameForAssetWork(tool).toLowerCase();
  const action = getToolActionLabel(tool.name, tool.args).toLowerCase();
  const primary = (getToolPrimaryArg(tool.name, tool.args) ?? "").toLowerCase();
  const rawArgs = typeof tool.args.args === "string" ? tool.args.args.toLowerCase() : "";
  const text = `${tool.name} ${displayName} ${action} ${primary} ${rawArgs}`;
  const techniques: string[] = [];

  if (/\b(httpx|curl|wget|gowitness)\b/.test(text) || text.includes("liveness")) {
    techniques.push("LIVENESS");
  }
  if (/\b(naabu|masscan|nmap)\b/.test(text) || text.includes("port")) {
    techniques.push("PORT");
  }
  const serviceFingerprintIntent =
    action.includes("probing services") || action.includes("fingerprinting");
  if (
    /\b(whatweb)\b/.test(text) ||
    text.includes("-sv") ||
    text.includes("-s v") ||
    text.includes("fingerprint") ||
    serviceFingerprintIntent
  ) {
    techniques.push("SERVICE");
  }

  return uniqueTechniqueLabels(techniques);
}

export function summarizeSubAgentAssetWork(
  tools: readonly SubAgentToolCall[],
  options: DisplayStatusOptions = {}
): StageAssetCoverageWorkItem[] {
  return tools
    .map((tool): StageAssetCoverageWorkItem => {
      const status = getSubAgentToolDisplayStatus(tool, options);
      const primary = getToolPrimaryArg(tool.name, tool.args);
      const subjects = extractAssetSubjectsFromToolCall(tool);
      return {
        id: tool.id,
        displayToolName: displayToolNameForAssetWork(tool),
        rawToolName: tool.name,
        subject: subjects[0] ?? null,
        subjects,
        primary,
        techniques: inferCoverageTechniquesFromToolCall(tool),
        status,
        startedAt: tool.startedAt,
        completedAt: tool.completedAt,
        outputPreview: latestOutputPreview(tool) ?? undefined,
      };
    })
    .filter((item) => item.subject || item.primary);
}

export function getSubAgentToolDisplayStatus(
  tool: Pick<SubAgentToolCall, "status" | "result">,
  options: DisplayStatusOptions = {}
): DetailToolStatus {
  const status: DetailToolStatus =
    (tool.status as string) === "completed"
      ? "completed"
      : (tool.status as string) === "error"
        ? "error"
        : (tool.status as string) === "interrupted"
          ? "interrupted"
          : (tool.status as string) === "backgrounded"
            ? "backgrounded"
            : "running";

  if (options.parentStageStopped && status === "running") return "interrupted";
  return status === "completed" && toolResultIndicatesFailure(tool.result) ? "error" : status;
}

export function getSubAgentHeaderDisplayStatus(
  agent: Pick<ActiveSubAgent, "status" | "toolCalls">,
  options: DisplayStatusOptions = {}
): SubAgentHeaderStatus {
  const toolStatuses = agent.toolCalls.map((tool) => getSubAgentToolDisplayStatus(tool, options));
  if (toolStatuses.some((status) => status === "running")) {
    return "running";
  }
  if (toolStatuses.some((status) => status === "backgrounded")) {
    return "backgrounded";
  }
  if (agent.status === "completed" && agent.toolCalls.length > 0) {
    const latestToolStatus = toolStatuses[toolStatuses.length - 1];
    if (latestToolStatus === "error") return "error";
  }
  if (
    options.parentStageStopped &&
    (agent.status === "running" || toolStatuses.some((status) => status === "interrupted"))
  ) {
    return "interrupted";
  }
  return agent.status;
}

export function isSubAgentShellLikeOutputTool(
  tool: Pick<SubAgentToolCall, "name" | "args">
): boolean {
  if (tool.name === "run_pty_cmd" || tool.name === "run_command" || tool.name === "pentest_run") {
    return true;
  }
  return (
    isRecord(tool.args) &&
    typeof tool.args.tool_name === "string" &&
    (tool.args.background === true ||
      typeof tool.args.args === "string" ||
      typeof tool.args.timeout_secs === "number")
  );
}

const DSML_BAR = "[|｜]";
const DSML_TAG_PREFIX = String.raw`<\s*/?\s*(?:${DSML_BAR}\s*)+DSML\s*(?:${DSML_BAR}\s*)+/?\s*`;

function dsmlBlockRegex(tagPattern: string): RegExp {
  return new RegExp(
    `${DSML_TAG_PREFIX}${tagPattern}\\b[^>]*>[\\s\\S]*?(?:${DSML_TAG_PREFIX}${tagPattern}\\b[^>]*>|$)`,
    "gi"
  );
}

function stripDsmlToolCallMarkup(text: string): string {
  return text
    .replace(dsmlBlockRegex("tool_calls?"), "")
    .replace(dsmlBlockRegex("invoke"), "")
    .replace(dsmlBlockRegex("parameter"), "")
    .replace(new RegExp(`${DSML_TAG_PREFIX}(?:tool_calls?|invoke|parameter)\\b[^>]*>`, "gi"), "");
}

export function stripAgentXmlTags(text: string): string {
  return stripAllAnsi(
    stripDsmlToolCallMarkup(text)
      .replace(
        /<\/?(task_assignment|original_request|execution_plan|execution_context|prior_knowledge)>/gi,
        ""
      )
      // Anthropic-style tool-call markup (DeepSeek V4 leaks this into agent text
      // when its native tool-call channel degrades): strip the full wrapper +
      // invoke blocks first (complete or unterminated/streaming), then any
      // leftover tags below.
      .replace(/<tool_calls\b[^>]*>[\s\S]*?(?:<\/tool_calls>|$)/g, "")
      .replace(/<invoke\b[^>]*>[\s\S]*?(?:<\/invoke>|$)/g, "")
      // MiMo / GLM `<function=...>` dialect.
      .replace(/<tool_call\b[^>]*>[\s\S]*?<\/tool_call>/g, "")
      .replace(/<function=[^>]*>[\s\S]*?(?:<\/function>|$)/g, "")
      // `<parameter=key>` (MiMo) and `<parameter name="key">` (Anthropic) values.
      .replace(/<parameter[=\s][^>]*>[\s\S]*?(?:<\/parameter>|$)/g, "")
      .replace(/<\/?tool_calls?\b[^>]*>/g, "")
      .replace(/<\/?(?:function|parameter|invoke)[^>]*>/g, "")
  ).trim();
}

function humanizeDirectiveToken(value: string): string {
  const spaced = value
    .replace(/_/g, " ")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .trim();
  if (!spaced) return "";
  return spaced
    .toLowerCase()
    .replace(/\b[a-z]/g, (match) => match.toUpperCase())
    .replace(/\bEas\b/g, "EAS")
    .replace(/\bDb\b/g, "DB");
}

function parseDirectiveList(raw: string | undefined): string[] {
  if (!raw) return [];
  return raw
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

export function parseStageRefinerDirectiveSummary(
  text: string
): StageRefinerDirectiveSummary | null {
  const cleaned = stripAgentXmlTags(text);
  const markerIndex = cleaned.indexOf("STAGE REFINER DIRECTIVE");
  if (markerIndex < 0) return null;

  const directive = cleaned.slice(markerIndex);
  const rootCause =
    directive.match(/STAGE REFINER DIRECTIVE\s*(?:\([^)]+\))?:\s*([^\n]+)/)?.[1]?.trim() ??
    "Deterministic repair directive";
  const stageMatch = directive.match(/Stage:\s*([^.]+)\.\s*Repair kind:\s*([^.]+)\./);
  const gapCountText = directive.match(
    /deterministic gate found\s+(\d+)\s+non-terminal coverage gap action\(s\)/i
  )?.[1];
  const allowedTools = parseDirectiveList(
    directive.match(/Allowed next tools:\s*\[([^\]]*)\]/)?.[1]
  );
  const forbiddenTools = parseDirectiveList(
    directive.match(/Forbidden in this repair:\s*\[([^\]]*)\]/)?.[1]
  );
  const actionCount = (directive.match(/^\s*\d+\.\s+/gm) ?? []).length;

  return {
    rootCause,
    stageLabel: stageMatch?.[1] ? humanizeDirectiveToken(stageMatch[1]) : null,
    repairKindLabel: stageMatch?.[2] ? humanizeDirectiveToken(stageMatch[2]) : null,
    gapCount: gapCountText ? Number(gapCountText) : null,
    actionCount: actionCount > 0 ? actionCount : null,
    allowedTools,
    forbiddenTools,
    batchFirst: /batch-first/i.test(directive),
  };
}

function compactToolName(tool: string): string {
  return tool.replace(/_/g, " ");
}

/* ─── Sub-agent text output block ─── */

const StageRefinerDirectiveBlock = memo(function StageRefinerDirectiveBlock({
  text,
  summary,
  streaming = false,
}: {
  text: string;
  summary: StageRefinerDirectiveSummary;
  streaming?: boolean;
}) {
  const [isExpanded, setIsExpanded] = useState(false);
  const allowedPreview = summary.allowedTools.slice(0, 4);
  const hiddenAllowed = Math.max(0, summary.allowedTools.length - allowedPreview.length);
  const workCountLabel =
    summary.gapCount != null
      ? `${summary.gapCount} gaps`
      : summary.actionCount != null
        ? `${summary.actionCount} actions`
        : null;

  return (
    <Collapsible
      open={isExpanded}
      onOpenChange={setIsExpanded}
      className="mx-4 my-2 overflow-hidden rounded-md border border-amber-300/20 border-l-2 border-l-amber-300/70 bg-amber-400/[0.055]"
    >
      <div className="px-3 py-2.5">
        <div className="flex min-w-0 items-start gap-2">
          <div className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded bg-amber-300/12 text-amber-300">
            {streaming ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <Wrench className="h-3 w-3" />
            )}
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 flex-wrap items-center gap-1.5">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-amber-200/90">
                Stage Refiner
              </span>
              {summary.repairKindLabel && (
                <span className="rounded border border-amber-300/20 bg-amber-300/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-100">
                  {summary.repairKindLabel}
                </span>
              )}
              {workCountLabel && (
                <span className="rounded border border-border/25 bg-background/35 px-1.5 py-0.5 text-[10px] tabular-nums text-foreground/75">
                  {workCountLabel}
                </span>
              )}
              {summary.batchFirst && (
                <span className="rounded border border-[var(--ansi-blue)]/25 bg-[var(--ansi-blue)]/10 px-1.5 py-0.5 text-[10px] text-[var(--ansi-blue)]">
                  Batch-first
                </span>
              )}
            </div>
            <div className="mt-1 min-w-0 text-[12.5px] font-medium text-foreground/90">
              {summary.stageLabel ?? "Harness repair directive"}
            </div>
            <div className="mt-0.5 line-clamp-2 text-[11.5px] leading-relaxed text-muted-foreground/85">
              {summary.rootCause}
            </div>
            {summary.allowedTools.length > 0 && (
              <div className="mt-2 flex min-w-0 flex-wrap items-center gap-1.5">
                <span className="text-[10px] font-medium text-muted-foreground/65">Allowed</span>
                {allowedPreview.map((tool) => (
                  <span
                    key={tool}
                    className="rounded bg-background/45 px-1.5 py-0.5 font-mono text-[10px] text-foreground/70"
                  >
                    {compactToolName(tool)}
                  </span>
                ))}
                {hiddenAllowed > 0 && (
                  <span className="text-[10px] text-muted-foreground/65">+{hiddenAllowed}</span>
                )}
                {summary.forbiddenTools.length > 0 && (
                  <span className="ml-1 inline-flex items-center gap-1 text-[10px] text-muted-foreground/60">
                    <AlertTriangle className="h-2.5 w-2.5 text-amber-300/70" />
                    {summary.forbiddenTools.length} blocked
                  </span>
                )}
              </div>
            )}
          </div>
          <CollapsibleTrigger className="mt-0.5 inline-flex h-6 shrink-0 items-center gap-1 rounded px-1.5 text-[10px] text-muted-foreground/75 transition-colors hover:bg-foreground/5 hover:text-foreground/90">
            {isExpanded ? (
              <ChevronDown className="h-3 w-3" />
            ) : (
              <ChevronRight className="h-3 w-3" />
            )}
            Details
          </CollapsibleTrigger>
        </div>
      </div>
      <CollapsibleContent className="border-t border-amber-300/15 bg-background/25 px-3 py-2.5">
        <div className="max-h-72 overflow-auto break-words text-[11.5px] leading-[1.65] text-foreground/80 [overflow-wrap:anywhere] [&_code]:break-words [&_p]:mb-2 [&_p:last-child]:mb-0 [&_pre]:my-2 [&_pre]:max-w-full [&_pre]:overflow-x-auto [&_ul]:my-1.5">
          <Markdown content={text} streaming={streaming} />
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
});

const AgentOutputBlock = memo(function AgentOutputBlock({
  compactTop = false,
  text,
  streaming = false,
}: {
  compactTop?: boolean;
  text: string;
  streaming?: boolean;
}) {
  const cleaned = stripAgentXmlTags(text);
  if (!cleaned) return null;
  const refinerDirective = parseStageRefinerDirectiveSummary(cleaned);
  if (refinerDirective) {
    return (
      <StageRefinerDirectiveBlock text={cleaned} summary={refinerDirective} streaming={streaming} />
    );
  }
  return (
    <div
      className={
        compactTop
          ? SUB_AGENT_DETAIL_NARRATIVE_COMPACT_TOP_CLASS
          : SUB_AGENT_DETAIL_NARRATIVE_BLOCK_CLASS
      }
    >
      <div className="overflow-hidden break-words text-[12.5px] leading-[1.65] text-foreground/90 [overflow-wrap:anywhere] [&_blockquote]:my-2 [&_code]:break-words [&_h1]:!text-foreground [&_h1]:mt-3 [&_h2]:!text-foreground/90 [&_h2]:mt-2.5 [&_h3]:!text-foreground/85 [&_h3]:mt-2 [&_ol]:my-1.5 [&_p]:mb-2 [&_p:last-child]:mb-0 [&_pre]:my-2 [&_pre]:max-w-full [&_pre]:overflow-x-auto [&_table]:my-2 [&_table]:block [&_table]:max-w-full [&_table]:overflow-x-auto [&_td]:break-words [&_th]:break-words [&_ul]:my-1.5">
        <Markdown content={cleaned} streaming={streaming} />
      </div>
    </div>
  );
});

function comparableEntryText(text: string | undefined): string {
  return (text ?? "").replace(/\s+/g, " ").trim();
}

function isCoveredByLaterTextEntry(entries: readonly SubAgentEntry[], index: number): boolean {
  const current = comparableEntryText(entries[index].text);
  if (!current) return false;

  for (let i = index + 1; i < entries.length; i++) {
    const next = entries[i];
    if (next.kind === "tool_call") return false;
    if (next.kind !== "text") continue;

    const later = comparableEntryText(next.text);
    if (later.length > current.length && later.startsWith(current)) return true;
  }

  return false;
}

export function normalizeSubAgentEntriesForDetail(
  entries: readonly SubAgentEntry[]
): SubAgentEntry[] {
  return entries.filter((entry, index) => {
    if (entry.kind !== "text") return true;
    return !isCoveredByLaterTextEntry(entries, index);
  });
}

export function shouldSeparateSubAgentDetailEntries(
  previous: Pick<SubAgentEntry, "kind"> | null | undefined,
  current: Pick<SubAgentEntry, "kind">
): boolean {
  if (!previous) return false;
  return previous.kind === "tool_call" && current.kind !== "tool_call";
}

export function getSubAgentToolCallVisualRelation(
  previous: Pick<SubAgentEntry, "kind"> | null | undefined
): SubAgentToolCallVisualRelation {
  if (previous?.kind === "thinking" || previous?.kind === "text") return "after_narrative";
  if (previous?.kind === "tool_call") return "stacked";
  return "standalone";
}

/* ─── Structured args display ─── */

function ToolArgsTable({ args }: { args: Record<string, unknown> }) {
  if (Object.keys(args).length === 0) return null;
  return <JsonView value={args} className="px-3 py-2" />;
}

export function getSubAgentShellOutputForDetail(
  tool: Pick<SubAgentToolCall, "result" | "streamingOutput"> & { status?: DetailToolStatus }
): {
  text: string | null;
  pending: boolean;
} {
  let raw: string | null = null;
  if (tool.streamingOutput) {
    raw = tool.streamingOutput;
  } else if (tool.result && typeof tool.result === "object") {
    const r = tool.result as Record<string, unknown>;
    const parts: string[] = [];
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
    if (stdout.trim()) parts.push(stdout.trim());
    if (typeof r.output === "string" && r.output.trim() && parts.length === 0) {
      parts.push(r.output.trim());
    }
    if (stderr) {
      const stderrText = stderr.trim();
      if (stderrText) parts.push(parts.length > 0 ? `stderr:\n${stderrText}` : stderrText);
    }
    raw = parts.join("\n\n") || null;
  }

  const text = raw ? normalizeSubAgentShellText(raw) : null;
  const pending = isLiveToolStatus(tool.status) && !text;
  const terminalNoOutput =
    (tool.status === "completed" || tool.status === "error" || tool.status === "interrupted") &&
    tool.result !== null &&
    tool.result !== undefined &&
    !text;
  return {
    text: text ?? (pending ? "Waiting for output..." : terminalNoOutput ? "No output." : null),
    pending,
  };
}

export function getSubAgentLiveOutputForDetail(
  tool: Pick<SubAgentToolCall, "streamingOutput"> & { status?: string }
): {
  text: string | null;
  pending: boolean;
} {
  const text = tool.streamingOutput ? normalizeSubAgentShellText(tool.streamingOutput) : null;
  const pending = isLiveToolStatus(tool.status) && !text;
  return {
    text: text ?? (pending ? "Waiting for output..." : null),
    pending,
  };
}

export function getSubAgentShellOutputFieldsForDetail(
  tool: Pick<SubAgentToolCall, "result">
): ShellOutputField[] {
  if (!tool.result || typeof tool.result !== "object" || Array.isArray(tool.result)) return [];

  const r = tool.result as Record<string, unknown>;
  const fields: ShellOutputField[] = [];
  const pushString = (key: string, value: unknown) => {
    if (typeof value !== "string") return;
    const text = normalizeSubAgentShellFieldText(value);
    if (text) fields.push({ key, value: text });
  };

  pushString("stdout", r.stdout);
  pushString("output", r.output);
  pushString("stderr", r.stderr);
  pushString("error", r.error ?? r.message);

  if (typeof r.exit_code === "number") {
    fields.push({ key: "exit_code", value: String(r.exit_code) });
  }

  return fields;
}

export function getSubAgentShellOutputJsonValueForDetail(
  tool: Pick<SubAgentToolCall, "result"> & { status?: DetailToolStatus }
): Record<string, string> | null {
  if (isLiveToolStatus(tool.status)) return null;
  const fields = getSubAgentShellOutputFieldsForDetail(tool);
  if (fields.length === 0) return null;

  return Object.fromEntries(fields.map((field) => [field.key, field.value]));
}

/* ─── Tool result display ─── */

const ToolResultDisplay = memo(function ToolResultDisplay({ result }: { result: unknown }) {
  const isMarkdownLike =
    typeof result === "string" &&
    (/^#{1,3}\s/m.test(result) ||
      /\*\*/.test(result) ||
      /^[-*]\s/m.test(result) ||
      /```/.test(result));

  if (isMarkdownLike) {
    return (
      <div className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-64 overflow-auto text-[12px] text-foreground leading-[1.65] [&_h1]:!text-foreground [&_h2]:!text-foreground/90 [&_h3]:!text-foreground/85 [&_p]:mb-1.5 [&_p:last-child]:mb-0">
        <Markdown content={result as string} />
      </div>
    );
  }

  // Structured result (object/array, or a JSON string) → collapsible JsonView;
  // plain text stays a <pre>.
  let structured: unknown;
  if (result !== null && typeof result === "object") {
    structured = result;
  } else if (typeof result === "string") {
    const trimmed = result.trim();
    const looksJson =
      (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
      (trimmed.startsWith("[") && trimmed.endsWith("]"));
    if (looksJson) {
      try {
        structured = JSON.parse(trimmed);
      } catch {
        structured = undefined;
      }
    }
  }
  if (structured !== undefined) {
    return (
      <div className="space-y-2">
        <ToolAiTraceSummary value={structured} />
        <div className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-64 overflow-auto">
          <JsonView value={structured} />
        </div>
      </div>
    );
  }

  return (
    <pre className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-64 overflow-auto text-[11px] font-mono text-foreground/80 whitespace-pre-wrap break-words leading-relaxed">
      {safeStringify(result, 5000)}
    </pre>
  );
});

/* ─── Sub-agent tool call block ─── */

const AgentToolCallBlock = memo(function AgentToolCallBlock({
  tool,
  parentStageStopped = false,
  visualRelation = "standalone",
}: {
  tool: SubAgentToolCall;
  parentStageStopped?: boolean;
  visualRelation?: SubAgentToolCallVisualRelation;
}) {
  const isShellRunner = tool.name === "run_pty_cmd" || tool.name === "run_command";
  const isShellLikeOutput = isSubAgentShellLikeOutputTool(tool);
  const [isExpanded, setIsExpanded] = useState(false);
  const preRef = useRef<HTMLPreElement>(null);
  const preScrollFrameRef = useRef<number | null>(null);
  const status = getSubAgentToolDisplayStatus(tool, { parentStageStopped });
  const displayTool = { ...tool, status };
  const isLive = isLiveToolStatus(status);
  const isStreaming = isShellLikeOutput && isLive && !!tool.streamingOutput;

  useEffect(() => {
    if (isStreaming && preRef.current) {
      if (preScrollFrameRef.current != null) cancelAnimationFrame(preScrollFrameRef.current);
      preScrollFrameRef.current = requestAnimationFrame(() => {
        preScrollFrameRef.current = null;
        const el = preRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });
    }
    return () => {
      if (preScrollFrameRef.current != null) {
        cancelAnimationFrame(preScrollFrameRef.current);
        preScrollFrameRef.current = null;
      }
    };
  }, [isStreaming, tool.streamingOutput]);

  // Reuse the shared primary-arg formatter (same as the main timeline cards) so
  // every collapsed row surfaces a one-line summary. In particular pentest_run
  // nests the real tool under `tool_name`/`args`, so this renders e.g.
  // "nmap -sV target" inline without the user expanding the row.
  const summaryArg = getToolPrimaryArg(tool.name, tool.args);
  const actionLabel = getToolActionLabel(tool.name, tool.args);
  const shellOutputState = getSubAgentShellOutputForDetail(displayTool);
  const shellOutputJsonValue = getSubAgentShellOutputJsonValueForDetail(displayTool);
  const liveOutputState = !isShellLikeOutput
    ? getSubAgentLiveOutputForDetail(displayTool)
    : { text: null, pending: false };
  const displayedLiveOutputText = liveOutputState.text
    ? limitLiveOutputForRender(liveOutputState.text, isLive)
    : null;
  const isAttachedToNarrative = visualRelation === "after_narrative";
  const isStackedTool = visualRelation === "stacked";

  return (
    <div
      className={cn(
        "relative mx-4 my-1.5",
        isAttachedToNarrative && "mt-0 pl-5",
        isStackedTool && "-mt-0.5 pl-5"
      )}
    >
      {(isAttachedToNarrative || isStackedTool) && (
        <div
          aria-hidden="true"
          className={cn(
            "absolute left-2 top-0 w-px bg-[var(--ansi-magenta)]/25",
            isAttachedToNarrative ? "-translate-y-3 h-3" : "-translate-y-2 h-2"
          )}
        />
      )}
      <div className="overflow-hidden rounded-md border border-border/15 border-l border-l-[var(--ansi-magenta)]/35 bg-background/35">
        <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
          <CollapsibleTrigger className="group flex w-full min-w-0 items-center gap-1.5 px-3 py-2 text-xs transition-colors hover:bg-foreground/[0.035]">
            {isExpanded ? (
              <ChevronDown className="h-3 w-3 text-muted-foreground flex-shrink-0" />
            ) : (
              <ChevronRight className="h-3 w-3 text-muted-foreground flex-shrink-0" />
            )}
            <Wand2 className="h-3 w-3 text-[var(--ansi-magenta)]/55 flex-shrink-0" />
            <StatusIcon status={status} size="sm" />
            {isShellLikeOutput ? (
              <Terminal className="h-3 w-3 text-[var(--ansi-green)]/85 flex-shrink-0" />
            ) : null}
            <span className="shrink-0 text-[12px] font-medium text-foreground/85" title={tool.name}>
              {actionLabel}
            </span>
            {summaryArg && (
              <span
                className={cn(
                  "min-w-0 truncate font-mono",
                  isShellRunner ? "text-[var(--ansi-green)]/80" : "text-muted-foreground/85"
                )}
                title={summaryArg}
              >
                {isShellRunner && <span className="text-muted-foreground/50 mr-1">$</span>}
                {summaryArg}
              </span>
            )}
            <div className="flex-1" />
            {tool.completedAt && (
              <span className="text-[10px] text-muted-foreground/80 tabular-nums flex-shrink-0">
                {formatDurationShort(
                  new Date(tool.completedAt).getTime() - new Date(tool.startedAt).getTime()
                )}
              </span>
            )}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <div className="px-4 pb-3 space-y-2.5 text-xs overflow-hidden border-t border-border/10 bg-background/25 pt-2.5">
              {!isShellRunner && tool.args && typeof tool.args === "object" && (
                <div className="overflow-hidden">
                  <div className="flex items-center gap-1.5 mb-1.5">
                    <ChevronRight className="w-2.5 h-2.5 text-[var(--ansi-cyan)]/50" />
                    <span className="text-[9px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                      Input
                    </span>
                  </div>
                  <div className="rounded-md bg-muted/30 border border-border/15 overflow-hidden">
                    <ToolArgsTable args={tool.args as Record<string, unknown>} />
                  </div>
                </div>
              )}

              {isShellLikeOutput && shellOutputState.text && (
                <div className="overflow-hidden">
                  <div className="flex items-center gap-1.5 mb-1.5">
                    {isLive ? (
                      <Loader2
                        className={cn(
                          SUB_AGENT_DETAIL_PENDING_OUTPUT_SPINNER_CLASS,
                          status === "backgrounded" && "text-amber-300"
                        )}
                      />
                    ) : status === "error" ? (
                      <XCircle className="w-2.5 h-2.5 text-[var(--ansi-red)]/70" />
                    ) : (
                      <CheckCircle2 className="w-2.5 h-2.5 text-[var(--ansi-green)]/50" />
                    )}
                    <span className="text-[9px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                      Output
                    </span>
                  </div>
                  {shellOutputJsonValue ? (
                    <div className="max-h-60 overflow-auto rounded-md border border-border/15 bg-muted/30">
                      <JsonView value={shellOutputJsonValue} className="px-3 py-2" />
                    </div>
                  ) : (
                    <pre
                      ref={preRef}
                      className={cn(
                        "ansi-output max-h-60 overflow-auto whitespace-pre-wrap rounded border border-border/15 bg-background/35 px-3 py-2 text-[11px] font-mono text-muted-foreground",
                        isStreaming && "border-l-2 border-[var(--ansi-blue)]"
                      )}
                    >
                      <SubAgentShellOutputText
                        text={limitLiveOutputForRender(shellOutputState.text, isLive)}
                      />
                    </pre>
                  )}
                </div>
              )}

              {!isShellLikeOutput && isLive && displayedLiveOutputText && (
                <div className="overflow-hidden">
                  <div className="flex items-center gap-1.5 mb-1.5">
                    <Loader2
                      className={cn(
                        SUB_AGENT_DETAIL_PENDING_OUTPUT_SPINNER_CLASS,
                        status === "backgrounded" && "text-amber-300"
                      )}
                    />
                    <span className="text-[9px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                      Output
                    </span>
                  </div>
                  <pre className="ansi-output max-h-60 overflow-auto whitespace-pre-wrap rounded border border-border/15 border-l-2 border-[var(--ansi-blue)] bg-background/35 px-3 py-2 text-[11px] font-mono text-muted-foreground">
                    <SubAgentShellOutputText text={displayedLiveOutputText} />
                  </pre>
                </div>
              )}

              {!isShellLikeOutput && !isLive && tool.result !== undefined && (
                <div className="overflow-hidden">
                  <div className="flex items-center gap-1.5 mb-1.5">
                    {status === "error" ? (
                      <XCircle className="w-2.5 h-2.5 text-[var(--ansi-red)]/70" />
                    ) : (
                      <CheckCircle2 className="w-2.5 h-2.5 text-[var(--ansi-green)]/50" />
                    )}
                    <span className="text-[9px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                      Output
                    </span>
                  </div>
                  <ToolResultDisplay result={tool.result} />
                </div>
              )}

              {isShellLikeOutput &&
                !!tool.result &&
                typeof tool.result === "object" &&
                !!(tool.result as Record<string, unknown>).error && (
                  <div className="flex items-start gap-2 rounded-md bg-[var(--ansi-red)]/10 px-3 py-2 text-[11px] text-[var(--ansi-red)]">
                    <XCircle className="w-3 h-3 mt-0.5 flex-shrink-0" />
                    <span>{String((tool.result as Record<string, unknown>).error)}</span>
                  </div>
                )}
            </div>
          </CollapsibleContent>
        </Collapsible>
      </div>
    </div>
  );
});

const NestedSubAgentCard = memo(function NestedSubAgentCard({
  agent,
  sessionId,
  onOpen,
}: {
  agent: ActiveSubAgent;
  sessionId: string;
  onOpen: (parentRequestId: string) => void;
}) {
  const { t } = useTranslation();
  const Icon = getAgentIcon(agent.agentName || agent.agentId);
  const color = getAgentColor(agent.agentName || agent.agentId);
  const status: "running" | "completed" | "error" | "interrupted" =
    agent.status === "completed"
      ? "completed"
      : agent.status === "error"
        ? "error"
        : agent.status === "interrupted"
          ? "interrupted"
          : "running";

  return (
    <button
      type="button"
      onClick={() => onOpen(agent.parentRequestId)}
      className={cn(
        "group mx-3 my-2 w-[calc(100%-1.5rem)] rounded-lg border border-border/30 bg-card/80 px-3 py-2.5 text-left shadow-sm transition-colors hover:border-accent/40 hover:bg-accent/10",
        status === "running" && "border-l-2 border-l-accent",
        status === "error" && "border-l-2 border-l-destructive/70"
      )}
      style={status === "running" ? { borderLeftColor: color } : undefined}
    >
      <div className="flex min-w-0 items-center gap-2">
        <StatusIcon status={status} size="sm" />
        <Icon className="h-3.5 w-3.5 flex-shrink-0" style={{ color }} />
        <span className="text-[11px] text-muted-foreground/65">{t("ai.subAgent.delegateTo")}</span>
        <span className="truncate text-[12px] font-semibold text-foreground/85">
          {agent.agentName || agent.agentId}
        </span>
        <AnchorChip sessionId={sessionId} requestId={agent.parentRequestId} />
        <span className="min-w-0 flex-1" />
        {agent.durationMs != null && (
          <span className="flex-shrink-0 text-[10px] text-muted-foreground/60 tabular-nums">
            {formatDurationShort(agent.durationMs)}
          </span>
        )}
        <ChevronRight className="h-3 w-3 flex-shrink-0 text-muted-foreground/45 transition-colors group-hover:text-accent/70" />
      </div>
      {agent.task && (
        <div className="mt-1.5 flex min-w-0 items-center gap-2 pl-5">
          <span className="h-px w-3 flex-shrink-0 bg-border/50" />
          <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground/65">
            {agent.task}
          </span>
        </div>
      )}
      {agent.thinking && status === "running" && (
        <div className="mt-1.5 flex min-w-0 items-center gap-2 pl-5">
          <Loader2 className={cn(SUB_AGENT_DETAIL_RUNNING_SPINNER_CLASS, "text-accent/70")} />
          <span className="min-w-0 flex-1 truncate text-[10px] text-accent/70">
            {agent.thinking}
          </span>
        </div>
      )}
    </button>
  );
});

/* ─── Status badge ─── */

export const SUB_AGENT_HEADER_STATUS_BADGE_STYLES: Record<
  SubAgentHeaderStatus,
  { badgeClass: string }
> = {
  running: {
    badgeClass: "border-[var(--ansi-blue)]/45 bg-[var(--ansi-blue)]/15 text-[var(--ansi-blue)]",
  },
  backgrounded: { badgeClass: "border-amber-300/45 bg-amber-400/15 text-amber-300" },
  completed: { badgeClass: "bg-[var(--success-dim)] text-[var(--success)]" },
  error: { badgeClass: "bg-destructive/10 text-destructive" },
  interrupted: { badgeClass: "bg-yellow-500/10 text-yellow-400" },
};

/* ─── Main Component ─── */

interface SubAgentDetailViewProps {
  sessionId: string;
}

const EMPTY_SUB_AGENT_LIST: ActiveSubAgent[] = [];
/** Stable empty array so the background-jobs selector doesn't churn re-renders. */
const EMPTY_BG_JOBS: never[] = [];
const LIVE_OUTPUT_RENDER_LIMIT = 20000;

interface StageCoverageContext {
  organizationId: string;
  organizationName: string;
  stage: string;
  stageLabel: string;
}

export function parseStageRunOrgRequestId(
  parentRequestId: string | null | undefined
): { stageRunRequestId: string; organizationId: string } | null {
  if (!parentRequestId) return null;
  const marker = "::org::";
  const idx = parentRequestId.indexOf(marker);
  if (idx <= 0) return null;
  const organizationId = parentRequestId.slice(idx + marker.length).trim();
  if (!organizationId) return null;
  return {
    stageRunRequestId: parentRequestId.slice(0, idx),
    organizationId,
  };
}

function stageKeyFromLabel(stageLabel: string, coverageAxis: string[]): string | null {
  const label = stageLabel.toLowerCase();
  if (
    label.includes("target") ||
    label.includes("intel") ||
    coverageAxis.some((tech) =>
      ["DNS", "WHOIS", "ASN", "CT", "SUBDOMAIN", "OSINT"].includes(tech.toUpperCase())
    )
  ) {
    return "target_intel";
  }
  if (label.includes("external") || coverageAxis.includes("LIVENESS")) {
    return "external_attack_surface";
  }
  if (label.includes("enumeration") || coverageAxis.includes("DIR")) {
    return "enumeration";
  }
  return null;
}

function stageSupportsAssetCoverage(stage: string | null) {
  return stage === "target_intel" || stage === "external_attack_surface" || stage === "enumeration";
}

export function isTerminalStageRunToolStatus(status: string | null | undefined): boolean {
  return Boolean(status && status !== "running" && status !== "backgrounded");
}

export function resolveStageCoverageContextForSubAgent(
  parentRequestId: string | null | undefined,
  stageRuns: Record<string, SessionStageRun> | undefined,
  fallbackStageRun: SessionStageRun | null | undefined
): StageCoverageContext | null {
  const parsed = parseStageRunOrgRequestId(parentRequestId);
  if (!parsed) return null;
  const stageRun =
    stageRuns?.[parsed.stageRunRequestId] ??
    (fallbackStageRun?.requestId === parsed.stageRunRequestId ? fallbackStageRun : null);
  if (!stageRun) return null;
  const row =
    stageRun.rows.find((candidate) => candidate.agentRequestId === parentRequestId) ??
    stageRun.rows.find((candidate) => candidate.id === parsed.organizationId);
  const stage = row?.stage ?? stageKeyFromLabel(stageRun.stageLabel, stageRun.coverageAxis);
  if (!stageSupportsAssetCoverage(stage)) return null;
  return {
    organizationId: parsed.organizationId,
    organizationName: row?.name ?? parsed.organizationId,
    stage,
    stageLabel: stageRun.stageLabel,
  };
}

function limitLiveOutputForRender(text: string, isLive: boolean): string {
  if (!isLive || text.length <= LIVE_OUTPUT_RENDER_LIMIT) return text;
  return `... (showing latest output)\n${text.slice(-LIVE_OUTPUT_RENDER_LIMIT)}`;
}

export const SubAgentDetailView = memo(function SubAgentDetailView({
  sessionId,
}: SubAgentDetailViewProps) {
  const { t } = useTranslation();
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);
  const requestIds = useStore((s) => s.sessions[sessionId]?.toolDetailRequestIds);
  const targetRequestId = requestIds?.[requestIds.length - 1] ?? null;
  const subAgents = useStore((s) => s.activeSubAgents[sessionId] ?? EMPTY_SUB_AGENT_LIST);

  const subAgent = useStore((s) => {
    if (!targetRequestId) return null;
    return (
      (s.activeSubAgents[sessionId] ?? EMPTY_SUB_AGENT_LIST).find(
        (a) => a.parentRequestId === targetRequestId
      ) ?? null
    );
  });
  const sessionStageRuns = useStore((s) => s.sessions[sessionId]?.stageRuns);
  const sessionStageRun = useStore((s) => s.sessions[sessionId]?.stageRun);
  const stageCoverageContext = useMemo(
    () =>
      resolveStageCoverageContextForSubAgent(targetRequestId, sessionStageRuns, sessionStageRun),
    [targetRequestId, sessionStageRuns, sessionStageRun]
  );
  const parentStageRunToolStatus = useStore((s) => {
    const parsed = parseStageRunOrgRequestId(targetRequestId);
    if (!parsed) return null;
    const timeline = s.timelines[sessionId] ?? [];
    for (let i = timeline.length - 1; i >= 0; i--) {
      const block = timeline[i];
      if (block.type === "ai_tool_execution" && block.data.requestId === parsed.stageRunRequestId) {
        return block.data.status;
      }
    }
    return null;
  });
  const parentStageRunToolStartedAt = useStore((s) => {
    const parsed = parseStageRunOrgRequestId(targetRequestId);
    if (!parsed) return null;
    const timeline = s.timelines[sessionId] ?? [];
    for (let i = timeline.length - 1; i >= 0; i--) {
      const block = timeline[i];
      if (block.type === "ai_tool_execution" && block.data.requestId === parsed.stageRunRequestId) {
        return block.data.startedAt ?? null;
      }
    }
    return null;
  });
  const parentStageStopped = isTerminalStageRunToolStatus(parentStageRunToolStatus);
  // Session-wide background jobs (soft-timeout→detached commands still running),
  // surfaced here so backgrounded recon/sub-agent commands are visible from the
  // detail view, not only the input-row badge.
  const backgroundJobs = useStore((s) => s.backgroundJobs[sessionId]) ?? EMPTY_BG_JOBS;

  const scrollRef = useRef<HTMLDivElement>(null);
  const timelineScrollFrameRef = useRef<number | null>(null);
  const shouldStickToBottomRef = useRef(true);
  const previousTimelineScrollTopRef = useRef(0);
  const [copiedSection, setCopiedSection] = useState<string | null>(null);
  const [isTaskExpanded, setIsTaskExpanded] = useState(false);
  const [activeDetailTab, setActiveDetailTab] = useState<SubAgentDetailTab>("run");
  const isRunning = subAgent?.status === "running" && !parentStageStopped;
  const hasParentSubAgent = (requestIds?.length ?? 0) > 1;
  const backLabel = hasParentSubAgent
    ? t("ai.subAgentDetail.backToParent")
    : t("ai.toolDetail.backToTerminal");

  const detailEntries = subAgent ? normalizeSubAgentEntriesForDetail(subAgent.entries) : [];
  const latestEntry = detailEntries.length > 0 ? detailEntries[detailEntries.length - 1] : null;
  const latestRunningTool = subAgent?.toolCalls.find(
    (tool) => getSubAgentToolDisplayStatus(tool, { parentStageStopped }) === "running"
  );
  const activityVersion = [
    subAgent?.parentRequestId,
    subAgent?.status,
    parentStageStopped ? "parent-stopped" : "parent-active",
    detailEntries.length,
    latestEntry?.kind,
    latestEntry?.text?.length ?? 0,
    latestEntry?.toolCallId ?? "",
    subAgent?.toolCalls.length ?? 0,
    latestRunningTool?.id ?? "",
    latestRunningTool?.streamingOutput?.length ?? 0,
  ].join(":");

  const updateStickiness = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    shouldStickToBottomRef.current = shouldStickToBottomAfterScroll(
      previousTimelineScrollTopRef.current,
      {
        scrollTop: el.scrollTop,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
      }
    );
    previousTimelineScrollTopRef.current = el.scrollTop;
  }, []);

  const handleTimelineWheel = useCallback((event: WheelEvent<HTMLDivElement>) => {
    if (event.deltaY < 0) {
      shouldStickToBottomRef.current = false;
    }
  }, []);

  const scheduleTimelineScrollToBottom = useCallback(() => {
    if (timelineScrollFrameRef.current != null) return;
    timelineScrollFrameRef.current = requestAnimationFrame(() => {
      timelineScrollFrameRef.current = null;
      const el = scrollRef.current;
      if (!el || !shouldStickToBottomRef.current) return;
      el.scrollTop = el.scrollHeight;
      previousTimelineScrollTopRef.current = el.scrollTop;
    });
  }, []);

  // Follow streaming output only while the user is already near the bottom.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !isRunning || !shouldStickToBottomRef.current) return;
    scheduleTimelineScrollToBottom();
  }, [activityVersion, isRunning, scheduleTimelineScrollToBottom]);

  useEffect(() => {
    shouldStickToBottomRef.current = true;
    previousTimelineScrollTopRef.current = scrollRef.current?.scrollTop ?? 0;
    if (scrollRef.current && isRunning) scheduleTimelineScrollToBottom();
  }, [targetRequestId, isRunning, scheduleTimelineScrollToBottom]);

  useEffect(() => {
    setIsTaskExpanded(false);
    setActiveDetailTab("run");
  }, [targetRequestId]);

  useEffect(() => {
    if (!stageCoverageContext && activeDetailTab === "coverage") {
      setActiveDetailTab("run");
    }
  }, [activeDetailTab, stageCoverageContext]);

  useEffect(() => {
    return () => {
      if (timelineScrollFrameRef.current != null) {
        cancelAnimationFrame(timelineScrollFrameRef.current);
        timelineScrollFrameRef.current = null;
      }
    };
  }, []);

  const handleCopy = async (content: string, section: string) => {
    if (await copyToClipboard(content)) {
      setCopiedSection(section);
      setTimeout(() => setCopiedSection(null), 2000);
    }
  };

  const navigateBack = useCallback(() => {
    if (requestIds && requestIds.length > 1) {
      const popped = requestIds.slice(0, -1);
      const newTop = popped[popped.length - 1];
      const store = useStore.getState();
      store.setToolDetailRequestIds(sessionId, popped);
      // If the level we popped back to isn't a sub-agent (e.g. the `stage_run`
      // tool row a per-org drill-in started from), return to the tool-call
      // detail instead of staying here — otherwise this pane would look up a
      // sub-agent that doesn't exist for that requestId and render empty.
      const stillSubAgent = (store.activeSubAgents[sessionId] ?? []).some(
        (a) => a.parentRequestId === newTop
      );
      if (!stillSubAgent) {
        store.setDetailViewMode(sessionId, "tool-detail");
      }
      return;
    }
    setDetailViewMode(sessionId, "timeline");
  }, [requestIds, sessionId, setDetailViewMode]);
  const openSubAgent = useCallback(
    (parentRequestId: string) => {
      const store = useStore.getState();
      store.setToolDetailRequestIds(sessionId, [...(requestIds ?? []), parentRequestId]);
      store.setDetailViewMode(sessionId, "sub-agent-detail");
    },
    [requestIds, sessionId]
  );

  if (!subAgent) {
    return (
      <div className="h-full flex flex-col bg-card">
        <div className="flex items-center gap-3 px-3 py-2 border-b border-[var(--border-subtle)] flex-shrink-0">
          <button
            type="button"
            onClick={navigateBack}
            className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
          >
            <ArrowLeft className="w-3.5 h-3.5" />
            {backLabel}
          </button>
        </div>
        <div className="flex-1 flex items-center justify-center text-sm text-muted-foreground/60">
          {t("ai.subAgentDetail.notFound")}
        </div>
      </div>
    );
  }

  const AgentIcon = getAgentIcon(subAgent.agentName);
  const agentColor = getAgentColor(subAgent.agentName);
  const headerDisplayStatus = getSubAgentHeaderDisplayStatus(subAgent, { parentStageStopped });
  const headerStatus = SUB_AGENT_HEADER_STATUS_BADGE_STYLES[headerDisplayStatus];
  const isHeaderLive = headerDisplayStatus === "running" || headerDisplayStatus === "backgrounded";
  const stageAssetWorkItems = summarizeSubAgentAssetWork(subAgent.toolCalls, {
    parentStageStopped,
  });
  const showCoverageView = Boolean(stageCoverageContext && activeDetailTab === "coverage");
  const toolMap = new Map(subAgent.toolCalls.map((tc) => [tc.id, tc]));
  const subAgentMap = new Map(subAgents.map((agent) => [agent.parentRequestId, agent]));
  const hasEntries = detailEntries.length > 0;
  const backgroundedToolCount = subAgent.toolCalls.filter(
    (tool) => getSubAgentToolDisplayStatus(tool, { parentStageStopped }) === "backgrounded"
  ).length;
  const cleanedTask = stripAgentXmlTags(subAgent.task);
  const taskPreview = cleanedTask.replace(/\s+/g, " ").trim();

  return (
    <div className="h-full flex flex-col bg-card">
      {/* Header */}
      <div className="flex items-center gap-3 px-3 py-2 border-b border-[var(--border-subtle)] flex-shrink-0">
        <button
          type="button"
          onClick={navigateBack}
          className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          <ArrowLeft className="w-3.5 h-3.5" />
          {backLabel}
        </button>
        <div className="w-px h-4 bg-[var(--border-subtle)]" />
        <AgentIcon className="w-4 h-4 flex-shrink-0" style={{ color: agentColor }} />
        <span className="text-sm font-medium truncate">{subAgent.agentName}</span>
        <AnchorChip sessionId={sessionId} requestId={subAgent.parentRequestId} />
        <Badge
          variant="outline"
          className={cn("gap-1 flex items-center text-[10px] px-2 py-0.5", headerStatus.badgeClass)}
        >
          {isHeaderLive && (
            <Loader2
              className={cn(
                SUB_AGENT_DETAIL_RUNNING_SPINNER_CLASS,
                headerDisplayStatus === "backgrounded" && "text-amber-300"
              )}
            />
          )}
          {t(`ai.subAgentDetail.status.${headerDisplayStatus}`)}
        </Badge>
        {subAgent.durationMs != null && (
          <span className="text-[11px] text-muted-foreground/70 tabular-nums flex items-center gap-1">
            <Clock className="w-3 h-3" />
            {formatDurationShort(subAgent.durationMs)}
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

      {/* Task assignment block */}
      {cleanedTask && (
        <Collapsible
          open={isTaskExpanded}
          onOpenChange={setIsTaskExpanded}
          className="mx-3 mt-2 mb-3 flex-shrink-0 overflow-hidden rounded-md border border-border/50 border-l-2 border-l-accent/60 bg-background/70 shadow-sm"
        >
          <div className="flex items-center justify-between gap-2 px-3 py-2">
            <CollapsibleTrigger className="group flex min-w-0 flex-1 items-center gap-2 text-left">
              {isTaskExpanded ? (
                <ChevronDown className="h-3.5 w-3.5 flex-shrink-0 text-foreground/55" />
              ) : (
                <ChevronRight className="h-3.5 w-3.5 flex-shrink-0 text-foreground/55" />
              )}
              <span className="flex-shrink-0 text-[10px] font-semibold text-foreground/80 uppercase tracking-wider">
                {t("ai.subAgentDetail.task")}
              </span>
              <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground/80">
                {taskPreview}
              </span>
            </CollapsibleTrigger>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => handleCopy(subAgent.task, "task")}
              className="h-5 flex-shrink-0 text-[10px] px-1.5 opacity-60 hover:opacity-100 transition-opacity"
            >
              <Copy className="w-2.5 h-2.5 mr-0.5" />
              {copiedSection === "task"
                ? t("ai.subAgentDetail.copied")
                : t("ai.subAgentDetail.copy")}
            </Button>
          </div>
          <CollapsibleContent className="border-t border-border/25 bg-card/70 px-3 pb-3 pt-2">
            <div className="max-h-36 overflow-auto pr-1 text-[12.5px] text-foreground leading-[1.7] [&_p]:mb-2 [&_p:last-child]:mb-0 [&_ul]:my-1.5 [&_ol]:my-1.5">
              <Markdown content={cleanedTask} />
            </div>
          </CollapsibleContent>
        </Collapsible>
      )}

      {stageCoverageContext && !showCoverageView && (
        <div className="mx-3 mb-2 flex-shrink-0">
          <StageAssetCoverageBlock
            displayMode="summary"
            organizationId={stageCoverageContext.organizationId}
            stage={stageCoverageContext.stage}
            sessionId={sessionId}
            stageStartedAt={parentStageRunToolStartedAt}
            title="资产覆盖"
            subtitle={`${stageCoverageContext.stageLabel} · ${stageCoverageContext.organizationName}`}
            pollWhileActive={isHeaderLive}
            workItems={stageAssetWorkItems}
            onOpenCoverage={() => setActiveDetailTab("coverage")}
          />
        </div>
      )}

      {stageCoverageContext && showCoverageView ? (
        <div className="min-h-0 flex-1 px-3 pb-3">
          <StageAssetCoverageBlock
            displayMode="panel"
            organizationId={stageCoverageContext.organizationId}
            stage={stageCoverageContext.stage}
            sessionId={sessionId}
            stageStartedAt={parentStageRunToolStartedAt}
            title="资产覆盖"
            subtitle={`${stageCoverageContext.stageLabel} · ${stageCoverageContext.organizationName}`}
            pollWhileActive={isHeaderLive}
            workItems={stageAssetWorkItems}
            onBackToRun={() => setActiveDetailTab("run")}
          />
        </div>
      ) : (
        /* Timeline content */
        <div
          ref={scrollRef}
          className="flex-1 overflow-y-auto"
          onScroll={updateStickiness}
          onWheelCapture={handleTimelineWheel}
        >
          {/* Prompt generation (collapsible) */}
          {subAgent.promptGeneration && (
            <div className="border-b border-border/20">
              <Collapsible>
                <CollapsibleTrigger className="group flex w-full items-center gap-1.5 px-4 py-2 text-xs hover:bg-accent/30 transition-colors">
                  {subAgent.promptGeneration.status === "generating" ? (
                    <Loader2 className="h-3 w-3 text-[var(--ansi-yellow)] animate-spin" />
                  ) : subAgent.promptGeneration.status === "completed" ? (
                    <CheckCircle2 className="h-3 w-3 text-[var(--ansi-green)]" />
                  ) : (
                    <XCircle className="h-3 w-3 text-[var(--ansi-red)]" />
                  )}
                  <Wand2 className="h-3 w-3 text-[var(--ansi-yellow)]" />
                  <span className="text-muted-foreground">
                    {subAgent.promptGeneration.status === "generating"
                      ? t("ai.subAgentDetail.promptGenerating")
                      : subAgent.promptGeneration.status === "completed"
                        ? t("ai.subAgentDetail.promptGenerated")
                        : t("ai.subAgentDetail.promptFailed")}
                  </span>
                  {subAgent.promptGeneration.durationMs != null && (
                    <span className="ml-auto text-[10px] text-muted-foreground flex items-center gap-0.5">
                      <Clock className="h-2.5 w-2.5" />
                      {formatDurationShort(subAgent.promptGeneration.durationMs)}
                    </span>
                  )}
                </CollapsibleTrigger>
                <CollapsibleContent className="px-4 pb-2">
                  <div className="space-y-1.5 text-xs">
                    {subAgent.promptGeneration.generatedPrompt && (
                      <details className="group" open>
                        <summary className="cursor-pointer select-none text-muted-foreground hover:text-foreground/80">
                          Generated system prompt
                        </summary>
                        <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap rounded bg-muted px-2 py-1 text-[10px]">
                          {subAgent.promptGeneration.generatedPrompt}
                        </pre>
                      </details>
                    )}
                  </div>
                </CollapsibleContent>
              </Collapsible>
            </div>
          )}

          {/* Interleaved timeline entries: agent narrative blocks + tool calls */}
          <div>
            {hasEntries
              ? detailEntries.map((entry, i) => {
                  const previous = i > 0 ? detailEntries[i - 1] : null;
                  const boundaryClass = shouldSeparateSubAgentDetailEntries(previous, entry)
                    ? "border-t border-border/10"
                    : "";
                  const renderedEntry = (() => {
                    if (entry.kind === "thinking" && entry.text) {
                      return (
                        <div className={SUB_AGENT_DETAIL_NARRATIVE_BLOCK_CLASS}>
                          <ThinkingBlock
                            content={entry.text}
                            isActive={isRunning && i === detailEntries.length - 1}
                            startedAt={entry.startedAt}
                            endedAt={entry.endedAt}
                            variant="detail"
                          />
                        </div>
                      );
                    }
                    if (entry.kind === "text" && entry.text) {
                      return (
                        <AgentOutputBlock
                          compactTop={previous?.kind === "thinking"}
                          text={entry.text}
                          streaming={isRunning && i === detailEntries.length - 1}
                        />
                      );
                    }
                    if (entry.kind === "tool_call" && entry.toolCallId) {
                      const tool = toolMap.get(entry.toolCallId);
                      if (tool?.name.startsWith("sub_agent_")) {
                        const nestedAgent = subAgentMap.get(tool.id);
                        if (nestedAgent) {
                          return (
                            <NestedSubAgentCard
                              agent={nestedAgent}
                              sessionId={sessionId}
                              onOpen={openSubAgent}
                            />
                          );
                        }
                      }
                      if (tool) {
                        return (
                          <AgentToolCallBlock
                            tool={tool}
                            parentStageStopped={parentStageStopped}
                            visualRelation={getSubAgentToolCallVisualRelation(previous)}
                          />
                        );
                      }
                    }
                    return null;
                  })();
                  if (!renderedEntry) return null;
                  return (
                    <div key={`entry-${i}`} className={boundaryClass}>
                      {renderedEntry}
                    </div>
                  );
                })
              : subAgent.toolCalls.length > 0
                ? subAgent.toolCalls.map((tool) => (
                    <AgentToolCallBlock
                      key={tool.id}
                      tool={tool}
                      parentStageStopped={parentStageStopped}
                    />
                  ))
                : null}
          </div>

          {/* Error */}
          {subAgent.error && (
            <div className="mx-3 my-2.5 rounded-lg bg-destructive/10 border border-destructive/25 p-3.5 overflow-hidden">
              <div className="flex items-start gap-2">
                <XCircle className="w-3.5 h-3.5 text-destructive mt-0.5 flex-shrink-0" />
                <p className="text-[12.5px] text-destructive leading-[1.6] whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
                  {subAgent.error}
                </p>
              </div>
            </div>
          )}
        </div>
      )}

      {/* Running footer */}
      {isRunning && (
        <div className="px-3 py-2 border-t border-[var(--border-subtle)] bg-[var(--ansi-blue)]/10 flex items-center gap-2 flex-shrink-0">
          <Loader2
            className={cn(SUB_AGENT_DETAIL_RUNNING_SPINNER_CLASS, "text-[var(--ansi-blue)]")}
          />
          <span className="text-[11px] text-[var(--ansi-blue)]">
            {t("ai.subAgentDetail.agentRunning")}
          </span>
        </div>
      )}
    </div>
  );
});
