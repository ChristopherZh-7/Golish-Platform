import { ChevronDown, ChevronRight } from "lucide-react";
import { type ReactNode, useState } from "react";
import { Button } from "@/components/ui/button";

export function CollapsibleSection({
  title,
  count,
  children,
  emptyText,
  headerAction,
  headerActionLabel,
  defaultCollapsed = false,
}: {
  title: string;
  count: number;
  children: ReactNode;
  emptyText: string;
  headerAction?: () => void;
  headerActionLabel?: string;
  defaultCollapsed?: boolean;
}) {
  const [collapsed, setCollapsed] = useState(defaultCollapsed);

  return (
    <div className="border-b border-border last:border-b-0">
      <button
        type="button"
        className="w-full flex items-center justify-between px-2 py-1.5 hover:bg-muted/30 cursor-pointer select-none text-left"
        onClick={() => setCollapsed(!collapsed)}
      >
        <div className="flex items-center gap-1.5">
          {collapsed ? (
            <ChevronRight className="w-3.5 h-3.5 text-muted-foreground" />
          ) : (
            <ChevronDown className="w-3.5 h-3.5 text-muted-foreground" />
          )}
          <span className="text-xs font-medium text-foreground">{title}</span>
          <span className="text-[10px] text-muted-foreground bg-muted px-1.5 py-0.5 rounded-full">
            {count}
          </span>
        </div>
        {headerAction && headerActionLabel && count > 0 && (
          <Button
            variant="ghost"
            size="sm"
            className="h-5 text-[10px] px-1.5"
            onClick={(e) => {
              e.stopPropagation();
              headerAction();
            }}
          >
            {headerActionLabel}
          </Button>
        )}
      </button>
      {!collapsed && (
        <div className="pb-1">
          {count === 0 ? (
            <div className="text-[11px] text-muted-foreground px-3 py-2 italic">{emptyText}</div>
          ) : (
            children
          )}
        </div>
      )}
    </div>
  );
}
