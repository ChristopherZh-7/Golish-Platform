import React, { useCallback, useRef } from "react";
import { cn } from "@/lib/utils";

export const WikiEditor = React.forwardRef<
  HTMLTextAreaElement,
  { value: string; onChange: (v: string) => void; language: string | null }
>(function WikiEditor({ value, onChange, language }, ref) {
  const lines = value.split("\n");
  const lineCount = lines.length;
  const internalRef = useRef<HTMLTextAreaElement>(null);
  const lineNumRef = useRef<HTMLDivElement>(null);
  const resolvedRef = (ref as React.MutableRefObject<HTMLTextAreaElement | null>) || internalRef;

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Tab") {
        e.preventDefault();
        const ta = e.currentTarget;
        const start = ta.selectionStart;
        const end = ta.selectionEnd;
        const newValue = value.substring(0, start) + "  " + value.substring(end);
        onChange(newValue);
        requestAnimationFrame(() => {
          ta.selectionStart = ta.selectionEnd = start + 2;
        });
      }
    },
    [value, onChange],
  );

  const handleScroll = useCallback(() => {
    if (resolvedRef.current && lineNumRef.current) {
      lineNumRef.current.scrollTop = resolvedRef.current.scrollTop;
    }
  }, [resolvedRef]);

  return (
    <div className="flex-1 flex min-h-0 overflow-hidden">
      <div
        ref={lineNumRef}
        className="flex-shrink-0 overflow-hidden select-none pt-4 pb-4 text-right pr-2 pl-2 text-[12px] font-mono leading-[1.7] text-muted-foreground/25 border-r border-border/8"
        style={{ width: "48px" }}
        aria-hidden
      >
        {Array.from({ length: lineCount }, (_, i) => (
          <div key={i}>{i + 1}</div>
        ))}
      </div>
      <textarea
        ref={resolvedRef}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onScroll={handleScroll}
        onKeyDown={handleKeyDown}
        spellCheck={false}
        className={cn(
          "flex-1 px-4 py-4 text-[13px] font-mono leading-[1.7] bg-transparent text-foreground outline-none resize-none overflow-y-auto",
          language && "text-emerald-100/90",
        )}
        style={{ tabSize: 2 }}
      />
    </div>
  );
});
