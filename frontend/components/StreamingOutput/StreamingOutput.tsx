import { useEffect, useMemo, useRef } from "react";
import { Ansi } from "@/components/Ansi";
import { stripOscSequences } from "@/lib/ansi";
import { cn } from "@/lib/utils";

interface StreamingOutputProps {
  content: string;
  /** Maximum height in pixels (default: 200) */
  maxHeight?: number;
  className?: string;
  /** Whether to auto-scroll to bottom on new content (default: true) */
  autoScroll?: boolean;
}

/**
 * A fixed-height output component that auto-scrolls as new content arrives.
 * Used for displaying streaming command output in real-time.
 */
export function StreamingOutput({
  content,
  maxHeight = 200,
  className,
  autoScroll = true,
}: StreamingOutputProps) {
  const containerRef = useRef<HTMLPreElement>(null);
  const scrollFrameRef = useRef<number | null>(null);
  const cleanContent = stripOscSequences(content);

  // Auto-scroll to bottom when content changes
  useEffect(() => {
    if (autoScroll && containerRef.current) {
      if (scrollFrameRef.current != null) cancelAnimationFrame(scrollFrameRef.current);
      scrollFrameRef.current = requestAnimationFrame(() => {
        scrollFrameRef.current = null;
        const el = containerRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });
    }
    return () => {
      if (scrollFrameRef.current != null) {
        cancelAnimationFrame(scrollFrameRef.current);
        scrollFrameRef.current = null;
      }
    };
  }, [cleanContent, autoScroll]);

  // Memoize style object to prevent recreation on each render
  const containerStyle = useMemo(() => ({ maxHeight }), [maxHeight]);

  if (!cleanContent.trim()) {
    return <span className="text-[10px] text-muted-foreground italic">No output</span>;
  }

  return (
    <pre
      ref={containerRef}
      style={containerStyle}
      className={cn(
        "ansi-output text-[11px] text-muted-foreground bg-background rounded p-2",
        "whitespace-pre-wrap break-all",
        "overflow-y-auto overflow-x-auto",
        className
      )}
    >
      <Ansi useClasses>{cleanContent}</Ansi>
    </pre>
  );
}
