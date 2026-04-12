import { ArrowLeft, Loader2 } from "lucide-react";
import { memo, useMemo } from "react";
import { SubAgentCard } from "@/components/SubAgentCard";
import { ToolExecutionCard } from "@/components/ToolExecutionCard";
import type { ActiveSubAgent, AiToolExecution } from "@/store";
import { useStore } from "@/store";

type DetailItem =
  | { kind: "tool"; data: AiToolExecution }
  | { kind: "sub_agent"; data: ActiveSubAgent };

interface ToolDetailViewProps {
  sessionId: string;
}

export const ToolDetailView = memo(function ToolDetailView({
  sessionId,
}: ToolDetailViewProps) {
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);
  const timeline = useStore((s) => s.timelines[sessionId] ?? []);
  const filterIds = useStore(
    (s) => s.sessions[sessionId]?.toolDetailRequestIds ?? null
  );

  const items = useMemo(() => {
    const result: DetailItem[] = [];
    const idSet = filterIds ? new Set(filterIds) : null;
    for (const block of timeline) {
      if (block.type === "ai_tool_execution") {
        if (!idSet || idSet.has(block.data.requestId)) {
          result.push({ kind: "tool", data: block.data });
        }
      } else if (block.type === "sub_agent_activity") {
        result.push({ kind: "sub_agent", data: block.data });
      }
    }
    return result;
  }, [timeline, filterIds]);

  const runningCount = items.filter((item) => {
    if (item.kind === "tool") return item.data.status === "running";
    return item.data.status === "running";
  }).length;

  const totalCount = items.length;
  const doneCount = items.filter((item) => {
    if (item.kind === "tool") return item.data.status === "completed" || item.data.status === "error";
    return item.data.status === "completed" || item.data.status === "error";
  }).length;

  return (
    <div className="h-full flex flex-col bg-background">
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-[var(--border-subtle)]">
        <button
          type="button"
          onClick={() => setDetailViewMode(sessionId, "timeline")}
          className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          <ArrowLeft className="w-3.5 h-3.5" />
          Back to Terminal
        </button>
        <div className="flex-1" />
        <span className="text-[10px] text-muted-foreground tabular-nums">
          {doneCount}/{totalCount} done
        </span>
      </div>

      {/* Execution list */}
      <div className="flex-1 overflow-y-auto px-3 py-2">
        {items.length === 0 ? (
          <div className="h-full flex items-center justify-center text-muted-foreground/50 text-sm">
            No tool executions yet
          </div>
        ) : (
          <div className="space-y-1">
            {items.map((item) =>
              item.kind === "tool" ? (
                <ToolExecutionCard key={item.data.requestId} execution={item.data} />
              ) : (
                <SubAgentCard key={item.data.parentRequestId} subAgent={item.data} />
              )
            )}
          </div>
        )}
      </div>

      {/* Footer — running count */}
      {runningCount > 0 && (
        <div className="px-4 py-2 border-t border-[var(--border-subtle)] bg-blue-500/5">
          <div className="flex items-center gap-2">
            <Loader2 className="w-3 h-3 text-blue-400 animate-spin" />
            <span className="text-[11px] text-blue-400/80">
              {runningCount} running...
            </span>
          </div>
        </div>
      )}
    </div>
  );
});
