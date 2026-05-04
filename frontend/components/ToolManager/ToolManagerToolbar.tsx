/* ── Tool list toolbar ────────────────────────────────────────────────
 *
 * Search box + category/tier filters + sort + grid/list mode switch.
 * Visible only when the editor is closed (i.e. when the list view is
 * showing). Extracted from `ToolManager.tsx` to keep the main file under
 * the 800-line file-size budget enforced by `scripts/check_file_sizes.sh`.
 */

import { ArrowUpDown, FolderOpen, Grid3X3, List, Search } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import type { SortKey, ViewMode } from "./OutputParserEditor";

export interface ToolManagerToolbarProps {
  search: string;
  setSearch: (v: string) => void;
  selectedCategory: string | null;
  setSelectedCategory: (v: string | null) => void;
  selectedTier: string | null;
  setSelectedTier: (v: string | null) => void;
  sortKey: SortKey;
  setSortKey: (v: SortKey) => void;
  allCategories: string[];
  categoryDisplayName: (catId: string) => string;
  viewMode: ViewMode;
  setViewMode: (v: ViewMode) => void;
}

export function ToolManagerToolbar({
  search,
  setSearch,
  selectedCategory,
  setSelectedCategory,
  selectedTier,
  setSelectedTier,
  sortKey,
  setSortKey,
  allCategories,
  categoryDisplayName,
  viewMode,
  setViewMode,
}: ToolManagerToolbarProps) {
  const { t } = useTranslation();
  return (
    <div className="px-6 py-3 flex items-center gap-3 border-b border-border/10 flex-shrink-0">
      <div className="relative flex-1 max-w-sm">
        <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
        <input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("toolManager.searchPlaceholder")}
          className="w-full h-8 pl-8 pr-3 text-[12px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
        />
      </div>

      <Select
        value={selectedCategory ?? "_all"}
        onValueChange={(v) => setSelectedCategory(v === "_all" ? null : v)}
      >
        <SelectTrigger
          size="sm"
          className="h-7 w-auto min-w-[120px] border-border/15 bg-[var(--bg-hover)]/30 text-[11px] shadow-none px-2.5 gap-1.5"
        >
          <FolderOpen className="w-3 h-3 text-muted-foreground/40" />
          <SelectValue placeholder={t("common.all")} />
        </SelectTrigger>
        <SelectContent position="popper" className="min-w-[140px]">
          <SelectItem value="_all" className="text-[12px]">
            {t("common.all")}
          </SelectItem>
          {allCategories.map((catId) => (
            <SelectItem key={catId} value={catId} className="text-[12px]">
              {categoryDisplayName(catId)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>

      <Select
        value={selectedTier ?? "_all"}
        onValueChange={(v) => setSelectedTier(v === "_all" ? null : v)}
      >
        <SelectTrigger
          size="sm"
          className="h-7 w-auto min-w-[110px] border-border/15 bg-[var(--bg-hover)]/30 text-[11px] shadow-none px-2.5 gap-1.5"
        >
          <SelectValue placeholder={t("common.all")} />
        </SelectTrigger>
        <SelectContent position="popper" className="min-w-[130px]">
          <SelectItem value="_all" className="text-[12px]">
            {t("common.all")}
          </SelectItem>
          <SelectItem value="essential" className="text-[12px] text-red-400">
            {t("toolManager.tierEssential")}
          </SelectItem>
          <SelectItem value="recommended" className="text-[12px] text-amber-400">
            {t("toolManager.tierRecommended")}
          </SelectItem>
          <SelectItem value="optional" className="text-[12px]">
            {t("toolManager.tierOptional")}
          </SelectItem>
        </SelectContent>
      </Select>

      <div className="ml-auto flex items-center gap-1">
        <Select value={sortKey} onValueChange={(v) => setSortKey(v as SortKey)}>
          <SelectTrigger
            size="sm"
            className="h-7 w-auto border-transparent bg-transparent hover:bg-[var(--bg-hover)] text-[11px] shadow-none px-2 gap-1 text-muted-foreground/50"
          >
            <ArrowUpDown className="w-3 h-3" />
            <SelectValue />
          </SelectTrigger>
          <SelectContent position="popper" className="min-w-[100px]">
            <SelectItem value="name" className="text-[12px]">
              {t("toolManager.sortByName")}
            </SelectItem>
            <SelectItem value="status" className="text-[12px]">
              {t("toolManager.sortByStatus")}
            </SelectItem>
            <SelectItem value="category" className="text-[12px]">
              {t("toolManager.sortByCategory")}
            </SelectItem>
            <SelectItem value="runtime" className="text-[12px]">
              {t("toolManager.sortByRuntime")}
            </SelectItem>
          </SelectContent>
        </Select>

        <div className="flex items-center rounded-md border border-border/10 overflow-hidden">
          <button
            type="button"
            onClick={() => setViewMode("grid")}
            title={t("toolManager.gridView")}
            className={cn(
              "p-1.5 transition-colors",
              viewMode === "grid"
                ? "bg-accent/15 text-accent"
                : "text-muted-foreground/50 hover:text-foreground"
            )}
          >
            <Grid3X3 className="w-3.5 h-3.5" />
          </button>
          <button
            type="button"
            onClick={() => setViewMode("list")}
            title={t("toolManager.listView")}
            className={cn(
              "p-1.5 transition-colors",
              viewMode === "list"
                ? "bg-accent/15 text-accent"
                : "text-muted-foreground/50 hover:text-foreground"
            )}
          >
            <List className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
}
