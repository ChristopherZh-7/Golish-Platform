import { Braces } from "lucide-react";
import { cn } from "@/lib/utils";
import { getEndpointParamNames } from "./endpointParams";

export function EndpointParamChips({
  params,
  limit = 8,
  emptyLabel,
  compact = false,
}: {
  params: unknown;
  limit?: number;
  emptyLabel?: string;
  compact?: boolean;
}) {
  const names = getEndpointParamNames(params);
  if (names.length === 0) {
    if (!emptyLabel) return null;
    return (
      <span className="inline-flex items-center gap-1 rounded bg-muted/20 px-1.5 py-0.5 text-[9px] text-muted-foreground">
        <Braces className="h-2.5 w-2.5" />
        {emptyLabel}
      </span>
    );
  }

  const visible = names.slice(0, limit);
  const overflow = names.length - visible.length;

  return (
    <div className={cn("flex min-w-0 flex-wrap gap-1", compact && "gap-0.5")}>
      {visible.map((name) => (
        <span
          key={name}
          className={cn(
            "inline-flex max-w-[160px] items-center gap-1 truncate rounded border border-cyan-400/15 bg-cyan-400/10 px-1.5 py-0.5 font-mono text-cyan-200",
            compact ? "text-[8px]" : "text-[9px]"
          )}
          title={name}
        >
          <Braces className="h-2.5 w-2.5 flex-shrink-0" />
          <span className="truncate">{name}</span>
        </span>
      ))}
      {overflow > 0 && (
        <span
          className={cn(
            "rounded border border-border/20 bg-muted/20 px-1.5 py-0.5 text-muted-foreground",
            compact ? "text-[8px]" : "text-[9px]"
          )}
        >
          +{overflow}
        </span>
      )}
    </div>
  );
}
