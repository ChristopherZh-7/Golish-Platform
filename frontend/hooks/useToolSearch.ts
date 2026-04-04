import { useEffect, useMemo, useRef, useState } from "react";
import { scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";

export function useToolSearch(query: string, enabled: boolean) {
  const [allTools, setAllTools] = useState<ToolConfig[]>([]);
  const [loaded, setLoaded] = useState(false);
  const loadedRef = useRef(false);

  useEffect(() => {
    if (loadedRef.current) return;
    loadedRef.current = true;
    scanTools()
      .then((r) => {
        if (r.success) setAllTools(r.tools);
      })
      .catch(() => {});
    setLoaded(true);
  }, []);

  const matches = useMemo(() => {
    if (!enabled || !query.trim() || !loaded) return [];
    const q = query.toLowerCase().trim();
    return allTools
      .filter((t) => {
        const haystack = [
          t.name,
          t.description,
          t.runtime,
          ...(t.tags || []),
          t.category,
          t.subcategory,
        ]
          .filter(Boolean)
          .join(" ")
          .toLowerCase();
        return haystack.includes(q);
      })
      .slice(0, 12);
  }, [allTools, query, enabled, loaded]);

  return { matches, allTools, reload: () => {
    scanTools()
      .then((r) => { if (r.success) setAllTools(r.tools); })
      .catch(() => {});
  }};
}
