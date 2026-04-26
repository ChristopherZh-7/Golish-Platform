import { useCallback, useEffect, useState } from "react";
import { scanTools, getCategories, checkToolsExecutablePermissions, checkToolExecutablePermission } from "@/lib/pentest/api";
import type { ToolConfig, ToolCategory } from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";
import type { ToolWithMeta, SortKey } from "../OutputParserEditor";

const VIA_NO_CHMOD: Array<NonNullable<ToolConfig["installedVia"]>> = ["homebrew", "gem", "system_path"];

export function useToolData() {
  const { t } = useTranslation();
  const [tools, setTools] = useState<ToolWithMeta[]>([]);
  const [categories, setCategories] = useState<ToolCategory[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [selectedTier, setSelectedTier] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>("name");

  const loadData = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    setError(null);
    try {
      const [scanResult, cats] = await Promise.all([
        scanTools().catch(() => ({ success: false, tools: [] as ToolConfig[] })),
        getCategories().catch(() => [] as ToolCategory[]),
      ]);
      const safeCats = Array.isArray(cats) ? cats : [];
      setCategories(safeCats);
      const catMap = new Map<string, string>();
      const subMap = new Map<string, string>();
      for (const c of safeCats) {
        catMap.set(c.id, c.name);
        for (const s of c.items) subMap.set(`${c.id}/${s.id}`, s.name);
      }
      const seen = new Set<string>();
      const enriched: ToolWithMeta[] = [];
      for (const tool of (scanResult.tools || [])) {
        if (seen.has(tool.id)) continue;
        seen.add(tool.id);
        enriched.push({
          ...tool,
          categoryName: catMap.get(tool.category) || tool.category,
          subcategoryName: subMap.get(`${tool.category}/${tool.subcategory}`) || tool.subcategory,
        });
      }

      const needPerm: { i: number; tool: ToolWithMeta }[] = [];
      enriched.forEach((tool, i) => {
        if (!tool.installed) return;
        if (tool.installedVia && VIA_NO_CHMOD.includes(tool.installedVia)) return;
        needPerm.push({ i, tool });
      });

      let permResults: { ok: boolean; reason?: string }[] = [];
      if (needPerm.length > 0) {
        try {
          permResults = await checkToolsExecutablePermissions(
            needPerm.map(({ tool }) => ({
              executable: tool.executable, runtime: tool.runtime, installMethod: tool.install?.method,
            })),
          );
        } catch {
          permResults = await Promise.all(
            needPerm.map(async ({ tool }) => {
              try {
                return await checkToolExecutablePermission({
                  executable: tool.executable, runtime: tool.runtime, installMethod: tool.install?.method,
                });
              } catch { return { ok: true as const, reason: undefined }; }
            }),
          );
        }
      }

      const permByIndex = new Map(needPerm.map((e, j) => [e.i, permResults[j]!]));
      const withPermissionState: ToolWithMeta[] = enriched.map((tool, index) => {
        if (!tool.installed) return { ...tool, executableReady: true, executableError: undefined };
        if (tool.installedVia && VIA_NO_CHMOD.includes(tool.installedVia)) return { ...tool, executableReady: true, executableError: undefined };
        const p = permByIndex.get(index);
        if (p) return { ...tool, executableReady: p.ok, executableError: p.reason };
        return { ...tool, executableReady: true, executableError: undefined };
      });
      setTools(withPermissionState);
    } catch (e) {
      setError(t("toolManager.loadFailed", { error: e }));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => { loadData(); }, [loadData]);

  const installedCount = tools.filter((tool) => tool.installed).length;
  const allCategories = Array.from(new Set(tools.map((tool) => tool.category)));
  const filteredTools = tools
    .filter((tool) => {
      if (selectedCategory && tool.category !== selectedCategory) return false;
      if (selectedTier && (tool.tier || "optional") !== selectedTier) return false;
      if (!search.trim()) return true;
      const q = search.trim().toLowerCase();
      return tool.name.toLowerCase().includes(q) || tool.description.toLowerCase().includes(q) || tool.id.toLowerCase().includes(q);
    })
    .sort((a, b) => {
      switch (sortKey) {
        case "status": return (b.installed ? 1 : 0) - (a.installed ? 1 : 0);
        case "category": return (a.categoryName || "").localeCompare(b.categoryName || "");
        case "runtime": return a.runtime.localeCompare(b.runtime);
        default: return a.name.localeCompare(b.name);
      }
    });

  const categoryDisplayName = (catId: string) => (categories ?? []).find((c) => c.id === catId)?.name || catId;

  return {
    tools, categories, loading, error, setError,
    search, setSearch, selectedCategory, setSelectedCategory,
    selectedTier, setSelectedTier, sortKey, setSortKey,
    loadData, installedCount, allCategories, filteredTools, categoryDisplayName,
  };
}
