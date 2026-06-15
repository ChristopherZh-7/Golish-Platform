/**
 * SubAgentDetailView
 *
 * 以时间线（对话流）样式展示单个 sub-agent 的完整执行过程：
 * 文字输出和工具调用按时间顺序交错显示，类似右侧 ChatPanel 的 primary agent 消息流。
 */
import {
  ArrowLeft,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  Copy,
  Loader2,
  Terminal,
  Wand2,
  XCircle,
} from "lucide-react";
import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type WheelEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { ThinkingBlock } from "@/components/AIChatPanel/ThinkingBlock";
import { JsonView } from "@/components/JsonView/JsonView";
import { Markdown } from "@/components/Markdown";
import { AnchorChip } from "@/components/ui/AnchorChip";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { StatusIcon } from "@/components/ui/StatusIcon";
import { collapseProgressBars, stripAllAnsi } from "@/lib/ansi";
import { copyToClipboard } from "@/lib/clipboard";
import { getAgentColor, getAgentIcon } from "@/lib/sub-agent-theme";
import { safeStringify } from "@/lib/text";
import { formatDurationShort } from "@/lib/time";
import { getToolPrimaryArg } from "@/lib/tools";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, SubAgentToolCall } from "@/store";
import { useStore } from "@/store";

export function stripAgentXmlTags(text: string): string {
  return stripAllAnsi(
    text
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

/* ─── Sub-agent text output block ─── */

const AgentOutputBlock = memo(function AgentOutputBlock({
  text,
  streaming = false,
}: {
  text: string;
  streaming?: boolean;
}) {
  const cleaned = stripAgentXmlTags(text);
  if (!cleaned) return null;
  return (
    <div className="px-4 py-3 border-l-2 border-accent/25 bg-[var(--bg-hover)]/60">
      <div className="flex items-center gap-1.5 mb-1.5">
        <Wand2 className="w-2.5 h-2.5 text-foreground/55" />
        <span className="text-[9px] font-semibold text-foreground/60 uppercase tracking-wider">
          Agent Output
        </span>
      </div>
      <div className="overflow-hidden break-words text-[12.5px] text-foreground leading-[1.7] [overflow-wrap:anywhere] [&_blockquote]:my-2 [&_code]:break-words [&_h1]:!text-foreground [&_h1]:mt-3 [&_h2]:!text-foreground/90 [&_h2]:mt-2.5 [&_h3]:!text-foreground/85 [&_h3]:mt-2 [&_ol]:my-1.5 [&_p]:mb-2 [&_p:last-child]:mb-0 [&_pre]:my-2 [&_pre]:max-w-full [&_pre]:overflow-x-auto [&_table]:my-2 [&_table]:block [&_table]:max-w-full [&_table]:overflow-x-auto [&_td]:break-words [&_th]:break-words [&_ul]:my-1.5">
        <Markdown content={cleaned} streaming={streaming} />
      </div>
    </div>
  );
});

/* ─── Structured args display ─── */

function ToolArgsTable({ args }: { args: Record<string, unknown> }) {
  if (Object.keys(args).length === 0) return null;
  return <JsonView value={args} className="px-3 py-2" />;
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
      <div className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-64 overflow-auto">
        <JsonView value={structured} />
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

const AgentToolCallBlock = memo(function AgentToolCallBlock({ tool }: { tool: SubAgentToolCall }) {
  const isShellCmd = tool.name === "run_pty_cmd" || tool.name === "run_command";
  const [isExpanded, setIsExpanded] = useState(false);
  const preRef = useRef<HTMLPreElement>(null);
  const status: "running" | "completed" | "error" | "interrupted" =
    (tool.status as string) === "completed"
      ? "completed"
      : (tool.status as string) === "error"
        ? "error"
        : (tool.status as string) === "interrupted"
          ? "interrupted"
          : "running";
  const isStreaming = isShellCmd && tool.status === "running" && !!tool.streamingOutput;

  useEffect(() => {
    if (isStreaming && preRef.current) {
      preRef.current.scrollTop = preRef.current.scrollHeight;
    }
  }, [isStreaming]);

  // Reuse the shared primary-arg formatter (same as the main timeline cards) so
  // every collapsed row surfaces a one-line summary. In particular pentest_run
  // nests the real tool under `tool_name`/`args`, so this renders e.g.
  // "nmap -sV target" inline without the user expanding the row.
  const summaryArg = getToolPrimaryArg(tool.name, tool.args);

  const shellOutput: string | null = (() => {
    if (!isShellCmd) return null;
    let raw: string | null;
    if (tool.streamingOutput) {
      raw = tool.streamingOutput;
    } else if (tool.result && typeof tool.result === "object") {
      const r = tool.result as Record<string, unknown>;
      raw = (r.stdout as string) || (r.output as string) || null;
    } else {
      raw = null;
    }
    // Collapse progress-bar spam (amass/nuclei "[--->] 100%") to the final frame.
    return raw ? collapseProgressBars(raw) : null;
  })();

  return (
    <div className="mx-3 my-2 rounded-lg border border-border/30 overflow-hidden bg-card/80 shadow-sm border-l-2 border-l-[var(--ansi-magenta)]/40">
      <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
        <CollapsibleTrigger className="group flex w-full items-center gap-1.5 px-3 py-2 text-xs hover:bg-accent/20 transition-colors">
          {isExpanded ? (
            <ChevronDown className="h-3 w-3 text-muted-foreground flex-shrink-0" />
          ) : (
            <ChevronRight className="h-3 w-3 text-muted-foreground flex-shrink-0" />
          )}
          <Wand2 className="h-3 w-3 text-[var(--ansi-magenta)]/70 flex-shrink-0" />
          <StatusIcon status={status} size="sm" />
          {isShellCmd ? (
            <Terminal className="h-3 w-3 text-[var(--ansi-green)] flex-shrink-0" />
          ) : null}
          <span className="font-mono text-[var(--ansi-cyan)]">{isShellCmd ? "" : tool.name}</span>
          {summaryArg && (
            <span
              className={cn(
                "truncate font-mono",
                isShellCmd ? "text-[var(--ansi-green)]/80" : "text-muted-foreground"
              )}
              title={summaryArg}
            >
              {isShellCmd && <span className="text-muted-foreground/50 mr-1">$</span>}
              {summaryArg}
            </span>
          )}
          <div className="flex-1" />
          {tool.completedAt && (
            <span className="text-[10px] text-muted-foreground tabular-nums flex-shrink-0">
              {formatDurationShort(
                new Date(tool.completedAt).getTime() - new Date(tool.startedAt).getTime()
              )}
            </span>
          )}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="px-4 pb-3 space-y-2.5 text-xs overflow-hidden border-t border-border/20 pt-2.5">
            {isShellCmd && shellOutput && (
              <pre
                ref={preRef}
                className={cn(
                  "ansi-output max-h-60 overflow-auto whitespace-pre-wrap rounded border border-border/15 bg-background/40 px-3 py-2 text-[11px] font-mono text-muted-foreground",
                  isStreaming && "border-l-2 border-[var(--ansi-blue)]"
                )}
              >
                {shellOutput.length > 5000
                  ? `${shellOutput.slice(0, 5000)}\n... (truncated)`
                  : shellOutput}
              </pre>
            )}

            {!isShellCmd && tool.args && typeof tool.args === "object" && (
              <div className="overflow-hidden">
                <div className="flex items-center gap-1.5 mb-1.5">
                  <ChevronRight className="w-2.5 h-2.5 text-[var(--ansi-cyan)]/50" />
                  <span className="text-[9px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                    Input
                  </span>
                </div>
                <div className="rounded-md bg-muted/40 border border-border/20 overflow-hidden">
                  <ToolArgsTable args={tool.args as Record<string, unknown>} />
                </div>
              </div>
            )}

            {!isShellCmd && tool.result !== undefined && (
              <div className="overflow-hidden">
                <div className="flex items-center gap-1.5 mb-1.5">
                  <CheckCircle2 className="w-2.5 h-2.5 text-[var(--ansi-green)]/50" />
                  <span className="text-[9px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                    Output
                  </span>
                </div>
                <ToolResultDisplay result={tool.result} />
              </div>
            )}

            {isShellCmd &&
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
        <span className="flex-shrink-0 text-[10px] text-muted-foreground/55">
          {agent.toolCalls.length} {t("ai.agentTree.tools")}
        </span>
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
          <Loader2 className="h-2.5 w-2.5 flex-shrink-0 animate-spin text-accent/70" />
          <span className="min-w-0 flex-1 truncate text-[10px] text-accent/70">
            {agent.thinking}
          </span>
        </div>
      )}
    </button>
  );
});

/* ─── Status badge ─── */

const STATUS_BADGE_STYLES: Record<
  "running" | "completed" | "error" | "interrupted",
  { badgeClass: string }
> = {
  running: { badgeClass: "bg-[var(--accent-dim)] text-accent" },
  completed: { badgeClass: "bg-[var(--success-dim)] text-[var(--success)]" },
  error: { badgeClass: "bg-destructive/10 text-destructive" },
  interrupted: { badgeClass: "bg-yellow-500/10 text-yellow-400" },
};

/* ─── Main Component ─── */

interface SubAgentDetailViewProps {
  sessionId: string;
}

const EMPTY_SUB_AGENT_LIST: ActiveSubAgent[] = [];
const AUTO_SCROLL_BOTTOM_THRESHOLD_PX = 96;

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

  const scrollRef = useRef<HTMLDivElement>(null);
  const shouldStickToBottomRef = useRef(true);
  const [copiedSection, setCopiedSection] = useState<string | null>(null);
  const [isTaskExpanded, setIsTaskExpanded] = useState(false);
  const isRunning = subAgent?.status === "running";
  const hasParentSubAgent = (requestIds?.length ?? 0) > 1;
  const backLabel = hasParentSubAgent
    ? t("ai.subAgentDetail.backToParent")
    : t("ai.toolDetail.backToTerminal");

  const latestEntry =
    subAgent && subAgent.entries.length > 0 ? subAgent.entries[subAgent.entries.length - 1] : null;
  const latestRunningTool = subAgent?.toolCalls.find((tool) => tool.status === "running");
  const activityVersion = [
    subAgent?.parentRequestId,
    subAgent?.status,
    subAgent?.entries.length ?? 0,
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
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    shouldStickToBottomRef.current = distanceFromBottom <= AUTO_SCROLL_BOTTOM_THRESHOLD_PX;
  }, []);

  const handleTimelineWheel = useCallback((event: WheelEvent<HTMLDivElement>) => {
    if (event.deltaY < 0) {
      shouldStickToBottomRef.current = false;
    }
  }, []);

  // Follow streaming output only while the user is already near the bottom.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el || !isRunning || !shouldStickToBottomRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [activityVersion, isRunning]);

  useLayoutEffect(() => {
    shouldStickToBottomRef.current = true;
    const el = scrollRef.current;
    if (el && isRunning) {
      el.scrollTop = el.scrollHeight;
    }
  }, [targetRequestId]);

  useEffect(() => {
    setIsTaskExpanded(false);
  }, [targetRequestId]);

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
  const status = STATUS_BADGE_STYLES[subAgent.status];
  const toolMap = new Map(subAgent.toolCalls.map((tc) => [tc.id, tc]));
  const subAgentMap = new Map(subAgents.map((agent) => [agent.parentRequestId, agent]));
  const hasEntries = subAgent.entries.length > 0;
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
          className={cn("gap-1 flex items-center text-[10px] px-2 py-0.5", status.badgeClass)}
        >
          {isRunning && <Loader2 className="w-3 h-3 animate-spin" />}
          {t(`ai.subAgentDetail.status.${subAgent.status}`)}
        </Badge>
        {subAgent.durationMs != null && (
          <span className="text-[11px] text-muted-foreground/70 tabular-nums flex items-center gap-1">
            <Clock className="w-3 h-3" />
            {formatDurationShort(subAgent.durationMs)}
          </span>
        )}
        <span className="text-[11px] text-muted-foreground/60 tabular-nums">
          {subAgent.toolCalls.length} {t("ai.agentTree.tools")}
        </span>
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

      {/* Timeline content */}
      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto"
        onScroll={updateStickiness}
        onWheel={handleTimelineWheel}
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

        {/* Interleaved timeline entries: text blocks + tool calls */}
        <div className="divide-y divide-border/10">
          {hasEntries
            ? subAgent.entries.map((entry, i) => {
                if (entry.kind === "thinking" && entry.text) {
                  return (
                    <div
                      key={`entry-${i}`}
                      className="px-4 py-3 border-l-2 border-muted-foreground/15 bg-[var(--bg-hover)]/35"
                    >
                      <ThinkingBlock
                        content={entry.text}
                        isActive={isRunning && i === subAgent.entries.length - 1}
                        startedAt={entry.startedAt}
                        endedAt={entry.endedAt}
                      />
                    </div>
                  );
                }
                if (entry.kind === "text" && entry.text) {
                  return (
                    <AgentOutputBlock
                      key={`entry-${i}`}
                      text={entry.text}
                      streaming={isRunning && i === subAgent.entries.length - 1}
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
                          key={nestedAgent.parentRequestId}
                          agent={nestedAgent}
                          sessionId={sessionId}
                          onOpen={openSubAgent}
                        />
                      );
                    }
                  }
                  if (tool) return <AgentToolCallBlock key={tool.id} tool={tool} />;
                }
                return null;
              })
            : subAgent.toolCalls.length > 0
              ? subAgent.toolCalls.map((tool) => <AgentToolCallBlock key={tool.id} tool={tool} />)
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

      {/* Running footer */}
      {isRunning && (
        <div className="px-3 py-2 border-t border-[var(--border-subtle)] bg-accent/5 flex items-center gap-2 flex-shrink-0">
          <Loader2 className="w-3 h-3 text-accent animate-spin" />
          <span className="text-[11px] text-accent/80">{t("ai.subAgentDetail.agentRunning")}</span>
        </div>
      )}
    </div>
  );
});
