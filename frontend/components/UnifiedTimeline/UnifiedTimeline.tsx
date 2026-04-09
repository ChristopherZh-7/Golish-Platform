import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { LiveTerminalBlock } from "@/components/LiveTerminalBlock";
import { WelcomeScreen } from "@/components/WelcomeScreen";
import { useSessionState } from "@/store/selectors/session";
import { VirtualizedTimeline } from "./VirtualizedTimeline";

interface UnifiedTimelineProps {
  sessionId: string;
}

export const UnifiedTimeline = memo(function UnifiedTimeline({ sessionId }: UnifiedTimelineProps) {
  // Use combined selector - replaces 10+ individual useStore calls with one
  const sessionState = useSessionState(sessionId);

  // Destructure for convenience (these are already stable references from the memoized selector)
  const {
    timeline,
    pendingCommand,
    workingDirectory,
  } = sessionState;

  // Terminal-only: just use timeline blocks directly (command blocks)
  const sortedTimeline = timeline;
  const containerRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  // Track if user is scrolled to bottom (for auto-scroll behavior)
  const [isAtBottom, setIsAtBottom] = useState(true);

  // Track programmatic scrolls to prevent content growth from flipping isAtBottom to false
  const programmaticScrollRef = useRef(false);

  // Track scroll position to determine if user is at bottom
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const handleScroll = () => {
      // Skip isAtBottom check during programmatic scrolls to avoid race condition
      // where content growth pushes scroll position away from bottom before we scroll
      if (programmaticScrollRef.current) return;
      const { scrollTop, scrollHeight, clientHeight } = container;
      // Consider "at bottom" if within 50px of the bottom
      setIsAtBottom(scrollHeight - scrollTop - clientHeight < 50);
    };

    container.addEventListener("scroll", handleScroll, { passive: true });
    return () => container.removeEventListener("scroll", handleScroll);
  }, []);

  // Reference for pending scroll animation frame
  const pendingScrollRef = useRef<number | null>(null);

  const scrollDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const scrollToBottom = useCallback(() => {
    if (pendingScrollRef.current !== null) {
      cancelAnimationFrame(pendingScrollRef.current);
    }
    if (scrollDebounceRef.current !== null) {
      clearTimeout(scrollDebounceRef.current);
    }

    // Debounce rapid scroll requests to prevent animation fighting
    scrollDebounceRef.current = setTimeout(() => {
      pendingScrollRef.current = requestAnimationFrame(() => {
        pendingScrollRef.current = requestAnimationFrame(() => {
          if (containerRef.current) {
            programmaticScrollRef.current = true;
            containerRef.current.scrollTo({
              top: containerRef.current.scrollHeight,
              behavior: "smooth",
            });
            setTimeout(() => {
              programmaticScrollRef.current = false;
            }, 400);
          }
          pendingScrollRef.current = null;
        });
      });
      scrollDebounceRef.current = null;
    }, 50);
  }, []);

  // Force-scroll to bottom when command state changes (start or end).
  // When a command finishes, the LiveTerminalBlock unmounts and a static
  // CommandBlock renders. We delay the scroll slightly to let the new block
  // layout with its estimated min-height, preventing a "jump" effect.
  const hasPendingCommand = !!pendingCommand?.command;
  const prevHadPendingRef = useRef(hasPendingCommand);
  const [showLiveBlock, setShowLiveBlock] = useState(hasPendingCommand);
  const [liveBlockFading, setLiveBlockFading] = useState(false);

  useEffect(() => {
    const wasRunning = prevHadPendingRef.current;
    prevHadPendingRef.current = hasPendingCommand;

    if (wasRunning !== hasPendingCommand) {
      if (wasRunning && !hasPendingCommand) {
        // Command just finished — fade out then unmount
        setLiveBlockFading(true);
        setTimeout(() => {
          setShowLiveBlock(false);
          setLiveBlockFading(false);
          setIsAtBottom(true);
          scrollToBottom();
        }, 200);
      } else {
        // Command starting — show immediately
        setShowLiveBlock(true);
        setLiveBlockFading(false);
        setIsAtBottom(true);
        requestAnimationFrame(() => scrollToBottom());
      }
    }
  }, [hasPendingCommand, scrollToBottom]);

  // Auto-scroll only when NEW blocks appear (not on content height changes from expand/collapse)
  const prevTimelineLengthRef = useRef(timeline.length);
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional triggers for auto-scroll
  useEffect(() => {
    const grew = timeline.length > prevTimelineLengthRef.current;
    prevTimelineLengthRef.current = timeline.length;
    if (isAtBottom && (grew || hasPendingCommand)) {
      scrollToBottom();
    }
  }, [
    scrollToBottom,
    isAtBottom,
    timeline.length,
    hasPendingCommand,
  ]);

  // Cleanup pending scroll on unmount
  useEffect(() => {
    return () => {
      if (pendingScrollRef.current !== null) {
        cancelAnimationFrame(pendingScrollRef.current);
      }
      if (scrollDebounceRef.current !== null) {
        clearTimeout(scrollDebounceRef.current);
      }
    };
  }, []);

  // Empty state - only show if no timeline and no command running
  const hasRunningCommand = pendingCommand?.command || pendingCommand?.output;
  const isEmpty = timeline.length === 0 && !hasRunningCommand;

  // Pipeline-sourced commands are shown inside PipelineProgressBlock (expandable rows),
  // so filter them out of the main timeline to avoid duplication.
  const filteredTimeline = useMemo(
    () => sortedTimeline.filter(
      (block) => !(block.type === "command" && block.data.source === "pipeline"),
    ),
    [sortedTimeline],
  );

  return (
    <div className="flex-1 min-h-0 min-w-0 flex flex-col overflow-hidden">
      <div ref={containerRef} className="flex-1 min-h-0 min-w-0 overflow-auto p-2 space-y-2">
        {isEmpty ? (
          <WelcomeScreen />
        ) : (
          <>
            <VirtualizedTimeline
              blocks={filteredTimeline}
              sessionId={sessionId}
              containerRef={containerRef}
              shouldScrollToBottom={isAtBottom}
              workingDirectory={workingDirectory}
            />

            {showLiveBlock && (pendingCommand?.command || pendingCommand?.output) && (
              <div
                className="transition-opacity duration-200"
                style={{ opacity: liveBlockFading ? 0 : 1 }}
              >
                <LiveTerminalBlock
                  sessionId={sessionId}
                  command={pendingCommand?.command || null}
                  interactive
                />
              </div>
            )}
          </>
        )}

        <div ref={bottomRef} />
      </div>
    </div>
  );
});
