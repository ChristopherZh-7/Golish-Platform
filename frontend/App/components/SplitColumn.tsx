import type React from "react";
import { memo } from "react";
import { cn } from "@/lib/utils";
import { PaneContainer } from "../../components/PaneContainer";
import { useStore } from "../../store";
import type { TabLayoutInfo } from "../../store/selectors";
import type { useSplitTabDrag } from "../hooks/useSplitTabDrag";

interface SplitColumnProps {
  rightPanelTabs: string[];
  rightActiveTab: string | null;
  setRightActiveTab: React.Dispatch<React.SetStateAction<string | null>>;
  tabLayouts: TabLayoutInfo[];
  closeRightTab: (tabId?: string) => void;
  splitDrag: ReturnType<typeof useSplitTabDrag>;
}

export const SplitColumn = memo(function SplitColumn({
  rightPanelTabs,
  rightActiveTab,
  setRightActiveTab,
  tabLayouts,
  closeRightTab,
  splitDrag,
}: SplitColumnProps) {
  return (
    <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float animate-in slide-in-from-right-4 fade-in duration-300">
      <div className="h-[34px] flex items-center border-b border-border/30 flex-shrink-0 overflow-x-auto">
        {rightPanelTabs.map((rTabId) => {
          const rSession = useStore.getState().sessions[rTabId];
          const rName =
            rSession?.customName ||
            rSession?.processName ||
            rSession?.workingDirectory?.split(/[/\\]/).pop() ||
            "Terminal";
          const isRightActive = rTabId === rightActiveTab;
          return (
            <div
              key={rTabId}
              className={cn(
                "flex items-center gap-1.5 px-3 py-1 h-full relative cursor-grab active:cursor-grabbing select-none text-[11px] font-mono transition-colors",
                isRightActive
                  ? "bg-muted text-foreground"
                  : "text-muted-foreground hover:text-foreground hover:bg-muted/50",
              )}
              onClick={() => setRightActiveTab(rTabId)}
              onPointerDown={(e) => splitDrag.onPointerDown(e, rTabId)}
              onPointerMove={(e) => splitDrag.onPointerMove(e, rName)}
              onPointerUp={(e) => splitDrag.onPointerUp(e)}
            >
              {isRightActive && (
                <span className="absolute bottom-0 left-0 right-0 h-px bg-accent" />
              )}
              <span className="truncate max-w-[140px]">{rName}</span>
              <button
                className="p-0.5 rounded hover:bg-background/50 text-muted-foreground hover:text-foreground transition-colors flex-shrink-0"
                onClick={(e) => {
                  e.stopPropagation();
                  closeRightTab(rTabId);
                }}
                title="Close split"
                onPointerDown={(e) => e.stopPropagation()}
              >
                <svg
                  width="8"
                  height="8"
                  viewBox="0 0 10 10"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  strokeLinecap="round"
                >
                  <line x1="2" y1="2" x2="8" y2="8" />
                  <line x1="8" y1="2" x2="2" y2="8" />
                </svg>
              </button>
            </div>
          );
        })}
      </div>
      <div className="flex-1 min-h-0 min-w-0 relative">
        {rightPanelTabs.map((rTabId) => {
          const layout = tabLayouts.find((l) => l.tabId === rTabId);
          if (!layout) return null;
          return (
            <div
              key={rTabId}
              className={`absolute inset-0 ${rTabId === rightActiveTab ? "visible" : "invisible pointer-events-none"}`}
            >
              <PaneContainer node={layout.root} tabId={rTabId} />
            </div>
          );
        })}
      </div>
    </div>
  );
});

export function SplitDropZone() {
  return (
    <div className="absolute inset-0 z-20 pointer-events-none flex animate-in fade-in duration-200">
      <div className="flex-1" />
      <div className="w-1/2 m-2 rounded-xl border-2 border-dashed border-accent/60 bg-accent/8 flex items-center justify-center backdrop-blur-[2px] animate-in slide-in-from-right-2 duration-300">
        <div className="flex flex-col items-center gap-1.5 animate-pulse">
          <svg
            width="28"
            height="28"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            className="text-accent/60"
          >
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <line x1="12" y1="3" x2="12" y2="21" />
          </svg>
          <span className="text-xs text-accent/60 font-medium">Split Right</span>
        </div>
      </div>
    </div>
  );
}
