import {
  Bot,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  Code2,
  Download,
  FileCode2,
  Loader2,
  Maximize2,
  Search,
  Settings2,
  Terminal,
  AlertTriangle,
  Circle,
  Wand2,
  XCircle,
} from "lucide-react";
import { memo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, SubAgentEntry, SubAgentToolCall } from "@/store";
import { SubAgentDetailsModal } from "./SubAgentDetailsModal";

interface SubAgentCardProps {
  subAgent: ActiveSubAgent;
  autoCollapse?: boolean;
  /** Compact inline style for nesting inside pipeline steps */
  compact?: boolean;
}

const AGENT_COLORS: Record<string, string> = {
  planner: "var(--ansi-blue)",
  coder: "var(--ansi-green)",
  researcher: "var(--ansi-yellow)",
  reviewer: "var(--ansi-cyan)",
  explorer: "var(--ansi-yellow)",
  analyst: "var(--ansi-cyan)",
  js_harvester: "#f59e0b",
  js_analyzer: "#f59e0b",
  executor: "var(--ansi-magenta)",
};

const AGENT_ICONS: Record<string, typeof Bot> = {
  coder: Code2,
  researcher: Search,
  explorer: Search,
  planner: Settings2,
  js_harvester: Download,
  js_analyzer: FileCode2,
  executor: Terminal,
};

function getAgentColor(agentName: string): string {
  const lower = agentName.toLowerCase();
  for (const [key, color] of Object.entries(AGENT_COLORS)) {
    if (lower.includes(key)) return color;
  }
  return "var(--ansi-magenta)";
}

function getAgentIcon(agentName: string): typeof Bot {
  const lower = agentName.toLowerCase();
  for (const [key, icon] of Object.entries(AGENT_ICONS)) {
    if (lower.includes(key)) return icon;
  }
  return Bot;
}

/** Status icon component */
function StatusIcon({
  status,
  size = "md",
}: {
  status: string;
  size?: "sm" | "md";
}) {
  const sizeClass = size === "sm" ? "w-3 h-3" : "w-4 h-4";

  switch (status) {
    case "completed":
      return <CheckCircle2 className={cn(sizeClass, "text-[var(--ansi-green)]")} />;
    case "running":
      return <Loader2 className={cn(sizeClass, "text-[var(--ansi-blue)] animate-spin")} />;
    case "error":
      return <XCircle className={cn(sizeClass, "text-[var(--ansi-red)]")} />;
    case "interrupted":
      return <AlertTriangle className={cn(sizeClass, "text-amber-400/60")} />;
    default:
      return <Circle className={cn(sizeClass, "text-muted-foreground/40")} />;
  }
}

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

/** Format duration in ms to human readable */
function formatDuration(ms?: number): string {
  if (!ms) return "";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** Individual tool call row */
const ToolCallRow = memo(function ToolCallRow({ tool }: { tool: SubAgentToolCall }) {
  const isShellCmd = tool.name === "run_pty_cmd" || tool.name === "run_command";
  const [isExpanded, setIsExpanded] = useState(isShellCmd);
  const status =
    tool.status === "completed" ? "completed" : tool.status === "error" ? "error" : tool.status === "interrupted" ? "interrupted" : "running";

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

  const shellOutput = (() => {
    if (!isShellCmd || !tool.result || typeof tool.result !== "object") return null;
    const r = tool.result as Record<string, unknown>;
    return (r.stdout as string) || (r.output as string) || null;
  })();

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
            {formatDuration(
              new Date(tool.completedAt).getTime() - new Date(tool.startedAt).getTime()
            )}
          </span>
        )}
      </CollapsibleTrigger>
      <CollapsibleContent className="px-4 py-1">
        <div className="space-y-1 text-xs">
          {/* Shell command output */}
          {isShellCmd && shellOutput && (
            <pre className="max-h-48 overflow-auto whitespace-pre-wrap rounded bg-[var(--ansi-black)]/20 px-2 py-1.5 text-[10px] font-mono text-foreground/80">
              {shellOutput.length > 3000
                ? `${shellOutput.slice(0, 3000)}\n... (truncated)`
                : shellOutput}
            </pre>
          )}

          {/* Non-shell arguments */}
          {!isShellCmd && (
            <div>
              <span className="text-muted-foreground">Args:</span>
              <pre className="mt-0.5 rounded bg-muted px-2 py-1 text-[10px]">
                {JSON.stringify(tool.args, null, 2)}
              </pre>
            </div>
          )}

          {/* Non-shell result */}
          {!isShellCmd && tool.result !== undefined && (
            <div>
              <span className="text-muted-foreground">Result:</span>
              <pre className="mt-0.5 max-h-40 overflow-auto rounded bg-muted px-2 py-1 text-[10px]">
                {typeof tool.result === "string"
                  ? tool.result
                  : JSON.stringify(tool.result, null, 2)}
              </pre>
            </div>
          )}

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

/** Render a text entry inline */
function TextEntryRow({ text }: { text: string }) {
  if (!text.trim()) return null;
  return (
    <div className="text-xs text-muted-foreground/80 px-1.5 border-l-2 border-accent/30 ml-1 my-1">
      <span className="whitespace-pre-wrap line-clamp-6">{text}</span>
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
  const agentColor = getAgentColor(subAgent.agentName);
  const AgentIcon = getAgentIcon(subAgent.agentName);
  const totalToolCalls = subAgent.toolCalls.length;
  const hasEntries = subAgent.entries.length > 0;

  return (
    <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
      <CollapsibleTrigger className="group flex w-full items-center gap-1.5 rounded px-1 py-0.5 text-xs hover:bg-accent/50">
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
            {formatDuration(subAgent.durationMs)}
          </span>
        )}
      </CollapsibleTrigger>

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
          <div className="text-[10px] text-muted-foreground/60 line-clamp-2 border-t border-border/20 pt-0.5 mt-0.5">
            {subAgent.response.length > 200
              ? `${subAgent.response.slice(0, 200)}...`
              : subAgent.response}
          </div>
        )}
        {subAgent.error && (
          <div className="text-[10px] text-[var(--ansi-red)] mt-0.5">Error: {subAgent.error}</div>
        )}
      </CollapsibleContent>
    </Collapsible>
  );
});

/** Sub-agent card component */
export const SubAgentCard = memo(function SubAgentCard({
  subAgent,
  autoCollapse,
  compact,
}: SubAgentCardProps) {
  if (compact) {
    return <CompactSubAgentCard subAgent={subAgent} />;
  }

  const defaultExpanded = autoCollapse ? false : true;
  const [isExpanded, setIsExpanded] = useState(defaultExpanded);
  const [showAllEntries, setShowAllEntries] = useState(false);
  const [showDetailsModal, setShowDetailsModal] = useState(false);

  const totalToolCalls = subAgent.toolCalls.length;
  const hasEntries = subAgent.entries.length > 0;

  const hasExpandableContent =
    hasEntries || totalToolCalls > 0 || !!subAgent.error || !!subAgent.promptGeneration || !!subAgent.task;

  const agentColor = getAgentColor(subAgent.agentName);
  const AgentIcon = getAgentIcon(subAgent.agentName);

  return (
    <>
      <div
        data-agent-block={`sub-agent-${subAgent.parentRequestId}`}
        className={cn(
          "mt-1 mb-1.5 rounded-lg border bg-card overflow-hidden transition-all scroll-mt-4",
          subAgent.status === "running"
            ? "border-l-2 animate-[pulse-border_2s_ease-in-out_infinite]"
            : "border border-border",
          "target:ring-2 target:ring-accent/40 target:ring-offset-1 target:ring-offset-background"
        )}
        style={subAgent.status === "running" ? {
          borderLeftColor: agentColor,
          boxShadow: `inset 2px 0 8px -4px ${agentColor}40`,
        } : undefined}
      >
        {hasExpandableContent ? (
          <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
            <div className="flex items-center gap-2 px-3 py-2">
              <CollapsibleTrigger className="flex flex-1 items-center gap-2 hover:bg-accent/30 rounded -ml-1 pl-1 py-0.5 min-w-0">
                {isExpanded ? (
                  <ChevronDown className="h-4 w-4 text-muted-foreground flex-shrink-0" />
                ) : (
                  <ChevronRight className="h-4 w-4 text-muted-foreground flex-shrink-0" />
                )}
                <AgentIcon className="h-4 w-4 flex-shrink-0" style={{ color: agentColor }} />
                <span className="font-medium text-sm truncate">
                  {subAgent.agentName || subAgent.agentId}
                </span>
                <StatusBadge status={subAgent.status} />
                {subAgent.depth > 1 && (
                  <Badge variant="outline" className="text-[10px] px-1.5 py-0 flex-shrink-0">
                    depth {subAgent.depth}
                  </Badge>
                )}
                {totalToolCalls > 0 && (
                  <span className="text-[10px] text-muted-foreground flex-shrink-0">
                    {totalToolCalls} tool{totalToolCalls > 1 ? "s" : ""}
                  </span>
                )}
              </CollapsibleTrigger>
              <div className="flex items-center gap-2 flex-shrink-0">
                {subAgent.durationMs !== undefined && (
                  <span className="text-xs text-muted-foreground">
                    {formatDuration(subAgent.durationMs)}
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
                  {subAgent.task}
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
                          {formatDuration(subAgent.promptGeneration.durationMs)}
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
                <div className="mt-1.5 text-xs text-muted-foreground/80 line-clamp-3 px-1.5 border-t border-border/30 pt-1.5">
                  {subAgent.response.length > 300
                    ? `${subAgent.response.slice(0, 300)}...`
                    : subAgent.response}
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
              <AgentIcon className="h-4 w-4 flex-shrink-0" style={{ color: agentColor }} />
              <span className="font-medium text-sm truncate">
                {subAgent.agentName || subAgent.agentId}
              </span>
              <StatusBadge status={subAgent.status} />
              {subAgent.depth > 1 && (
                <Badge variant="outline" className="text-[10px] px-1.5 py-0 flex-shrink-0">
                  depth {subAgent.depth}
                </Badge>
              )}
            </div>
            <div className="flex items-center gap-2 flex-shrink-0">
              {subAgent.durationMs !== undefined && (
                <span className="text-xs text-muted-foreground">
                  {formatDuration(subAgent.durationMs)}
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
