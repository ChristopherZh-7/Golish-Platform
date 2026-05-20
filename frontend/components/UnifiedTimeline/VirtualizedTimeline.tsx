import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useRef } from "react";
import { TimelineBlockErrorBoundary } from "@/components/TimelineBlockErrorBoundary";
import { estimateBlockHeight } from "@/lib/timeline/blockHeightEstimation";
import type { UnifiedBlock as UnifiedBlockType } from "@/store";
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
    estimateSize: (index) => {
      return estimateBlockHeight(blocks[index]);
    },
    overscan: 5,
  });

  const prevBlocksLengthRef = useRef(blocks.length);
  useEffect(() => {
    const grew = blocks.length > prevBlocksLengthRef.current;
    prevBlocksLengthRef.current = blocks.length;
    if (!(shouldScrollToBottom && grew)) return;
    // Defer scrollToIndex by one frame so the virtualizer has already
    // observed the freshly-inserted row's DOM node. Calling it synchronously
    // from the effect can fire before the row has been measured, which makes
    // @tanstack/virtual log `Failed to get offset for index: N` and then
    // silently skip the scroll. The double-RAF (requestAnimationFrame inside
    // requestAnimationFrame) outlasts the row's first measurement pass.
    const targetIndex = blocks.length - 1;
    let inner: number | undefined;
    const outer = requestAnimationFrame(() => {
      inner = requestAnimationFrame(() => {
        try {
          virtualizer.scrollToIndex(targetIndex, { align: "end" });
        } catch {
          // Silently swallow — virtualizer can throw if the target row was
          // removed in the same tick. Next append will re-trigger scrolling.
        }
      });
    });
    return () => {
      cancelAnimationFrame(outer);
      if (inner !== undefined) cancelAnimationFrame(inner);
    };
  }, [blocks.length, shouldScrollToBottom, virtualizer]);

  if (blocks.length < VIRTUALIZATION_THRESHOLD) {
    return (
      <div className="divide-y divide-[var(--border-color,rgba(255,255,255,0.06))]">
        {blocks.map((block) => {
          if (block.type !== "command") return null;
          return (
            <div key={block.id} className="py-1">
              <TimelineBlockErrorBoundary blockId={block.id}>
                <UnifiedBlock
                  block={block}
                  sessionId={sessionId}
                  workingDirectory={workingDirectory}
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
        if (block.type !== "command") return null;
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
