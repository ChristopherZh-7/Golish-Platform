import { Loader2, Zap } from "lucide-react";

export function CompactionNotice({ active, tokensBefore }: { active: boolean; tokensBefore?: number }) {
  return (
    <div className="mx-4 my-2 flex items-center gap-2 rounded-md bg-muted/30 px-3 py-2 text-[11px] text-muted-foreground/70">
      {active ? (
        <>
          <Loader2 className="w-3 h-3 animate-spin text-accent" />
          <span>
            Compacting context{tokensBefore ? ` (${(tokensBefore / 1000).toFixed(0)}K tokens)` : ""}
            ...
          </span>
        </>
      ) : (
        <>
          <Zap className="w-3 h-3 text-accent" />
          <span>
            Context compacted
            {tokensBefore ? ` from ${(tokensBefore / 1000).toFixed(0)}K tokens` : ""}
          </span>
        </>
      )}
    </div>
  );
}
