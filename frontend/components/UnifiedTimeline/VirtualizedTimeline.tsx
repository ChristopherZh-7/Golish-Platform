import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef } from "react";
import { TimelineBlockErrorBoundary } from "@/components/TimelineBlockErrorBoundary";
import { estimateBlockHeight } from "@/lib/timeline/blockHeightEstimation";
import type { UnifiedBlock as UnifiedBlockType, ActiveSubAgent } from "@/store";
import { UnifiedBlock } from "./UnifiedBlock";

const virtualItemBaseStyle = {
  position: "absolute",
  top: 0,
  left: 0,
  width: "100%",
} as const;

interface VirtualizedTimelineProps {
  blocks: UnifiedBlockType[];
  sessionId: string;
  containerRef: React.RefObject<HTMLDivElement | null>;
  shouldScrollToBottom: boolean;
  workingDirectory: string;
}

const VIRTUALIZATION_THRESHOLD = 50;

type SubAgentBlock = UnifiedBlockType & { type: "sub_agent_activity"; data: ActiveSubAgent; batchId?: string };

/**
 * Groups consecutive sub_agent_activity blocks by batchId.
 * Blocks are split into separate groups when batchId differs or a non-agent block intervenes.
 */
function computeSubAgentGroups(blocks: UnifiedBlockType[]): Map<number, { isLeader: true; agents: ActiveSubAgent[] } | { isHidden: true }> {
  const groups = new Map<number, { isLeader: true; agents: ActiveSubAgent[] } | { isHidden: true }>();
  let i = 0;
  while (i < blocks.length) {
    if (blocks[i].type === "sub_agent_activity") {
      const start = i;
      const agents: ActiveSubAgent[] = [];
      const firstBlock = blocks[i] as SubAgentBlock;
      const batchId = firstBlock.batchId;
      agents.push(firstBlock.data);
      i++;

      while (i < blocks.length && blocks[i].type === "sub_agent_activity") {
        const block = blocks[i] as SubAgentBlock;
        if (batchId && block.batchId !== batchId) break;
        agents.push(block.data);
        i++;
      }

      if (agents.length > 1) {
        groups.set(start, { isLeader: true, agents });
        for (let j = start + 1; j < start + agents.length; j++) {
          groups.set(j, { isHidden: true });
        }
      }
    } else {
      i++;
    }
  }
  return groups;
}

export const VirtualizedTimeline = memo(function VirtualizedTimeline({
  blocks,
  sessionId,
  containerRef,
  shouldScrollToBottom,
  workingDirectory,
}: VirtualizedTimelineProps) {
  const groupInfo = useMemo(() => computeSubAgentGroups(blocks), [blocks]);

  const virtualizer = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => containerRef.current,
    estimateSize: (index) => {
      const info = groupInfo.get(index);
      if (info && "isHidden" in info) return 0;
      return estimateBlockHeight(blocks[index]);
    },
    overscan: 5,
  });

  const prevBlocksLengthRef = useRef(blocks.length);
  useEffect(() => {
    const grew = blocks.length > prevBlocksLengthRef.current;
    prevBlocksLengthRef.current = blocks.length;
    if (shouldScrollToBottom && grew) {
      virtualizer.scrollToIndex(blocks.length - 1, { align: "end" });
    }
  }, [blocks.length, shouldScrollToBottom, virtualizer]);

  if (blocks.length < VIRTUALIZATION_THRESHOLD) {
    return (
      <div className="divide-y divide-[var(--border-color,rgba(255,255,255,0.06))]">
        {blocks.map((block, idx) => {
          const info = groupInfo.get(idx);
          if (info && "isHidden" in info) return null;
          const groupedAgents = info && "isLeader" in info ? info.agents : undefined;
          return (
            <div key={block.id} className="py-1">
              <TimelineBlockErrorBoundary blockId={block.id}>
                <UnifiedBlock
                  block={block}
                  sessionId={sessionId}
                  workingDirectory={workingDirectory}
                  groupedAgents={groupedAgents}
                />
              </TimelineBlockErrorBoundary>
            </div>
          );
        })}
      </div>
    );
  }

  const virtualItems = virtualizer.getVirtualItems();

  return (
    <div
      style={{
        height: virtualizer.getTotalSize(),
        width: "100%",
        position: "relative",
      }}
    >
      {virtualItems.map((virtualRow) => {
        const block = blocks[virtualRow.index];
        const info = groupInfo.get(virtualRow.index);
        if (info && "isHidden" in info) return null;
        const groupedAgents = info && "isLeader" in info ? info.agents : undefined;
        return (
          <div
            key={block.id}
            data-index={virtualRow.index}
            ref={virtualizer.measureElement}
            style={{
              ...virtualItemBaseStyle,
              transform: `translateY(${virtualRow.start}px)`,
            }}
          >
            <div className="py-1 border-b border-[rgba(255,255,255,0.06)]">
              <TimelineBlockErrorBoundary blockId={block.id}>
                <UnifiedBlock
                  block={block}
                  sessionId={sessionId}
                  workingDirectory={workingDirectory}
                  groupedAgents={groupedAgents}
                />
              </TimelineBlockErrorBoundary>
            </div>
          </div>
        );
      })}
    </div>
  );
});
