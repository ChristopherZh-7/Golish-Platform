import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type { WikiStats } from "@/lib/wiki";
import { CATEGORY_META, STATUS_COLORS } from "./utils";

export function WikiDashboard({
  stats,
  onOpenPage,
}: {
  stats: WikiStats | null;
  onOpenPage: (path: string) => void;
}) {
  if (!stats) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/20" />
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto px-8 py-6">
      <div className="grid grid-cols-4 gap-3 mb-6">
        {[
          { label: "Pages", value: stats.total_pages, color: "text-accent" },
          { label: "Words", value: stats.total_words.toLocaleString(), color: "text-blue-400" },
          {
            label: "Categories",
            value: Object.keys(stats.by_category).length,
            color: "text-emerald-400",
          },
          {
            label: "Orphans",
            value: stats.orphan_count,
            color: stats.orphan_count > 0 ? "text-yellow-400" : "text-green-400",
          },
        ].map((s) => (
          <div key={s.label} className="rounded-lg border border-border/10 bg-muted/5 px-4 py-3">
            <div className={cn("text-[20px] font-semibold", s.color)}>{s.value}</div>
            <div className="text-[10px] text-muted-foreground/40 mt-0.5">{s.label}</div>
          </div>
        ))}
      </div>

      <div className="grid grid-cols-2 gap-6">
        <div>
          <h3 className="text-[12px] font-medium text-foreground/70 mb-3">By Category</h3>
          <div className="space-y-2">
            {Object.entries(stats.by_category)
              .sort(([, a], [, b]) => b - a)
              .map(([cat, count]) => {
                const meta = CATEGORY_META[cat] || CATEGORY_META.uncategorized;
                const pct = stats.total_pages > 0 ? (count / stats.total_pages) * 100 : 0;
                return (
                  <div key={cat} className="flex items-center gap-2">
                    <span className="text-[12px] w-5 text-center">{meta.icon}</span>
                    <span className={cn("text-[11px] w-24", meta.color)}>{meta.label}</span>
                    <div className="flex-1 h-1.5 bg-muted/10 rounded-full overflow-hidden">
                      <div
                        className={cn("h-full rounded-full bg-accent/40")}
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                    <span className="text-[10px] text-muted-foreground/40 w-8 text-right">
                      {count}
                    </span>
                  </div>
                );
              })}
          </div>
        </div>

        <div>
          <h3 className="text-[12px] font-medium text-foreground/70 mb-3">By Status</h3>
          <div className="flex flex-wrap gap-2">
            {Object.entries(stats.by_status)
              .sort(([, a], [, b]) => b - a)
              .map(([status, count]) => (
                <div
                  key={status}
                  className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg border border-border/10 bg-muted/5"
                >
                  <span
                    className={cn(
                      "w-2 h-2 rounded-full",
                      STATUS_COLORS[status] || "bg-muted-foreground"
                    )}
                  />
                  <span className="text-[10px] text-foreground/60 capitalize">{status}</span>
                  <span className="text-[10px] text-muted-foreground/30 ml-1">{count}</span>
                </div>
              ))}
          </div>
        </div>
      </div>

      {stats.recent_changes.length > 0 && (
        <div className="mt-6">
          <h3 className="text-[12px] font-medium text-foreground/70 mb-3">Recent Changes</h3>
          <div className="space-y-1">
            {stats.recent_changes.map((ch) => {
              const meta = CATEGORY_META[ch.category] || CATEGORY_META.uncategorized;
              const actionColors: Record<string, string> = {
                create: "text-green-400 bg-green-500/10",
                update: "text-blue-400 bg-blue-500/10",
                delete: "text-red-400 bg-red-500/10",
                link: "text-accent bg-accent/10",
                unlink: "text-orange-400 bg-orange-500/10",
              };
              return (
                <button
                  type="button"
                  key={ch.id}
                  onClick={() => onOpenPage(ch.page_path)}
                  className="flex items-center gap-2 w-full px-3 py-2 rounded-lg hover:bg-muted/10 transition-colors text-left"
                >
                  <span
                    className={cn(
                      "text-[7px] px-1.5 py-0.5 rounded uppercase font-medium",
                      actionColors[ch.action] || "text-muted-foreground/40 bg-muted/10"
                    )}
                  >
                    {ch.action}
                  </span>
                  <span className="text-[10px]">{meta.icon}</span>
                  <span className="text-[11px] text-foreground/70 truncate flex-1">
                    {ch.title || ch.page_path}
                  </span>
                  <span className="text-[9px] text-muted-foreground/25 flex-shrink-0">
                    {new Date(ch.created_at).toLocaleDateString()}{" "}
                    {new Date(ch.created_at).toLocaleTimeString([], {
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </span>
                  <span className="text-[8px] text-muted-foreground/20 flex-shrink-0">
                    {ch.actor}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
