import { memo } from "react";
import { cn } from "@/lib/utils";

interface ContextUsageRingProps {
  contextUsage: { utilization: number; totalTokens: number; maxTokens: number } | null;
}

export const ContextUsageRing = memo(function ContextUsageRing({
  contextUsage,
}: ContextUsageRingProps) {
  const title = contextUsage
    ? `${(contextUsage.utilization * 100).toFixed(1)}% · ${(contextUsage.totalTokens / 1000).toFixed(1)}K / ${(contextUsage.maxTokens / 1000).toFixed(0)}K context used`
    : "No context data";

  return (
    <div className="relative group" title={title}>
      <svg className="w-5 h-5 -rotate-90" viewBox="0 0 20 20">
        <circle
          cx="10"
          cy="10"
          r="8"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          className="text-muted-foreground/20"
        />
        <circle
          cx="10"
          cy="10"
          r="8"
          fill="none"
          strokeWidth="2"
          strokeLinecap="round"
          strokeDasharray={`${(contextUsage?.utilization ?? 0) * 50.27} 50.27`}
          className={cn(
            "transition-all duration-300",
            !contextUsage
              ? "text-muted-foreground/30"
              : contextUsage.utilization > 0.9
                ? "text-red-400"
                : contextUsage.utilization > 0.7
                  ? "text-[#e0af68]"
                  : "text-accent",
          )}
          stroke="currentColor"
        />
      </svg>
      <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 rounded bg-popover border border-border/30 text-[10px] text-popover-foreground whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-50">
        {contextUsage
          ? `${(contextUsage.utilization * 100).toFixed(1)}% · ${(contextUsage.totalTokens / 1000).toFixed(1)}K / ${(contextUsage.maxTokens / 1000).toFixed(0)}K context used`
          : "Context usage unavailable"}
      </div>
    </div>
  );
});
