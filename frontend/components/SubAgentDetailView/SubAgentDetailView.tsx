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
import { memo, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Markdown } from "@/components/Markdown";
import { AnchorChip } from "@/components/ui/AnchorChip";
import { StatusIcon } from "@/components/ui/StatusIcon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { stripAllAnsi } from "@/lib/ansi";
import { copyToClipboard } from "@/lib/clipboard";
import { getAgentColor, getAgentIcon } from "@/lib/sub-agent-theme";
import { formatDurationShort } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, SubAgentEntry, SubAgentToolCall } from "@/store";
import { useStore } from "@/store";

function stripAgentXmlTags(text: string): string {
  return stripAllAnsi(
    text
      .replace(/<\/?(task_assignment|original_request|execution_plan|execution_context|prior_knowledge)>/gi, "")
      .replace(/<function=[^>]*>[\s\S]*?(?:<\/function>|$)/g, "")
      .replace(/<parameter=[^>]*>[\s\S]*?<\/parameter>/g, "")
      .replace(/<\/?(?:function|parameter)[^>]*>/g, ""),
  ).trim();
}

/* ─── Sub-agent text output block ─── */

function AgentOutputBlock({ text }: { text: string }) {
  const cleaned = stripAgentXmlTags(text);
  if (!cleaned) return null;
  return (
    <div className="px-4 py-3 border-l-2 border-accent/25 bg-[var(--bg-hover)]/60">
      <div className="flex items-center gap-1.5 mb-1.5">
        <Wand2 className="w-2.5 h-2.5 text-accent/50" />
        <span className="text-[9px] font-medium text-accent/50 uppercase tracking-wider">Agent Output</span>
      </div>
      <div className="text-[12.5px] text-foreground leading-[1.7] [&_p]:mb-2 [&_p:last-child]:mb-0 [&_ul]:my-1.5 [&_ol]:my-1.5 [&_pre]:my-2 [&_blockquote]:my-2 [&_h1]:mt-3 [&_h2]:mt-2.5 [&_h3]:mt-2">
        <Markdown content={cleaned} />
      </div>
    </div>
  );
}

/* ─── Structured args display ─── */

function ToolArgsTable({ args }: { args: Record<string, unknown> }) {
  const entries = Object.entries(args);
  if (entries.length === 0) return null;

  return (
    <div className="divide-y divide-border/15">
      {entries.map(([key, value]) => {
        const strValue = typeof value === "string" ? value : JSON.stringify(value, null, 2);
        const isLong = strValue.length > 120 || strValue.includes("\n");
        return (
          <div key={key} className={cn("px-3", isLong ? "py-2" : "py-1.5 flex items-baseline gap-3")}>
            <span className="text-[10px] font-mono text-[var(--ansi-cyan)]/70 flex-shrink-0">{key}</span>
            {isLong ? (
              <pre className="mt-1 text-[11px] font-mono text-foreground/80 whitespace-pre-wrap break-all max-h-32 overflow-auto leading-relaxed">
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

/* ─── Tool result display ─── */

function ToolResultDisplay({ result }: { result: unknown }) {
  const strResult = typeof result === "string" ? result : JSON.stringify(result, null, 2);
  const isMarkdownLike =
    typeof result === "string" &&
    (/^#{1,3}\s/m.test(result) || /\*\*/.test(result) || /^[-*]\s/m.test(result) || /```/.test(result));

  if (isMarkdownLike) {
    return (
      <div className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-64 overflow-auto text-[12px] text-foreground leading-[1.65] [&_p]:mb-1.5 [&_p:last-child]:mb-0">
        <Markdown content={result as string} />
      </div>
    );
  }

  return (
    <pre className="rounded-md bg-muted/40 border border-border/20 px-3 py-2.5 max-h-64 overflow-auto text-[11px] font-mono text-foreground/80 whitespace-pre-wrap break-all leading-relaxed">
      {strResult.length > 5000 ? `${strResult.slice(0, 5000)}\n... (truncated)` : strResult}
    </pre>
  );
}

/* ─── Sub-agent tool call block ─── */

function AgentToolCallBlock({ tool }: { tool: SubAgentToolCall }) {
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
  }, [isStreaming, tool.streamingOutput]);

  const summaryArg = (() => {
    const args = tool.args;
    if (typeof args === "object" && args !== null) {
      if ("command" in args) return String(args.command);
      if ("path" in args) return String(args.path);
      if ("file_path" in args) return String(args.file_path);
      if ("pattern" in args) return String(args.pattern);
      if ("url" in args) return String(args.url);
    }
    return null;
  })();

  const shellOutput: string | null = (() => {
    if (!isShellCmd) return null;
    if (tool.streamingOutput) return tool.streamingOutput;
    if (!tool.result || typeof tool.result !== "object") return null;
    const r = tool.result as Record<string, unknown>;
    return (r.stdout as string) || (r.output as string) || null;
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
          <span className="font-mono text-[var(--ansi-cyan)]">
            {isShellCmd ? "" : tool.name}
          </span>
          {summaryArg && (
            <span
              className={cn(
                "truncate font-mono",
                isShellCmd ? "text-[var(--ansi-green)]/80" : "text-muted-foreground",
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
                new Date(tool.completedAt).getTime() - new Date(tool.startedAt).getTime(),
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
                  "max-h-60 overflow-auto whitespace-pre-wrap rounded bg-[var(--ansi-black)]/20 px-3 py-2 text-[11px] font-mono text-foreground/80",
                  isStreaming && "border-l-2 border-[var(--ansi-blue)]",
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
                  <span className="text-[9px] font-semibold text-muted-foreground/70 uppercase tracking-wider">Input</span>
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
                  <span className="text-[9px] font-semibold text-muted-foreground/70 uppercase tracking-wider">Output</span>
                </div>
                <ToolResultDisplay result={tool.result} />
              </div>
            )}

            {isShellCmd &&
              tool.result &&
              typeof tool.result === "object" &&
              (tool.result as Record<string, unknown>).error && (
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
}

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

export const SubAgentDetailView = memo(function SubAgentDetailView({
  sessionId,
}: SubAgentDetailViewProps) {
  const { t } = useTranslation();
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);
  const requestIds = useStore((s) => s.sessions[sessionId]?.toolDetailRequestIds);
  const targetRequestId = requestIds?.[0] ?? null;

  const subAgent = useStore((s) => {
    if (!targetRequestId) return null;
    const list = s.activeSubAgents[sessionId] ?? EMPTY_SUB_AGENT_LIST;
    return list.find((a) => a.parentRequestId === targetRequestId) ?? null;
  });

  const scrollRef = useRef<HTMLDivElement>(null);
  const [copiedSection, setCopiedSection] = useState<string | null>(null);
  const isRunning = subAgent?.status === "running";

  // Auto-scroll to bottom when running
  useEffect(() => {
    if (isRunning && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [isRunning, subAgent?.entries.length, subAgent?.streamingText]);

  const handleCopy = async (content: string, section: string) => {
    if (await copyToClipboard(content)) {
      setCopiedSection(section);
      setTimeout(() => setCopiedSection(null), 2000);
    }
  };

  const navigateBack = () => setDetailViewMode(sessionId, "timeline");

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
            {t("ai.toolDetail.backToTerminal")}
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
  const hasEntries = subAgent.entries.length > 0;

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
          {t("ai.toolDetail.backToTerminal")}
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

      {/* Timeline content */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto">
        {/* Task assignment block */}
        {subAgent.task && (
          <div className="px-4 py-3 border-b border-border/30 bg-accent/5">
            <div className="flex items-center justify-between mb-2">
              <span className="text-[10px] font-semibold text-accent uppercase tracking-wider flex items-center gap-1.5">
                <span className="w-1.5 h-1.5 rounded-full bg-accent/60" />
                {t("ai.subAgentDetail.task")}
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => handleCopy(subAgent.task, "task")}
                className="h-5 text-[10px] px-1.5 opacity-60 hover:opacity-100 transition-opacity"
              >
                <Copy className="w-2.5 h-2.5 mr-0.5" />
                {copiedSection === "task" ? t("ai.subAgentDetail.copied") : t("ai.subAgentDetail.copy")}
              </Button>
            </div>
            <div className="text-[12.5px] text-foreground leading-[1.7] [&_p]:mb-2 [&_p:last-child]:mb-0 [&_ul]:my-1.5 [&_ol]:my-1.5">
              <Markdown content={stripAgentXmlTags(subAgent.task)} />
            </div>
          </div>
        )}

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
          {hasEntries ? (
            subAgent.entries.map((entry, i) => {
              if (entry.kind === "text" && entry.text) {
                return <AgentOutputBlock key={`entry-${i}`} text={entry.text} />;
              }
              if (entry.kind === "tool_call" && entry.toolCallId) {
                const tool = toolMap.get(entry.toolCallId);
                if (tool) return <AgentToolCallBlock key={tool.id} tool={tool} />;
              }
              return null;
            })
          ) : subAgent.toolCalls.length > 0 ? (
            subAgent.toolCalls.map((tool) => (
              <AgentToolCallBlock key={tool.id} tool={tool} />
            ))
          ) : null}
        </div>

        {/* Streaming text (live AI output while running) */}
        {isRunning && subAgent.streamingText && (
          <div className="px-4 py-3 bg-accent/[0.04] border-l-2 border-accent/50">
            <div className="text-[12.5px] text-foreground leading-[1.7] [&_p]:mb-2 [&_p:last-child]:mb-0 [&_ul]:my-1.5 [&_ol]:my-1.5 [&_pre]:my-2">
              <Markdown content={stripAgentXmlTags(subAgent.streamingText)} />
            </div>
            <div className="mt-2.5 flex items-center gap-1.5">
              <Loader2 className="w-3 h-3 animate-spin text-accent" />
              <span className="text-[10px] text-accent/70 font-medium">{t("ai.subAgentDetail.liveOutput")}</span>
            </div>
          </div>
        )}

        {/* Final response */}
        {subAgent.response && (
          <div className="px-4 py-3 bg-[var(--success-dim)]/30 border-t border-[var(--success)]/15">
            <div className="flex items-center justify-between mb-2">
              <span className="text-[10px] font-semibold text-[var(--success)] uppercase tracking-wider flex items-center gap-1.5">
                <CheckCircle2 className="w-3 h-3" />
                {t("ai.subAgentDetail.response")}
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => handleCopy(subAgent.response ?? "", "response")}
                className="h-5 text-[10px] px-1.5 opacity-60 hover:opacity-100 transition-opacity"
              >
                <Copy className="w-2.5 h-2.5 mr-0.5" />
                {copiedSection === "response" ? t("ai.subAgentDetail.copied") : t("ai.subAgentDetail.copy")}
              </Button>
            </div>
            <div className="text-[12.5px] text-foreground leading-[1.7] [&_p]:mb-2 [&_p:last-child]:mb-0 [&_ul]:my-1.5 [&_ol]:my-1.5 [&_pre]:my-2 [&_blockquote]:my-2">
              <Markdown content={stripAgentXmlTags(subAgent.response)} />
            </div>
          </div>
        )}

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
          <span className="text-[11px] text-accent/80">
            {t("ai.subAgentDetail.agentRunning")}
          </span>
        </div>
      )}
    </div>
  );
});
