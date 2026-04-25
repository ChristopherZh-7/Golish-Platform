import { ChevronDown, Loader2 } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/utils";

export function ThinkingBlock({ content, isActive }: { content: string; isActive: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const preview = content.length > 80 ? content.slice(0, 80) + "..." : content;

  return (
    <div className="mb-2">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="flex items-center gap-1.5 text-[11px] text-muted-foreground/50 hover:text-muted-foreground/70 transition-colors"
      >
        {isActive ? (
          <Loader2 className="w-3 h-3 animate-spin" />
        ) : (
          <ChevronDown className={cn("w-3 h-3 transition-transform", !expanded && "-rotate-90")} />
        )}
        <span className="italic">{expanded ? "Thinking" : preview}</span>
      </button>
      {expanded && (
        <div className="mt-1.5 pl-4.5 text-[12px] text-muted-foreground/60 leading-[1.6] whitespace-pre-wrap border-l-2 border-muted-foreground/15 ml-1.5 pl-3">
          {content}
        </div>
      )}
    </div>
  );
}
