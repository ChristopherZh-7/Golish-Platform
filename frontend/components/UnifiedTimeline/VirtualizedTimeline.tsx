import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useRef } from "react";
import { TimelineBlockErrorBoundary } from "@/components/TimelineBlockErrorBoundary";
import { estimateBlockHeight } from "@/lib/timeline/blockHeightEstimation";
import type { UnifiedBlock as UnifiedBlockType } from "@/store";
import { UnifiedBlock } from "./UnifiedBlock";

// Static style constants for virtualized items
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

// Minimum block count before enabling virtualization
// Below this threshold, direct rendering is more efficient
const VIRTUALIZATION_THRESHOLD = 50;

/**
 * Renders timeline blocks using virtualization for improved performance.
 * Only blocks visible in the viewport (plus overscan) are rendered to the DOM.
 */
export const VirtualizedTimeline = memo(function VirtualizedTimeline({
  blocks,
  sessionId,
  containerRef,
  shouldScrollToBottom,
  workingDirectory,
}: VirtualizedTimelineProps) {
  const virtualizer = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => containerRef.current,
    estimateSize: (index) => estimateBlockHeight(blocks[index]),
    overscan: 5, // Render 5 extra items above/below viewport for smooth scrolling
  });

  // Only scroll to bottom when NEW blocks are added (not on height changes from expand/collapse)
  const prevBlocksLengthRef = useRef(blocks.length);
  useEffect(() => {
    const grew = blocks.length > prevBlocksLengthRef.current;
    prevBlocksLengthRef.current = blocks.length;
    if (shouldScrollToBottom && grew) {
      virtualizer.scrollToIndex(blocks.length - 1, { align: "end" });
    }
  }, [blocks.length, shouldScrollToBottom, virtualizer]);

  // For small timelines, skip virtualization overhead
  if (blocks.length < VIRTUALIZATION_THRESHOLD) {
    return (
      <div className="divide-y divide-[var(--border-color,rgba(255,255,255,0.06))]">
        {blocks.map((block) => (
          <div key={block.id} className="py-1">
            <TimelineBlockErrorBoundary blockId={block.id}>
              <UnifiedBlock
                block={block}
                sessionId={sessionId}
                workingDirectory={workingDirectory}
              />
            </TimelineBlockErrorBoundary>
          </div>
        ))}
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
                />
              </TimelineBlockErrorBoundary>
            </div>
          </div>
        );
      })}
    </div>
  );
});
