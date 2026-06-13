/**
 * StageRunDetail — left-pane detail view for an active stage run
 * (设计 2026-06-13-stage-run-fanout §3.3). Rendered by PaneLeaf when
 * `detailViewMode === "stage-run"`; reads the session's `stageRun` state and
 * renders {@link StageRunView}. Mirrors how SubAgentDetailView / ToolCallDetailView
 * own the pane, with a back button to return to the chat timeline.
 */

import { ArrowLeft } from "lucide-react";
import { useStore } from "@/store";
import { StageRunView } from "./StageRunView";

export function StageRunDetail({ sessionId }: { sessionId: string }) {
  const stageRun = useStore((s) => s.sessions[sessionId]?.stageRun ?? null);
  const toggleRow = useStore((s) => s.toggleStageRunRow);
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b border-border/60 bg-background/60 px-3 py-2 text-[12px]">
        <button
          type="button"
          onClick={() => setDetailViewMode(sessionId, "timeline")}
          className="flex items-center gap-1 text-muted-foreground/80 hover:text-foreground"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          返回
        </button>
        <span className="font-semibold">
          详情 · {stageRun ? `${stageRun.stageLabel} 流水线` : "Stage run"}
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto py-2">
        {stageRun ? (
          <StageRunView
            rows={stageRun.rows}
            summary={stageRun.summary}
            concurrency={stageRun.concurrency}
            stageLabel={stageRun.stageLabel}
            stageTag={stageRun.stageTag}
            roleLabel={stageRun.roleLabel}
            coverageAxis={stageRun.coverageAxis}
            onToggleRow={(id) => toggleRow(sessionId, id)}
          />
        ) : (
          <div className="flex h-full items-center justify-center px-6 text-center text-[12px] text-muted-foreground/50">
            当前没有进行中的 stage run。
          </div>
        )}
      </div>
    </div>
  );
}
