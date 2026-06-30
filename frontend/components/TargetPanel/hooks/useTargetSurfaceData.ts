import { useCallback, useEffect, useMemo, useState } from "react";
import { onCustomEvent, onEvent } from "@/lib/events";
import { type DirectoryEntry, listDirectoryEntries } from "@/lib/pentest/api";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import {
  type ApiEndpoint,
  type AuditRow,
  apiEndpointsList,
  type Fingerprint,
  fingerprintsList,
  type JsAnalysisResult,
  jsAnalysisList,
  oplogListByTarget,
  type PassiveScanLog,
  passiveScansList,
  type TargetAsset,
  type TimelineEntry,
  targetAssetsList,
  targetTimeline,
} from "@/lib/security-analysis";

export interface TargetSurfaceData {
  assets: TargetAsset[];
  endpoints: ApiEndpoint[];
  fingerprints: Fingerprint[];
  jsResults: JsAnalysisResult[];
  passiveScans: PassiveScanLog[];
  timeline: TimelineEntry[];
  directoryEntries: DirectoryEntry[];
  logs: AuditRow[];
}

const EMPTY_SURFACE_DATA: TargetSurfaceData = {
  assets: [],
  endpoints: [],
  fingerprints: [],
  jsResults: [],
  passiveScans: [],
  timeline: [],
  directoryEntries: [],
  logs: [],
};
const NO_RELATED_TARGET_IDS: string[] = [];
const SURFACE_WRITE_TOOLS = new Set([
  "browser_collect_js_api",
  "js_extract_apis",
  "route_probe_paths",
  "pentest_run",
  "output_parse_and_store",
  "discover_apis",
]);

function uniqueIds(ids: Array<string | null | undefined>): string[] {
  return [...new Set(ids.map((id) => id?.trim()).filter(Boolean) as string[])];
}

function mergeById<T extends { id: string | number }>(groups: T[][]): T[] {
  const seen = new Set<string | number>();
  const out: T[] = [];
  for (const group of groups) {
    for (const item of group) {
      if (seen.has(item.id)) continue;
      seen.add(item.id);
      out.push(item);
    }
  }
  return out;
}

function mergeTimelineEntries(groups: TimelineEntry[][]): TimelineEntry[] {
  const seen = new Set<string>();
  const out: TimelineEntry[] = [];
  for (const group of groups) {
    for (const item of group) {
      const key = `${item.source}:${item.event}:${item.createdAt}:${item.details}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(item);
    }
  }
  return out.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

function mergeSurfaceData(results: TargetSurfaceData[]): TargetSurfaceData {
  if (results.length === 0) return EMPTY_SURFACE_DATA;
  return {
    assets: mergeById(results.map((result) => result.assets)),
    endpoints: mergeById(results.map((result) => result.endpoints)),
    fingerprints: mergeById(results.map((result) => result.fingerprints)),
    jsResults: mergeById(results.map((result) => result.jsResults)),
    passiveScans: mergeById(results.map((result) => result.passiveScans)),
    timeline: mergeTimelineEntries(results.map((result) => result.timeline)),
    directoryEntries: mergeById(results.map((result) => result.directoryEntries)),
    logs: mergeById(results.map((result) => result.logs)).sort((a, b) => b.createdAt - a.createdAt),
  };
}

async function loadSingleTargetSurfaceData(targetId: string): Promise<TargetSurfaceData> {
  const [
    assets,
    endpoints,
    fingerprints,
    jsResults,
    passiveScans,
    timeline,
    directoryEntries,
    logs,
  ] = await Promise.all([
    targetAssetsList(targetId),
    apiEndpointsList(targetId),
    fingerprintsList(targetId),
    jsAnalysisList(targetId),
    passiveScansList(targetId, 50),
    targetTimeline(targetId, 100),
    listDirectoryEntries({ targetId }),
    oplogListByTarget(targetId, 50),
  ]);

  return {
    assets: Array.isArray(assets) ? assets : [],
    endpoints: Array.isArray(endpoints) ? endpoints : [],
    fingerprints: Array.isArray(fingerprints) ? fingerprints : [],
    jsResults: Array.isArray(jsResults) ? jsResults : [],
    passiveScans: Array.isArray(passiveScans) ? passiveScans : [],
    timeline: Array.isArray(timeline) ? timeline : [],
    directoryEntries: Array.isArray(directoryEntries) ? directoryEntries : [],
    logs: Array.isArray(logs) ? logs : [],
  };
}

async function loadTargetSurfaceData(targetIds: string[]): Promise<TargetSurfaceData> {
  if (targetIds.length === 0) return EMPTY_SURFACE_DATA;
  return mergeSurfaceData(await Promise.all(targetIds.map(loadSingleTargetSurfaceData)));
}

export function useTargetSurfaceData(
  targetId: string | null | undefined,
  relatedTargetIds: string[] = NO_RELATED_TARGET_IDS
) {
  const targetIds = useMemo(
    () => uniqueIds([targetId, ...relatedTargetIds]),
    [targetId, relatedTargetIds]
  );
  const [data, setData] = useState<TargetSurfaceData>(EMPTY_SURFACE_DATA);
  const [loading, setLoading] = useState(targetIds.length > 0);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    if (targetIds.length === 0) {
      setData(EMPTY_SURFACE_DATA);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);
    try {
      setData(await loadTargetSurfaceData(targetIds));
    } catch (err) {
      setError(String(err));
      setData(EMPTY_SURFACE_DATA);
    } finally {
      setLoading(false);
    }
  }, [targetIds]);

  useEffect(() => {
    if (targetIds.length === 0) return undefined;
    const unlistenAi = onEvent("ai-event", (payload) => {
      const event = payload as { type?: string; tool_name?: string };
      if (
        event.type === "tool_result" &&
        event.tool_name &&
        SURFACE_WRITE_TOOLS.has(event.tool_name)
      ) {
        void reload();
      }
    });
    const unlistenChanged = onCustomEvent("targets-changed", () => {
      void reload();
    });
    return () => {
      runTauriUnlistenFromPromise(unlistenAi);
      runTauriUnlistenFromPromise(unlistenChanged);
    };
  }, [reload, targetIds.length]);

  useEffect(() => {
    let cancelled = false;
    if (targetIds.length === 0) {
      setData(EMPTY_SURFACE_DATA);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);
    loadTargetSurfaceData(targetIds)
      .then((surfaceData) => {
        if (cancelled) return;
        setData(surfaceData);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
        setData(EMPTY_SURFACE_DATA);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [targetIds]);

  return { data, loading, error, reload };
}
