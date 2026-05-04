import { Clock } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ActivityItem } from "./types";
import { fmtDur, fmtTime, itemColor, itemIcon } from "./utils";

/** Single line in the activity feed. `indent=true` is used when the item
 *  belongs to a step group, to nest it under the parent step row. */
export function ItemRow({ item, indent = false }: { item: ActivityItem; indent?: boolean }) {
  const active =
    item.kind === "tool_start" || item.kind === "agent_thinking" || item.kind === "sub_agent_start";
  return (
    <div
      className={cn(
        "flex items-start gap-2 py-1 px-3 transition-colors",
        indent && "pl-8",
        active && "bg-blue-500/[0.03]"
      )}
    >
      <div className="mt-0.5 flex-shrink-0">{itemIcon(item.kind)}</div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-1.5">
          <span className={cn("text-[11px] font-medium truncate", itemColor(item.kind))}>
            {item.label}
          </span>
          {item.durationMs != null && (
            <span className="text-[9px] text-muted-foreground/20 flex items-center gap-0.5 flex-shrink-0">
              <Clock className="w-2 h-2" />
              {fmtDur(item.durationMs)}
            </span>
          )}
          <span className="text-[9px] text-muted-foreground/12 ml-auto flex-shrink-0">
            {fmtTime(item.ts)}
          </span>
        </div>
        {item.detail && (
          <p className="mt-0.5 text-[10px] text-muted-foreground/25 font-mono leading-relaxed truncate">
            {item.detail}
          </p>
        )}
      </div>
    </div>
  );
}
