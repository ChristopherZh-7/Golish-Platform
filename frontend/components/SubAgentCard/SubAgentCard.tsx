import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  Loader2,
  Maximize2,
  Terminal,
  Wand2,
  XCircle,
} from "lucide-react";
import { memo, useEffect, useRef, useState, type ReactElement } from "react";
import { Markdown } from "@/components/Markdown";
import { StatusIcon } from "@/components/ui/StatusIcon";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { stripAllAnsi } from "@/lib/ansi";
import { getAgentColor, getAgentIcon } from "@/lib/sub-agent-theme";
import { formatDurationShort } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, SubAgentEntry, SubAgentToolCall } from "@/store";
import { SubAgentDetailsModal } from "./SubAgentDetailsModal";

/**
 * Strip XML wrapper tags, raw function-call XML, and ANSI escape codes
 * from sub-agent text output.
 */
function cleanSubAgentText(text: string): string {
  return stripAllAnsi(
    text
      .replace(/<\/?(task_assignment|original_request|execution_plan|execution_context|prior_knowledge)>/gi, "")
      .replace(/<function=[^>]*>[\s\S]*?(?:<\/function>|$)/g, "")
      .replace(/<parameter=[^>]*>[\s\S]*?<\/parameter>/g, "")
      .replace(/<\/?(?:function|parameter)[^>]*>/g, "")
  ).trim();
}

interface SubAgentCardProps {
  subAgent: ActiveSubAgent;
  autoCollapse?: boolean;
  /** Compact inline style for nesting inside pipeline steps */
  compact?: boolean;
  highlighted?: boolean;
}

/** Status icon component */

/** Status badge component - styled like ToolGroup's running indicator */
function StatusBadge({ status }: { status: string }) {
  if (status !== "running") return null;

  return (
    <Badge
      variant="outline"
      className="ml-auto gap-1 flex items-center text-[10px] px-2 py-0.5 rounded-full bg-[var(--accent-dim)] text-accent"
    >
      <Loader2 className="w-3 h-3 animate-spin" />
      Running
    </Badge>
  );
}

/** Individual tool call row */
const ToolCallRow = memo(function ToolCallRow({ tool }: { tool: SubAgentToolCall }) {
  const isShellCmd = tool.name === "run_pty_cmd" || tool.name === "run_command";
  const [isExpanded, setIsExpanded] = useState(isShellCmd);
  const preRef = useRef<HTMLPreElement>(null);
  const rawStatus = tool.status as string;
  const status: "running" | "completed" | "error" | "interrupted" =
    rawStatus === "completed" ? "completed" : rawStatus === "error" ? "error" : rawStatus === "interrupted" ? "interrupted" : "running";
  const isStreaming = isShellCmd && tool.status === "running" && !!tool.streamingOutput;

  useEffect(() => {
    if (isStreaming && preRef.current) {
      preRef.current.scrollTop = preRef.current.scrollHeight;
    }
  }, [isStreaming, tool.streamingOutput]);

  const primaryArg = (() => {
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
  const argsPreview: string =
    JSON.stringify(tool.args ?? null, null, 2) ?? String(tool.args ?? "");
  const resultPreview: string =
    typeof tool.result === "string"
      ? tool.result
      : (JSON.stringify(tool.result as Record<string, unknown>, null, 2) ?? "");
  const renderArgsSection = (): ReactElement | null => {
    if (isShellCmd) return null;
    return (
      <div>
        <span className="text-muted-foreground">Args:</span>
        <pre className="mt-0.5 rounded bg-muted px-2 py-1 text-[10px]">
          {argsPreview}
        </pre>
      </div>
    );
  };
  const renderResultSection = (): ReactElement | null => {
    if (isShellCmd || tool.result === undefined) return null;
    return (
      <div>
        <span className="text-muted-foreground">Result:</span>
        <pre className="mt-0.5 max-h-40 overflow-auto rounded bg-muted px-2 py-1 text-[10px]">
          {resultPreview}
        </pre>
      </div>
    );
  };

  return (
    <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
      <CollapsibleTrigger className="group flex w-full items-center gap-1.5 rounded px-1.5 py-0.5 text-xs hover:bg-accent/50">
        {isExpanded ? (
          <ChevronDown className="h-3 w-3 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3 w-3 text-muted-foreground" />
        )}
        <StatusIcon status={status} size="sm" />
        {isShellCmd ? (
          <Terminal className="h-3 w-3 text-[var(--ansi-green)] flex-shrink-0" />
        ) : null}
        <span className="font-mono text-[var(--ansi-cyan)]">
          {isShellCmd ? "" : tool.name}
        </span>
        {primaryArg && (
          <span
            className={cn(
              "truncate font-mono",
              isShellCmd ? "text-[var(--ansi-green)]/80" : "text-muted-foreground"
            )}
            title={primaryArg}
          >
            {isShellCmd && <span className="text-muted-foreground/50 mr-1">$</span>}
            {primaryArg}
          </span>
        )}
        {tool.completedAt && (
          <span className="ml-auto text-[10px] text-muted-foreground">
            {formatDurationShort(
              new Date(tool.completedAt).getTime() - new Date(tool.startedAt).getTime()
            )}
          </span>
        )}
      </CollapsibleTrigger>
      <CollapsibleContent className="px-4 py-1">
        <div className="space-y-1 text-xs">
          {/* Shell command output */}
          {isShellCmd && shellOutput && (
            <pre
              ref={preRef}
              className={cn(
                "max-h-48 overflow-auto whitespace-pre-wrap rounded bg-[var(--ansi-black)]/20 px-2 py-1.5 text-[10px] font-mono text-foreground/80",
                isStreaming && "border-l-2 border-[var(--ansi-blue)]",
              )}
            >
              {shellOutput.length > 3000
                ? `${shellOutput.slice(0, 3000)}\n... (truncated)`
                : shellOutput}
            </pre>
          )}

          {/* Non-shell arguments */}
          {renderArgsSection() as any}

          {/* Non-shell result */}
          {renderResultSection() as any}

          {/* Shell error output */}
          {isShellCmd && tool.result && typeof tool.result === "object" && (tool.result as Record<string, unknown>).error && (
            <div className="rounded bg-[var(--ansi-red)]/10 px-2 py-1 text-[10px] text-[var(--ansi-red)]">
              {String((tool.result as Record<string, unknown>).error)}
            </div>
          )}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
});

/** Number of entries to show by default (from the end) */
const VISIBLE_TAIL_ENTRIES = 8;

/** Render a text entry inline with markdown parsing */
function TextEntryRow({ text }: { text: string }) {
  const cleaned = cleanSubAgentText(text);
  if (!cleaned) return null;
  return (
    <div className="text-xs text-muted-foreground px-1.5 border-l-2 border-accent/40 ml-1 my-1 line-clamp-6 overflow-hidden">
      <Markdown content={cleaned} className="text-xs [&_p]:mb-1 [&_p]:last:mb-0" />
    </div>
  );
}

/** Render interleaved entries (text + tool calls) */
function InterleavedEntries({
  entries,
  toolCalls,
  showAll,
  onToggleShowAll,
}: {
  entries: SubAgentEntry[];
  toolCalls: SubAgentToolCall[];
  showAll: boolean;
  onToggleShowAll?: () => void;
}) {
  const toolMap = new Map(toolCalls.map((t) => [t.id, t]));
  const totalEntries = entries.length;
  const hiddenCount = showAll ? 0 : Math.max(0, totalEntries - VISIBLE_TAIL_ENTRIES);
  const visibleEntries = showAll ? entries : entries.slice(-VISIBLE_TAIL_ENTRIES);

  return (
    <div className="space-y-0.5">
      {hiddenCount > 0 && onToggleShowAll && (
        <button
          type="button"
          onClick={onToggleShowAll}
          className="flex w-full items-center gap-1.5 rounded px-1.5 py-1 text-xs text-muted-foreground hover:bg-accent/50 hover:text-foreground"
        >
          <ChevronRight className="h-3 w-3" />
          <span>{hiddenCount} previous entries</span>
        </button>
      )}
      {showAll && hiddenCount > 0 && onToggleShowAll && (
        <button
          type="button"
          onClick={onToggleShowAll}
          className="flex w-full items-center gap-1.5 rounded px-1.5 py-1 text-xs text-muted-foreground hover:bg-accent/50 hover:text-foreground"
        >
          <ChevronDown className="h-3 w-3" />
          <span>Hide {totalEntries - VISIBLE_TAIL_ENTRIES} entries</span>
        </button>
      )}
      {visibleEntries.map((entry, i) => {
        if (entry.kind === "text") {
          return entry.text ? <TextEntryRow key={`text-${i}`} text={entry.text} /> : null;
        }
        const tool = entry.toolCallId ? toolMap.get(entry.toolCallId) : undefined;
        return tool ? <ToolCallRow key={tool.id} tool={tool} /> : null;
      })}
    </div>
  );
}

/** Compact inline sub-agent row for nesting inside pipeline steps */
const CompactSubAgentCard = memo(function CompactSubAgentCard({
  subAgent,
}: { subAgent: ActiveSubAgent }) {
  const [isExpanded, setIsExpanded] = useState(subAgent.status === "running");
  const [showAll, setShowAll] = useState(false);
  const [showDetailsModal, setShowDetailsModal] = useState(false);
  const agentColor = getAgentColor(subAgent.agentName);
  const AgentIcon = getAgentIcon(subAgent.agentName);
  const totalToolCalls = subAgent.toolCalls.length;
  const hasEntries = subAgent.entries.length > 0;

  return (
    <>
    <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
      <div className="flex items-center gap-0.5">
        <CollapsibleTrigger className="group flex flex-1 items-center gap-1.5 rounded px-1 py-0.5 text-xs hover:bg-accent/50 min-w-0">
          {isExpanded ? (
            <ChevronDown className="h-3 w-3 text-muted-foreground flex-shrink-0" />
          ) : (
            <ChevronRight className="h-3 w-3 text-muted-foreground flex-shrink-0" />
          )}
          <StatusIcon status={subAgent.status} size="sm" />
          <AgentIcon className="h-3 w-3 flex-shrink-0" style={{ color: agentColor }} />
          <span className="font-mono text-[var(--ansi-cyan)] truncate">
            {subAgent.agentName || subAgent.agentId}
          </span>
          {totalToolCalls > 0 && (
            <span className="text-[10px] text-muted-foreground flex-shrink-0">
              {totalToolCalls} tool{totalToolCalls > 1 ? "s" : ""}
            </span>
          )}
          {subAgent.durationMs !== undefined && (
            <span className="ml-auto text-[10px] text-muted-foreground flex-shrink-0">
              {formatDurationShort(subAgent.durationMs)}
            </span>
          )}
        </CollapsibleTrigger>
        <button
          type="button"
          onClick={() => setShowDetailsModal(true)}
          className="p-0.5 hover:bg-accent/50 rounded transition-colors flex-shrink-0"
          title="View details"
        >
          <Maximize2 className="w-3 h-3 text-muted-foreground hover:text-foreground" />
        </button>
      </div>

      <CollapsibleContent className="pl-5 pr-1 pb-0.5">
        {hasEntries ? (
          <InterleavedEntries
            entries={subAgent.entries}
            toolCalls={subAgent.toolCalls}
            showAll={showAll}
            onToggleShowAll={() => setShowAll((v) => !v)}
          />
        ) : (
          subAgent.toolCalls.slice(-3).map((tool) => (
            <ToolCallRow key={tool.id} tool={tool} />
          ))
        )}
              {subAgent.status === "completed" && subAgent.response && (
                <div className="text-[10px] text-muted-foreground line-clamp-2 border-t border-border/30 pt-0.5 mt-0.5">
                  <Markdown
                    content={cleanSubAgentText(
                      subAgent.response.length > 200
                        ? `${subAgent.response.slice(0, 200)}...`
                        : subAgent.response
                    )}
                    className="text-[10px] [&_p]:mb-0"
                  />
                </div>
              )}
        {subAgent.error && (
          <div className="text-[10px] text-[var(--ansi-red)] mt-0.5">Error: {subAgent.error}</div>
        )}
      </CollapsibleContent>
    </Collapsible>
    {showDetailsModal && (
      <SubAgentDetailsModal subAgent={subAgent} onClose={() => setShowDetailsModal(false)} />
    )}
    </>
  );
});

/** Sub-agent card component */
export const SubAgentCard = memo(function SubAgentCard({
  subAgent,
  autoCollapse,
  compact,
  highlighted = false,
}: SubAgentCardProps) {
  if (compact) {
    return <CompactSubAgentCard subAgent={subAgent} />;
  }

  return (
    <FullSubAgentCard
      subAgent={subAgent}
      autoCollapse={autoCollapse}
      highlighted={highlighted}
    />
  );
});

const FullSubAgentCard = memo(function FullSubAgentCard({
  subAgent,
  autoCollapse,
  highlighted = false,
}: Omit<SubAgentCardProps, "compact">) {
  const defaultExpanded = autoCollapse ? false : highlighted ? true : true;
  const [isExpanded, setIsExpanded] = useState(defaultExpanded);
  const prevStatusRef = useRef(subAgent.status);
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (highlighted) {
      setIsExpanded(true);
      cardRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }
  }, [highlighted]);

  useEffect(() => {
    const prev = prevStatusRef.current;
    prevStatusRef.current = subAgent.status;
    if (prev === "running" && subAgent.status !== "running") {
      setIsExpanded(false);
    }
  }, [subAgent.status]);
  const [showAllEntries, setShowAllEntries] = useState(false);
  const [showDetailsModal, setShowDetailsModal] = useState(false);

  const totalToolCalls = subAgent.toolCalls.length;
  const hasEntries = subAgent.entries.length > 0;

  const hasExpandableContent =
    hasEntries || totalToolCalls > 0 || !!subAgent.error || !!subAgent.promptGeneration || !!subAgent.task;

  const agentColor = getAgentColor(subAgent.agentName);
  const AgentIcon = getAgentIcon(subAgent.agentName);
  const isInterrupted = subAgent.status === "interrupted";
  const isRunning = subAgent.status === "running";
  const isError = subAgent.status === "error";
  const nestingDepth = Math.max(0, (subAgent.depth ?? 1) - 1);

  return (
    <>
      <div
        ref={cardRef}
        data-agent-block={`sub-agent-${subAgent.parentRequestId}`}
        className={cn(
          "mt-1 mb-1.5 rounded-lg overflow-hidden transition-all scroll-mt-4 bg-card",
          isRunning ? "border-l-2 border border-border" : "border border-border",
          highlighted && "ring-1 ring-accent/50 bg-accent/5",
          "target:ring-1 target:ring-accent/40"
        )}
        style={{
          marginLeft: nestingDepth > 0 ? nestingDepth * 16 : undefined,
          ...(isRunning
            ? { borderLeftColor: agentColor, boxShadow: `inset 2px 0 8px -4px ${agentColor}40` }
            : isError
              ? { borderLeftWidth: 2, borderLeftColor: "rgb(239 68 68 / 0.7)" }
              : isInterrupted
                ? { borderLeftWidth: 2, borderLeftColor: "rgb(245 158 11 / 0.6)" }
                : undefined),
        }}
      >
        {hasExpandableContent ? (
          <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
            <div className="flex items-center gap-2 px-3 py-2">
              <CollapsibleTrigger className="flex flex-1 items-center gap-2 hover:bg-accent/30 rounded -ml-1 pl-1 py-0.5 min-w-0">
                {isExpanded ? (
                  <ChevronDown className="h-3.5 w-3.5 text-muted-foreground flex-shrink-0" />
                ) : (
                  <ChevronRight className="h-3.5 w-3.5 text-muted-foreground flex-shrink-0" />
                )}
                <AgentIcon
                  className="h-4 w-4 flex-shrink-0"
                  style={{ color: agentColor }}
                />
                <span className="font-medium text-sm truncate">
                  {subAgent.agentName || subAgent.agentId}
                </span>
                <StatusBadge status={subAgent.status} />
                {totalToolCalls > 0 && (
                  <span className="text-[10px] text-muted-foreground flex-shrink-0">
                    {totalToolCalls} tool{totalToolCalls > 1 ? "s" : ""}
                  </span>
                )}
              </CollapsibleTrigger>
              <div className="flex items-center gap-2 flex-shrink-0">
                {subAgent.durationMs !== undefined && (
                  <span className="text-xs text-muted-foreground">
                    {formatDurationShort(subAgent.durationMs)}
                  </span>
                )}
                <button
                  type="button"
                  onClick={() => setShowDetailsModal(true)}
                  className="p-1 hover:bg-accent/50 rounded transition-colors"
                  title="View details"
                >
                  <Maximize2 className="w-3.5 h-3.5 text-muted-foreground hover:text-foreground" />
                </button>
              </div>
            </div>

            <CollapsibleContent className="px-3 pb-2">
              {/* Task description */}
              {subAgent.task && (
                <div className="mb-1.5 text-xs text-muted-foreground line-clamp-2 px-1.5">
                  <Markdown content={cleanSubAgentText(subAgent.task)} className="text-xs [&_p]:mb-0" />
                </div>
              )}

              {/* Prompt generation info */}
              {subAgent.promptGeneration && (
                <div className="mb-1.5">
                  <Collapsible>
                    <CollapsibleTrigger className="group flex w-full items-center gap-1.5 rounded px-1.5 py-0.5 text-xs hover:bg-accent/50">
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
                          ? "Generating system prompt..."
                          : subAgent.promptGeneration.status === "completed"
                            ? "System prompt generated"
                            : "Prompt generation failed"}
                      </span>
                      {subAgent.promptGeneration.durationMs !== undefined && (
                        <span className="ml-auto text-[10px] text-muted-foreground flex items-center gap-0.5">
                          <Clock className="h-2.5 w-2.5" />
                          {formatDurationShort(subAgent.promptGeneration.durationMs)}
                        </span>
                      )}
                    </CollapsibleTrigger>
                    <CollapsibleContent className="px-4 py-1">
                      <div className="space-y-1.5 text-xs">
                        <details className="group">
                          <summary className="cursor-pointer select-none text-muted-foreground hover:text-foreground/80">
                            Architect system prompt
                          </summary>
                          <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-muted px-2 py-1 text-[10px]">
                            {subAgent.promptGeneration.architectSystemPrompt}
                          </pre>
                        </details>
                        <details className="group">
                          <summary className="cursor-pointer select-none text-muted-foreground hover:text-foreground/80">
                            Task input
                          </summary>
                          <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-muted px-2 py-1 text-[10px]">
                            {subAgent.promptGeneration.architectUserMessage}
                          </pre>
                        </details>
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

              {/* Interleaved text + tool calls */}
              {hasEntries ? (
                <InterleavedEntries
                  entries={subAgent.entries}
                  toolCalls={subAgent.toolCalls}
                  showAll={showAllEntries}
                  onToggleShowAll={() => setShowAllEntries((v) => !v)}
                />
              ) : totalToolCalls > 0 ? (
                <div className="space-y-0.5">
                  {subAgent.toolCalls.map((tool) => (
                    <ToolCallRow key={tool.id} tool={tool} />
                  ))}
                </div>
              ) : null}

              {/* Response preview (completed agents) */}
              {subAgent.status === "completed" && subAgent.response && (
                <div className="mt-1.5 text-xs text-muted-foreground line-clamp-3 px-1.5 border-t border-border/40 pt-1.5">
                  <Markdown
                    content={cleanSubAgentText(
                      subAgent.response.length > 300
                        ? `${subAgent.response.slice(0, 300)}...`
                        : subAgent.response
                    )}
                    className="text-xs [&_p]:mb-0"
                  />
                </div>
              )}

              {/* Error indicator */}
              {subAgent.error && (
                <div className="mt-2 rounded bg-[var(--ansi-red)]/10 px-2 py-1.5 text-xs text-[var(--ansi-red)]">
                  <span className="font-medium">Error: </span>
                  {subAgent.error}
                </div>
              )}
            </CollapsibleContent>
          </Collapsible>
        ) : (
          <div className="flex items-center gap-2 px-3 py-2">
            <div className="flex flex-1 items-center gap-2 -ml-1 pl-1 py-0.5 min-w-0">
              <AgentIcon
                className="h-4 w-4 flex-shrink-0"
                style={{ color: agentColor }}
              />
              <span className="font-medium text-sm truncate">
                {subAgent.agentName || subAgent.agentId}
              </span>
              <StatusBadge status={subAgent.status} />
            </div>
            <div className="flex items-center gap-2 flex-shrink-0">
              {subAgent.durationMs !== undefined && (
                <span className="text-xs text-muted-foreground">
                  {formatDurationShort(subAgent.durationMs)}
                </span>
              )}
              <button
                type="button"
                onClick={() => setShowDetailsModal(true)}
                className="p-1 hover:bg-accent/50 rounded transition-colors"
                title="View details"
              >
                <Maximize2 className="w-3.5 h-3.5 text-muted-foreground hover:text-foreground" />
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Details Modal */}
      {showDetailsModal && (
        <SubAgentDetailsModal subAgent={subAgent} onClose={() => setShowDetailsModal(false)} />
      )}
    </>
  );
});
