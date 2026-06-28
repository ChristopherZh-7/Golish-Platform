import { ChevronDown, Loader2 } from "lucide-react";
import { type UIEvent, useEffect, useRef, useState, type WheelEvent } from "react";
import { shouldStickToBottomAfterScroll } from "@/lib/scroll-stickiness";
import { cn } from "@/lib/utils";

interface ThinkingBlockProps {
  content: string;
  isActive: boolean;
  /** Epoch ms when the first reasoning chunk arrived. */
  startedAt?: number;
  /** Epoch ms when the last reasoning chunk arrived (set on each delta). */
  endedAt?: number;
  variant?: "message" | "detail";
}

function formatThinkingDuration(ms: number | null): string {
  if (ms == null) return "";
  if (ms < 950) return `${Math.max(1, Math.round(ms / 100) * 100) / 1000}s`;
  const seconds = ms / 1000;
  if (seconds < 60) {
    return `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const restSec = Math.round(seconds - minutes * 60);
  return restSec ? `${minutes}m ${restSec}s` : `${minutes}m`;
}

/**
 * Collapsible thinking pane — defaults to a Cursor-style one-line
 * "Thought for Xs" summary that the user can expand into the full
 * reasoning content.
 *
 * - Streaming: shows "Thinking…" with a spinner.
 * - Settled, collapsed: shows "Thought for 3.4s" (or "0.7s", "1m 12s").
 * - Expanded: reveals the raw reasoning text with a hairline left rail.
 */
export function ThinkingBlock({
  content,
  isActive,
  startedAt,
  endedAt,
  variant = "message",
}: ThinkingBlockProps) {
  // Auto-open while the model is actively thinking so the user can watch the
  // chain-of-thought stream in real time; auto-collapse once thinking ends.
  // Track whether the user has manually overridden this default so we don't
  // fight their toggle on subsequent renders.
  const [expanded, setExpanded] = useState<boolean>(isActive);
  const userToggledRef = useRef(false);
  const prevActiveRef = useRef<boolean>(isActive);
  const scrollRef = useRef<HTMLDivElement>(null);
  const scrollFrameRef = useRef<number | null>(null);
  const shouldStickToBottomRef = useRef(true);
  const previousScrollTopRef = useRef(0);

  useEffect(() => {
    if (isActive) shouldStickToBottomRef.current = true;
    if (userToggledRef.current) return;
    if (prevActiveRef.current !== isActive) {
      setExpanded(isActive);
    }
    prevActiveRef.current = isActive;
  }, [isActive]);

  // Pin the bounded reasoning pane to its latest line while streaming so the
  // newest thought stays in view (Cursor-style) instead of staying frozen at
  // the top once the content overflows the max height.
  useEffect(() => {
    if (isActive && expanded && scrollRef.current && shouldStickToBottomRef.current) {
      if (scrollFrameRef.current != null) cancelAnimationFrame(scrollFrameRef.current);
      scrollFrameRef.current = requestAnimationFrame(() => {
        scrollFrameRef.current = null;
        const el = scrollRef.current;
        if (el && shouldStickToBottomRef.current) {
          el.scrollTop = el.scrollHeight;
          previousScrollTopRef.current = el.scrollTop;
        }
      });
    }
    return () => {
      if (scrollFrameRef.current != null) {
        cancelAnimationFrame(scrollFrameRef.current);
        scrollFrameRef.current = null;
      }
    };
  }, [content, isActive, expanded]);

  const handleToggle = () => {
    userToggledRef.current = true;
    setExpanded((v) => !v);
  };

  const handleReasoningWheelCapture = (event: WheelEvent<HTMLDivElement>) => {
    if (event.deltaY < 0) {
      shouldStickToBottomRef.current = false;
    }
  };

  const handleReasoningScroll = (event: UIEvent<HTMLDivElement>) => {
    const el = event.currentTarget;
    shouldStickToBottomRef.current = shouldStickToBottomAfterScroll(previousScrollTopRef.current, {
      scrollTop: el.scrollTop,
      scrollHeight: el.scrollHeight,
      clientHeight: el.clientHeight,
    });
    previousScrollTopRef.current = el.scrollTop;
  };

  const durationMs = startedAt && endedAt && endedAt >= startedAt ? endedAt - startedAt : null;
  const collapsedLabel = isActive
    ? "Thinking"
    : durationMs != null
      ? `Thought for ${formatThinkingDuration(durationMs)}`
      : "Thought";
  const isDetailVariant = variant === "detail";

  return (
    <div className="m-0">
      <button
        type="button"
        onClick={handleToggle}
        className={cn(
          "flex items-center gap-1.5 text-[11px] leading-5 transition-colors",
          isDetailVariant
            ? "w-full font-normal text-muted-foreground/65 hover:text-muted-foreground/90"
            : "text-foreground/60 hover:text-foreground/80",
          "focus:outline-none"
        )}
        aria-expanded={expanded}
      >
        {isActive ? (
          <Loader2 className="w-3 h-3 animate-spin" />
        ) : (
          <ChevronDown
            className={cn("w-3 h-3 transition-transform", expanded ? "rotate-0" : "-rotate-90")}
          />
        )}
        <span>{collapsedLabel}</span>
      </button>

      {expanded && content && (
        <div
          ref={scrollRef}
          className={cn(
            "ml-1.5 whitespace-pre-wrap overflow-y-auto overscroll-contain",
            isDetailVariant
              ? "mt-1.5 max-h-64 border-l-2 border-foreground/10 pl-3 text-[12.5px] leading-[1.65] text-muted-foreground/80"
              : "mt-1.5 max-h-64 border-l-2 border-foreground/20 pl-3 text-[12px] leading-[1.6] text-foreground/70"
          )}
          onScroll={handleReasoningScroll}
          onWheelCapture={handleReasoningWheelCapture}
        >
          {content}
        </div>
      )}
    </div>
  );
}
