import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { WelcomeScreen } from "@/components/WelcomeScreen";
import { useStore } from "@/store";
import { useSessionState } from "@/store/selectors/session";
import { RunningCommandCard } from "./RunningCommandCard";
import { VirtualizedTimeline } from "./VirtualizedTimeline";

interface UnifiedTimelineProps {
  sessionId: string;
}

export const UnifiedTimeline = memo(function UnifiedTimeline({ sessionId }: UnifiedTimelineProps) {
  // Use combined selector - replaces 10+ individual useStore calls with one
  const sessionState = useSessionState(sessionId);

  // Destructure for convenience (these are already stable references from the memoised selector)
  const { timeline, pendingCommand, workingDirectory } = sessionState;

  // When the Warp-style interactive cell is active, PaneLeaf renders
  // its own self-contained card (`InteractiveCell`) at the bottom that
  // already shows the live output. Suppress this timeline-side
  // `RunningCommandCard` so the user doesn't see the same output
  // duplicated in two places.
  const isInteractiveInputActive = useStore(
    (s) => s.sessions[sessionId]?.interactiveMode?.active === true
  );

  // Terminal-only: just use timeline blocks directly (command blocks)
  const sortedTimeline = timeline;
  const containerRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  // Track if user is scrolled to bottom (for auto-scroll behavior)
  const [isAtBottom, setIsAtBottom] = useState(true);
  // Mirror of `isAtBottom` for use inside synchronous ResizeObserver callbacks
  // (which run outside React's render cycle and therefore can't see the
  // latest state directly).
  const isAtBottomRef = useRef(true);
  isAtBottomRef.current = isAtBottom;

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

  // Defensive fix for "timeline jumps up one row when typing in the input
  // box below" — when a sibling flex item (the input panel) resizes, the
  // browser may clamp our scrollTop, producing a visible jump. If the user
  // was anchored to the bottom, snap them back to the bottom as soon as
  // the resize lands so the jump is invisible.
  //
  // This is a defence-in-depth measure: the primary fix lives in
  // `useUnifiedInputState.ts::adjustTextareaHeight`, which avoids the
  // unnecessary `height='auto'` reflow that was the trigger. This observer
  // additionally covers any future cause of container resize (toolbar
  // expand, model selector growth, etc.).
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    if (typeof ResizeObserver === "undefined") return;

    let lastHeight = container.clientHeight;
    const ro = new ResizeObserver(() => {
      const newHeight = container.clientHeight;
      if (newHeight === lastHeight) return;
      lastHeight = newHeight;
      if (!isAtBottomRef.current) return;
      // Use a synchronous assignment (not scrollTo with smooth) so the
      // correction lands inside the same frame as the resize — otherwise
      // the user perceives a brief flicker before we re-anchor.
      programmaticScrollRef.current = true;
      container.scrollTop = container.scrollHeight;
      // Release the programmatic flag on the next frame; long enough for
      // the scroll event from our own scrollTop write to fire and be
      // skipped, short enough not to swallow a legitimate user scroll.
      requestAnimationFrame(() => {
        programmaticScrollRef.current = false;
      });
    });
    ro.observe(container);
    return () => ro.disconnect();
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

  // Track command lifecycle so we can keep the timeline anchored to
  // the bottom when a command starts / ends. The old `LiveTerminalBlock`
  // overlay was removed alongside Phase A (the running command's output
  // now streams straight into the regular `CommandBlock` and Y/N-style
  // interaction happens in the bottom `UnifiedInput`), so this effect
  // only handles scroll bookkeeping.
  const hasPendingCommand = !!pendingCommand?.command;
  const prevHadPendingRef = useRef(hasPendingCommand);

  useEffect(() => {
    const wasRunning = prevHadPendingRef.current;
    prevHadPendingRef.current = hasPendingCommand;

    if (wasRunning !== hasPendingCommand) {
      setIsAtBottom(true);
      requestAnimationFrame(() => scrollToBottom());
    }
  }, [hasPendingCommand, scrollToBottom]);

  // Auto-scroll only when NEW blocks appear (not on content height changes from expand/collapse)
  const prevTimelineLengthRef = useRef(timeline.length);
  useEffect(() => {
    const grew = timeline.length > prevTimelineLengthRef.current;
    prevTimelineLengthRef.current = timeline.length;
    if (isAtBottom && (grew || hasPendingCommand)) {
      scrollToBottom();
    }
  }, [scrollToBottom, isAtBottom, timeline.length, hasPendingCommand]);

  // Cleanup pending scroll timer on unmount
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
  const hasRunningCommand = !!pendingCommand;
  const isEmpty = timeline.length === 0 && !hasRunningCommand;

  // Terminal view shows only manually typed commands.
  // AI-driven content (tool executions, pipeline progress, sub-agents) is shown
  // in the right AI chat panel instead.
  const filteredTimeline = useMemo(() => {
    const aiCmdSet = new Set<string>();
    for (const block of sortedTimeline) {
      if (block.type === "ai_tool_execution") {
        const cmd = block.data.args?.command;
        if (typeof cmd === "string") aiCmdSet.add(cmd.trim());
      }
    }
    return sortedTimeline.filter((block) => {
      if (block.type !== "command") return false;
      if (block.data.source === "pipeline") return false;
      if (aiCmdSet.has(block.data.command.trim())) return false;
      return true;
    });
  }, [sortedTimeline]);

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

            {hasPendingCommand && pendingCommand && !isInteractiveInputActive && (
              <RunningCommandCard sessionId={sessionId} command={pendingCommand.command ?? null} />
            )}
          </>
        )}

        <div ref={bottomRef} />
      </div>
    </div>
  );
});
