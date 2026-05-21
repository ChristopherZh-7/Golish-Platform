/**
 * Category sidebar + global search box.
 *
 * Categories are derived from the loaded integration list rather than
 * being hardcoded — adding a new integration with a never-seen-before
 * `category` automatically gets a slot in the nav.
 */

import { Search } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ResolvedIntegration } from "@/lib/api/integrations";
import { cn } from "@/lib/utils";

interface CategoryNavProps {
  integrations: ResolvedIntegration[];
  /** Currently-active category id, or `"all"` for "show everything". */
  activeCategory: string;
  onCategoryChange: (next: string) => void;
  search: string;
  onSearchChange: (next: string) => void;
}

export function CategoryNav({
  integrations,
  activeCategory,
  onCategoryChange,
  search,
  onSearchChange,
}: CategoryNavProps) {
  const { t } = useTranslation();
  const categories = collectCategories(integrations);
  const counts = countByCategory(integrations);

  return (
    <div className="space-y-3">
      <div className="relative">
        <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-muted-foreground/60" />
        <input
          type="text"
          value={search}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder={t("integrations.searchPlaceholder")}
          aria-label={t("integrations.searchPlaceholder")}
          className={cn(
            "w-full pl-6 pr-2.5 py-1 text-[11px] rounded-md border bg-background",
            "border-border/40 focus:border-accent outline-none transition-colors"
          )}
        />
      </div>

      <nav className="flex flex-col gap-0.5">
        <CategoryButton
          active={activeCategory === "all"}
          onClick={() => onCategoryChange("all")}
          label={t("integrations.category.all")}
          count={integrations.length}
        />
        {categories.map((c) => (
          <CategoryButton
            key={c}
            active={activeCategory === c}
            onClick={() => onCategoryChange(c)}
            label={t(`integrations.category.${c}`, { defaultValue: c })}
            count={counts.get(c) ?? 0}
          />
        ))}
      </nav>
    </div>
  );
}

interface CategoryButtonProps {
  active: boolean;
  onClick: () => void;
  label: string;
  count: number;
}

function CategoryButton({ active, onClick, label, count }: CategoryButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "w-full px-2.5 py-1 rounded-md text-[11px] text-left transition-colors",
        "flex items-center justify-between",
        active
          ? "bg-accent/10 text-accent"
          : "text-muted-foreground/70 hover:text-foreground hover:bg-[var(--bg-hover)]/40"
      )}
    >
      <span className="truncate">{label}</span>
      <span className="text-[9px] text-muted-foreground/40 tabular-nums">{count}</span>
    </button>
  );
}

function collectCategories(list: ResolvedIntegration[]): string[] {
  const set = new Set<string>();
  for (const item of list) {
    if (item.schema.category) set.add(item.schema.category);
  }
  return Array.from(set).sort();
}

function countByCategory(list: ResolvedIntegration[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const item of list) {
    const k = item.schema.category;
    if (!k) continue;
    out.set(k, (out.get(k) ?? 0) + 1);
  }
  return out;
}

/**
 * Fuzzy-match an integration against a search query. Matches against
 * `tool_id`, `display_name`, `description`, and `category`. Empty
 * query passes everything. Designed to be cheap (≤ 1k integrations).
 */
export function matchSearch(item: ResolvedIntegration, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const haystack = [
    item.tool_id,
    item.schema.display_name,
    item.schema.description ?? "",
    item.schema.category ?? "",
    ...item.schema.groups.map((g) => g.name),
  ]
    .join("\n")
    .toLowerCase();
  // Token-based AND match: every whitespace-separated token must
  // appear somewhere in the haystack.
  return q.split(/\s+/).every((tok) => haystack.includes(tok));
}
