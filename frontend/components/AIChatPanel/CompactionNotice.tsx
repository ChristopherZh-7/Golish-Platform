import { Loader2, Zap } from "lucide-react";

export function CompactionNotice({
  active,
  tokensBefore,
}: {
  active: boolean;
  tokensBefore?: number;
}) {
  return (
    <div className="mx-4 my-2 flex items-start gap-2 rounded-md bg-muted/30 px-3 py-2 text-[11px] text-muted-foreground/70">
      {active ? (
        <>
          <Loader2 className="mt-0.5 h-3 w-3 shrink-0 animate-spin text-accent" />
          <span>
            Compacting context{tokensBefore ? ` (${(tokensBefore / 1000).toFixed(0)}K tokens)` : ""}
            ...
          </span>
        </>
      ) : (
        <>
          <Zap className="mt-0.5 h-3 w-3 shrink-0 text-accent" />
          <details className="group min-w-0">
            <summary className="cursor-pointer list-none font-medium text-foreground/80 marker:content-none">
              Context compacted
              {tokensBefore ? ` from ${(tokensBefore / 1000).toFixed(0)}K tokens` : ""}
              <span className="ml-2 text-[10px] font-normal uppercase tracking-wide text-muted-foreground/60 group-open:hidden">
                Details
              </span>
              <span className="ml-2 hidden text-[10px] font-normal uppercase tracking-wide text-muted-foreground/60 group-open:inline">
                Hide
              </span>
            </summary>
            <div className="mt-2 max-w-sm border-t border-border/50 pt-2 leading-relaxed text-muted-foreground">
              <p>
                Earlier messages were summarized so this conversation can continue within its
                context window.
              </p>
              <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
                <dt>Before compaction</dt>
                <dd className="text-right font-mono text-foreground/70">
                  {tokensBefore ? `${(tokensBefore / 1000).toFixed(0)}K tokens` : "Unavailable"}
                </dd>
                <dt>Conversation state</dt>
                <dd className="text-right text-foreground/70">Continuing from summary</dd>
              </dl>
            </div>
          </details>
        </>
      )}
    </div>
  );
}
