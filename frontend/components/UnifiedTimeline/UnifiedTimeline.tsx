import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { WelcomeScreen } from "@/components/WelcomeScreen";
import { useSessionState } from "@/store/selectors/session";
import { RunningCommandCard } from "./RunningCommandCard";
import { VirtualizedTimeline } from "./VirtualizedTimeline";

interface UnifiedTimelineProps {
  sessionId: string;
}

/**
 * UnifiedTimeline (transform-based scroll, 2026-05-16 rewrite)
 *
 * We previously used native `overflow: auto` + `scrollTop`. On macOS
 * Tauri WKWebView this exposed a deeply-buried trackpad-momentum bug
 * that reset `scrollTop` to 0 mid-gesture even with
 * `overscroll-behavior: none` and a preventDefault'd wheel handler.
 * Five rounds of JS-side patches (overscroll-contain → -none →
 * race-detection → manual scrollTop write) all failed because the
 * glitch lives below the wheel-event level inside WebKit.
 *
 * This rewrite ditches native scroll entirely. The viewport is
 * `overflow: hidden` and the inner content is positioned with
 * `transform: translateY(...)`. We compute the translate value from
 * a React state (`scrollPosition`) that the WebView cannot touch.
 * Wheel events are translated 1:1 into scroll-position updates.
 *
 * Trade-offs vs native scroll:
 *  + Glitch-free: WKWebView cannot reset our position because we own
 *    the scroll state in JS.
 *  + Mount-snap-to-bottom is trivial — start with isAtBottom=true and
 *    let the resize observer set scrollPosition=maxScroll.
 *  - No native trackpad inertia: a flick stops the moment fingers
 *    leave the pad. Acceptable for a log surface; users can flick
 *    again to keep going.
 *  - No browser-provided scrollbar UI. We render an opt-in custom
 *    track via `data-show-scrollbar="..."` below (purely cosmetic;
 *    not draggable yet — wheel/drag-to-scroll is the only input).
 *  - Keyboard PageUp / Home / End shortcuts no longer affect this
 *    container by default. If/when needed, wire them to
 *    `setScrollPosition` from a keydown listener.
 */
export const UnifiedTimeline = memo(function UnifiedTimeline({ sessionId }: UnifiedTimelineProps) {
  const sessionState = useSessionState(sessionId);
  const { timeline, pendingCommand, workingDirectory } = sessionState;

  const viewportRef = useRef<HTMLDivElement>(null);
  const innerRef = useRef<HTMLDivElement>(null);

  // scrollPosition: 0 = top of content visible, maxScroll = bottom of
  // content visible. Mirrors the semantics of native scrollTop.
  const [scrollPosition, setScrollPosition] = useState(0);
  // maxScroll: max(0, innerHeight - viewportHeight). 0 means the
  // content fits inside the viewport with no scrolling needed.
  const [maxScroll, setMaxScroll] = useState(0);
  // viewportHeight is tracked so the custom scrollbar thumb can size
  // itself proportionally to the visible window.
  const [viewportHeight, setViewportHeight] = useState(0);

  // Refs mirror state for use inside synchronous wheel handler / RO
  // callbacks (both run outside React's render commit so they can't
  // see the freshest state directly).
  const scrollPositionRef = useRef(0);
  scrollPositionRef.current = scrollPosition;
  const maxScrollRef = useRef(0);
  maxScrollRef.current = maxScroll;

  // Derived: clamp scroll position into the valid range. maxScroll
  // may lag scrollPosition by one render after a content shrink, so
  // we always render off the clamped value.
  const clampedScrollPosition = Math.max(0, Math.min(maxScroll, scrollPosition));
  // "At bottom" = within 50 px of the latest content, or the content
  // is so short there is nothing to scroll.
  const isAtBottom = maxScroll === 0 || clampedScrollPosition >= maxScroll - 50;
  const isAtBottomRef = useRef(true);
  isAtBottomRef.current = isAtBottom;

  // -- Keep maxScroll in sync with viewport / content sizes --------
  //
  // ResizeObserver fires whenever the viewport (window resize, sibling
  // input panel growth) or the inner content (new command block,
  // CommandBlock collapse / expand) changes size. On every resize:
  //   1. Recompute maxScroll.
  //   2. If the user is currently at the bottom, stay at the bottom
  //      as content grows — the warp-style log behaviour.
  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const inner = innerRef.current;
    if (!viewport || !inner) return;

    const recompute = () => {
      const innerH = inner.scrollHeight;
      const vpH = viewport.clientHeight;
      const nextMax = Math.max(0, innerH - vpH);
      setViewportHeight(vpH);
      setMaxScroll(nextMax);
      if (isAtBottomRef.current) {
        setScrollPosition(nextMax);
      }
    };

    const id = requestAnimationFrame(recompute);

    let ro: ResizeObserver | undefined;
    if (typeof ResizeObserver !== "undefined") {
      ro = new ResizeObserver(recompute);
      ro.observe(viewport);
      ro.observe(inner);
    }

    return () => {
      cancelAnimationFrame(id);
      ro?.disconnect();
    };
  }, [sessionId]);

  // -- Mount snap-to-bottom on session change ----------------------
  //
  // Whenever we mount this UnifiedTimeline for a new sessionId, reset
  // scrollPosition. The ResizeObserver above will then push us to the
  // bottom once it has measured the inner content.
  useLayoutEffect(() => {
    setScrollPosition(0);
    isAtBottomRef.current = true;
  }, [sessionId]);

  const hasPendingCommand = !!pendingCommand?.command;
  const prevHadPendingRef = useRef(hasPendingCommand);

  // Debounce the false→true edge by ~180 ms so a fast command (`ls`,
  // `pwd`, etc.) that completes inside that window never flashes a
  // `RunningCommandCard` placeholder — by the time the timer fires
  // the command has already ended, `hasPendingCommand` flipped back
  // to false, and `setTimeout` is cleared in the effect cleanup. The
  // true→false edge fires immediately so the card disappears the
  // instant the command actually ends.
  const [showRunningCard, setShowRunningCard] = useState(false);
  useEffect(() => {
    if (hasPendingCommand) {
      const t = setTimeout(() => setShowRunningCard(true), 180);
      return () => clearTimeout(t);
    }
    setShowRunningCard(false);
  }, [hasPendingCommand]);

  // -- Command lifecycle snap-to-bottom ---------------------------
  //
  // When a command starts or ends, force isAtBottom + jump to bottom
  // so the user sees the freshly-started RunningCommandCard / the
  // newly-appended CommandBlock.
  useEffect(() => {
    const wasRunning = prevHadPendingRef.current;
    prevHadPendingRef.current = hasPendingCommand;
    if (wasRunning !== hasPendingCommand) {
      setScrollPosition(maxScrollRef.current);
      isAtBottomRef.current = true;
    }
  }, [hasPendingCommand]);

  // -- New blocks → stay at bottom if we were ----------------------
  //
  // When timeline.length grows AND the user was at the bottom, keep
  // them there. Initial value `0` so a restored project (mounts with
  // N blocks present) detects the 0 → N jump and snaps too.
  const prevTimelineLengthRef = useRef(0);
  useEffect(() => {
    const grew = timeline.length > prevTimelineLengthRef.current;
    prevTimelineLengthRef.current = timeline.length;
    if (grew && isAtBottomRef.current) {
      setScrollPosition(maxScrollRef.current);
    }
  }, [timeline.length]);

  // -- Wheel: the entire scrolling input path ----------------------
  //
  // Viewport is overflow:hidden so the browser can't scroll on its
  // own. We translate wheel.deltaY directly into a scrollPosition
  // delta and let React re-render the transform.
  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;

    const handleWheel = (e: WheelEvent) => {
      // Vertical only. Horizontal (shift+wheel, sideways trackpad) is
      // rare here and leaving it to the browser keeps native
      // text-selection scrolling intact if the user ever drags out
      // of a CommandBlock.
      if (e.deltaY === 0) return;
      e.preventDefault();
      const max = maxScrollRef.current;
      if (max === 0) return;
      const current = scrollPositionRef.current;
      // deltaY > 0 = user wants newer content = scrollPosition grows
      // deltaY < 0 = user wants older content = scrollPosition shrinks
      const next = Math.max(0, Math.min(max, current + e.deltaY));
      if (next !== current) {
        setScrollPosition(next);
      }
    };

    viewport.addEventListener("wheel", handleWheel, { passive: false });
    return () => viewport.removeEventListener("wheel", handleWheel);
  }, []);

  // Empty state — only show if no timeline and no command running.
  const hasRunningCommand = !!pendingCommand;
  const isEmpty = timeline.length === 0 && !hasRunningCommand;

  // Terminal view shows only manually typed commands. AI-driven content
  // (tool executions, sub-agents) is shown in the
  // right AI chat panel instead.
  const sortedTimeline = timeline;
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
      if (aiCmdSet.has(block.data.command.trim())) return false;
      return true;
    });
  }, [sortedTimeline]);

  // Translate value: inner is bottom-anchored by absolute positioning
  // (`absolute inset-x-0 bottom-0`, see JSX below) so the identity
  // transform (translateY(0)) shows the latest content. Subtracting
  // clampedScrollPosition from maxScroll yields the amount we need
  // to slide the inner DOWN to expose earlier content:
  //   scrollPosition=0        → translateY(maxScroll px) shows the top
  //   scrollPosition=maxScroll → translateY(0) shows the bottom
  const transformY = maxScroll - clampedScrollPosition;
  // Inner-style memoised so we don't allocate a new style object on
  // every render — keeps React reconciliation fast and avoids forcing
  // unnecessary CommandBlock subtree re-renders.
  const innerStyle = useMemo(
    () => ({
      transform: `translateY(${transformY}px)`,
      willChange: "transform" as const,
    }),
    [transformY]
  );

  // Click anywhere on the viewport background should NOT scroll —
  // wheel is the only input path. We deliberately don't handle
  // pointermove drag-to-scroll either; mobile is out of scope for
  // this desktop Tauri build.
  const noop = useCallback(() => {}, []);

  // -- Custom scrollbar (cosmetic + draggable) ---------------------
  //
  // We render our own thin track + thumb because the underlying
  // viewport is `overflow: hidden` so the browser draws no native
  // scrollbar. The track is inset from the viewport edges by
  // TRACK_INSET on top and bottom (see the `top-1 right-1 bottom-1`
  // utility on the track <div> below), so the track's *usable*
  // height is `viewportHeight - 2 * TRACK_INSET`. Earlier revisions
  // mistakenly sized the thumb against `viewportHeight` itself; that
  // let the thumb extend past the track's bottom edge by TRACK_INSET
  // pixels when scrolled to the end. Because the surrounding
  // timeline-pane container is `overflow: hidden`, the lower end-cap
  // of the rounded thumb was clipped — the user saw a flat bottom
  // instead of a capsule. Sizing against `trackHeight` fixes it.
  //
  // Sizing rules (industry-standard):
  //   thumbHeight = max(MIN_THUMB, trackHeight * (viewport / inner))
  //              = max(MIN_THUMB, trackHeight * (viewport / (viewport + maxScroll)))
  //   thumbTop    = (scrollPosition / maxScroll) * (trackHeight - thumbHeight)
  //
  // The thumb is draggable: mousedown captures the pointer, mousemove
  // converts vertical pixel delta back to scroll-position delta. We
  // skip touch / pointer events since this is a desktop-only Tauri
  // build.
  const TRACK_INSET_PX = 4; // matches `top-1` and `bottom-1` on the track <div>
  const trackHeight = Math.max(0, viewportHeight - TRACK_INSET_PX * 2);
  const showScrollbar = maxScroll > 0 && trackHeight > 0;
  const MIN_THUMB_PX = 24;
  const thumbHeight = showScrollbar
    ? Math.min(
        trackHeight,
        Math.max(MIN_THUMB_PX, trackHeight * (viewportHeight / (viewportHeight + maxScroll)))
      )
    : 0;
  const trackUsable = Math.max(0, trackHeight - thumbHeight);
  const thumbTop = showScrollbar ? (clampedScrollPosition / maxScroll) * trackUsable : 0;

  // Drag state — local to the handler so React doesn't re-render
  // every pointer move. We still need the closure to see live
  // maxScroll/viewportHeight, hence reading from refs.
  const onThumbPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      const startY = e.clientY;
      const startPos = scrollPositionRef.current;
      const max = maxScrollRef.current;
      const vp = viewportHeight;
      const usable = Math.max(0, vp - thumbHeight);
      if (max === 0 || usable === 0) return;

      const target = e.currentTarget;
      target.setPointerCapture(e.pointerId);

      const handleMove = (ev: PointerEvent) => {
        const dy = ev.clientY - startY;
        const delta = (dy / usable) * max;
        const next = Math.max(0, Math.min(max, startPos + delta));
        setScrollPosition(next);
      };
      const handleUp = (ev: PointerEvent) => {
        target.releasePointerCapture(ev.pointerId);
        target.removeEventListener("pointermove", handleMove);
        target.removeEventListener("pointerup", handleUp);
        target.removeEventListener("pointercancel", handleUp);
      };

      target.addEventListener("pointermove", handleMove);
      target.addEventListener("pointerup", handleUp);
      target.addEventListener("pointercancel", handleUp);
    },
    [viewportHeight, thumbHeight]
  );

  return (
    <div className="flex-1 min-h-0 min-w-0 flex flex-col overflow-hidden">
      <div
        className="relative flex-1 min-h-0 min-w-0 flex flex-col overflow-hidden"
        data-timeline-pane
      >
        <div
          ref={viewportRef}
          className="relative flex-1 min-h-0 min-w-0 overflow-hidden"
          data-testid="timeline-viewport"
          data-scroll-position={clampedScrollPosition}
          data-max-scroll={maxScroll}
          data-viewport-h={viewportHeight}
          data-inner-h={maxScroll + viewportHeight}
          data-transform-y={maxScroll - clampedScrollPosition}
          data-is-at-bottom={isAtBottom ? "1" : "0"}
        >
          {isEmpty ? (
            // Empty state: centre the welcome screen in the viewport.
            // No transform applied — WelcomeScreen is its own layout.
            <div className="absolute inset-0 flex items-center justify-center p-2">
              <WelcomeScreen />
            </div>
          ) : (
            // Warp-style bottom-anchored timeline. We anchor the inner
            // div to the viewport's bottom edge with absolute
            // positioning — NOT `flex justify-end`. The flex approach
            // breaks the moment inner exceeds the viewport: per the
            // spec, `justify-content` is silently ignored when flex
            // items overflow, leaving inner top-aligned with its
            // bottom children clipped by `overflow: hidden`. Absolute
            // `bottom: 0` keeps inner bottom glued to viewport bottom
            // regardless of size, so `transformY = 0` always reveals
            // the latest content and `transformY = maxScroll` reveals
            // the top exactly as the formula intends.
            <div
              ref={innerRef}
              className="absolute inset-x-0 bottom-0 flex flex-col p-2 space-y-2"
              style={innerStyle}
              onClick={noop}
            >
              <VirtualizedTimeline
                blocks={filteredTimeline}
                sessionId={sessionId}
                containerRef={viewportRef}
                shouldScrollToBottom={isAtBottom}
                workingDirectory={workingDirectory}
              />

              {showRunningCard && pendingCommand && (
                // Always render while there's a pending command (debounced
                // ~180ms so sub-180ms commands like `ls` don't flash).
                // In interactive mode (`stdin_wait` fired) this card is
                // ALSO the stdin sink — its zero-size capture textarea
                // grabs focus and the user's keystrokes flow straight to
                // the PTY, echoing inline at the cursor position next to
                // the program's `[Y/n]` prompt. PaneLeaf hides the bottom
                // `UnifiedInput` while interactive, so there's no double
                // input UI.
                <RunningCommandCard
                  sessionId={sessionId}
                  command={pendingCommand.command ?? null}
                />
              )}
            </div>
          )}
        </div>

        {showScrollbar && (
          // Custom scrollbar track + thumb. Absolute positioning over
          // the viewport's right edge; pointer-events:none on the
          // track so background clicks fall through to the timeline,
          // pointer-events:auto on the thumb so it stays draggable.
          <div
            className="absolute top-1 right-1 bottom-1 w-1.5 pointer-events-none"
            data-testid="timeline-scrollbar-track"
          >
            <div
              role="scrollbar"
              aria-controls="timeline-viewport"
              aria-orientation="vertical"
              aria-valuemin={0}
              aria-valuemax={maxScroll}
              aria-valuenow={clampedScrollPosition}
              data-testid="timeline-scrollbar-thumb"
              className="absolute right-0 w-full rounded-full cursor-grab active:cursor-grabbing pointer-events-auto"
              style={{
                top: `${thumbTop}px`,
                height: `${thumbHeight}px`,
              }}
              onPointerDown={onThumbPointerDown}
            />
          </div>
        )}
      </div>
    </div>
  );
});
