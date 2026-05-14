import { useCallback, useEffect, useMemo, useState } from "react";
import { onCustomEvent } from "@/lib/events";
import { scanTools } from "@/lib/pentest/api";
import { PENTEST_TOOLS_UPDATED_EVENT } from "@/lib/pentest/events";
import type { ToolConfig } from "@/lib/pentest/types";

export function useToolSearch(query: string, enabled: boolean) {
  const [allTools, setAllTools] = useState<ToolConfig[]>([]);
  const [loaded, setLoaded] = useState(false);

  const refresh = useCallback(() => {
    scanTools()
      .then((r) => {
        if (r.success) setAllTools(r.tools);
      })
      .catch(() => {})
      .finally(() => setLoaded(true));
  }, []);

  // Reload whenever `/t` mode is (re)entered so the popup never serves
  // stale data after the user installs / uninstalls a tool in ToolManager.
  // The earlier "load once with a ref lock" optimisation caused exactly
  // that staleness bug — popup never saw newly installed tools until the
  // whole app reloaded.
  useEffect(() => {
    if (!enabled) return;
    refresh();
  }, [enabled, refresh]);

  // Cross-component refresh: ToolManager (install / uninstall / delete /
  // edit) emits `pentest-tools-updated` after every successful mutation,
  // so any open `/t` popup instance reflects the change immediately.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    onCustomEvent(PENTEST_TOOLS_UPDATED_EVENT, () => {
      refresh();
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);

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

  return {
    matches,
    allTools,
    reload: refresh,
  };
}
