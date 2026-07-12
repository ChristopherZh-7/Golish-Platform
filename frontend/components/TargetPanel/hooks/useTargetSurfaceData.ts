import { useCallback, useEffect, useMemo, useState } from "react";
import { onCustomEvent, onEvent } from "@/lib/events";
import { type DirectoryEntry, listDirectoryEntries } from "@/lib/pentest/api";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import {
  type ApiEndpoint,
  type AuditRow,
  apiEndpointsList,
  type BackendSurfaceHierarchyDto,
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
  targetSurfaceHierarchyGet,
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

export type TargetSurfaceDataSource = keyof TargetSurfaceData;

export interface TargetSurfaceDataSourceError {
  targetId: string;
  source: TargetSurfaceDataSource;
  message: string;
}

interface TargetSurfaceDataLoadResult {
  data: TargetSurfaceData;
  errors: TargetSurfaceDataSourceError[];
}

export type BackendHierarchyStatus = "idle" | "loading" | "success" | "fallback" | "error";

export interface BackendHierarchyLoadResult {
  hierarchy: BackendSurfaceHierarchyDto | null;
  status: BackendHierarchyStatus;
  error: string | null;
}

export interface UseTargetSurfaceDataOptions {
  loadBackendHierarchy?: boolean;
  includeRelatedBackend?: boolean;
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
const EMPTY_BACKEND_RESULT: BackendHierarchyLoadResult = {
  hierarchy: null,
  status: "idle",
  error: null,
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

function settledArray<T>(
  targetId: string,
  source: TargetSurfaceDataSource,
  result: PromiseSettledResult<T[]>,
  errors: TargetSurfaceDataSourceError[]
): T[] {
  if (result.status === "fulfilled") {
    return Array.isArray(result.value) ? result.value : [];
  }
  errors.push({ targetId, source, message: String(result.reason) });
  return [];
}

async function loadSingleTargetSurfaceData(targetId: string): Promise<TargetSurfaceDataLoadResult> {
  const [
    assets,
    endpoints,
    fingerprints,
    jsResults,
    passiveScans,
    timeline,
    directoryEntries,
    logs,
  ] = await Promise.allSettled([
    targetAssetsList(targetId),
    apiEndpointsList(targetId),
    fingerprintsList(targetId),
    jsAnalysisList(targetId),
    passiveScansList(targetId, 50),
    targetTimeline(targetId, 100),
    listDirectoryEntries({ targetId }),
    oplogListByTarget(targetId, 50),
  ]);

  const errors: TargetSurfaceDataSourceError[] = [];

  return {
    data: {
      assets: settledArray(targetId, "assets", assets, errors),
      endpoints: settledArray(targetId, "endpoints", endpoints, errors),
      fingerprints: settledArray(targetId, "fingerprints", fingerprints, errors),
      jsResults: settledArray(targetId, "jsResults", jsResults, errors),
      passiveScans: settledArray(targetId, "passiveScans", passiveScans, errors),
      timeline: settledArray(targetId, "timeline", timeline, errors),
      directoryEntries: settledArray(targetId, "directoryEntries", directoryEntries, errors),
      logs: settledArray(targetId, "logs", logs, errors),
    },
    errors,
  };
}

async function loadTargetSurfaceData(targetIds: string[]): Promise<TargetSurfaceDataLoadResult> {
  if (targetIds.length === 0) return { data: EMPTY_SURFACE_DATA, errors: [] };
  const results = await Promise.all(targetIds.map(loadSingleTargetSurfaceData));
  return {
    data: mergeSurfaceData(results.map((result) => result.data)),
    errors: results.flatMap((result) => result.errors),
  };
}

function formatSourceErrors(errors: TargetSurfaceDataSourceError[]): string | null {
  if (errors.length === 0) return null;
  return errors
    .map(({ targetId, source, message }) => `${source} (${targetId}): ${message}`)
    .join("; ");
}

function backendHierarchyStatusFor(
  hierarchy: BackendSurfaceHierarchyDto | null
): BackendHierarchyStatus {
  if (!hierarchy) return "idle";
  if (hierarchy.mode !== "ip") return "fallback";
  if (hierarchy.dataSource !== "backend_identity") return "fallback";
  if (hierarchy.endpoints.length === 0 && hierarchy.webOrigins.length === 0) return "fallback";
  return "success";
}

async function loadBackendHierarchy(
  targetId: string | null | undefined,
  enabled: boolean,
  includeRelated: boolean
): Promise<BackendHierarchyLoadResult> {
  if (!enabled || !targetId) return EMPTY_BACKEND_RESULT;
  try {
    const hierarchy = await targetSurfaceHierarchyGet(targetId, includeRelated);
    return {
      hierarchy,
      status: backendHierarchyStatusFor(hierarchy),
      error: null,
    };
  } catch (err) {
    return {
      hierarchy: null,
      status: "error",
      error: String(err),
    };
  }
}

export function useTargetSurfaceData(
  targetId: string | null | undefined,
  relatedTargetIds: string[] = NO_RELATED_TARGET_IDS,
  options: UseTargetSurfaceDataOptions = {}
) {
  const targetIds = useMemo(
    () => uniqueIds([targetId, ...relatedTargetIds]),
    [targetId, relatedTargetIds]
  );
  const backendLoadEnabled = Boolean(options.loadBackendHierarchy && targetId);
  const includeRelatedBackend = options.includeRelatedBackend ?? true;
  const [data, setData] = useState<TargetSurfaceData>(EMPTY_SURFACE_DATA);
  const [loading, setLoading] = useState(targetIds.length > 0);
  const [error, setError] = useState<string | null>(null);
  const [sourceErrors, setSourceErrors] = useState<TargetSurfaceDataSourceError[]>([]);
  const [backendHierarchy, setBackendHierarchy] = useState<BackendSurfaceHierarchyDto | null>(null);
  const [backendHierarchyStatus, setBackendHierarchyStatus] =
    useState<BackendHierarchyStatus>("idle");
  const [backendHierarchyError, setBackendHierarchyError] = useState<string | null>(null);

  const applyBackendResult = useCallback((result: BackendHierarchyLoadResult) => {
    setBackendHierarchy(result.hierarchy);
    setBackendHierarchyStatus(result.status);
    setBackendHierarchyError(result.error);
  }, []);

  const reload = useCallback(async () => {
    if (targetIds.length === 0) {
      setData(EMPTY_SURFACE_DATA);
      setLoading(false);
      setError(null);
      setSourceErrors([]);
      applyBackendResult(EMPTY_BACKEND_RESULT);
      return;
    }

    setLoading(true);
    setError(null);
    setSourceErrors([]);
    setBackendHierarchyStatus(backendLoadEnabled ? "loading" : "idle");
    setBackendHierarchyError(null);
    try {
      const [surfaceResult, backendResult] = await Promise.all([
        loadTargetSurfaceData(targetIds),
        loadBackendHierarchy(targetId, backendLoadEnabled, includeRelatedBackend),
      ]);
      setData(surfaceResult.data);
      setSourceErrors(surfaceResult.errors);
      setError(formatSourceErrors(surfaceResult.errors));
      applyBackendResult(backendResult);
    } catch (err) {
      setError(String(err));
      setSourceErrors([]);
      setData(EMPTY_SURFACE_DATA);
      applyBackendResult(EMPTY_BACKEND_RESULT);
    } finally {
      setLoading(false);
    }
  }, [applyBackendResult, backendLoadEnabled, includeRelatedBackend, targetId, targetIds]);

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
      setSourceErrors([]);
      applyBackendResult(EMPTY_BACKEND_RESULT);
      return;
    }

    setLoading(true);
    setError(null);
    setSourceErrors([]);
    setBackendHierarchyStatus(backendLoadEnabled ? "loading" : "idle");
    setBackendHierarchyError(null);
    Promise.all([
      loadTargetSurfaceData(targetIds),
      loadBackendHierarchy(targetId, backendLoadEnabled, includeRelatedBackend),
    ])
      .then(([surfaceResult, backendResult]) => {
        if (cancelled) return;
        setData(surfaceResult.data);
        setSourceErrors(surfaceResult.errors);
        setError(formatSourceErrors(surfaceResult.errors));
        applyBackendResult(backendResult);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
        setSourceErrors([]);
        setData(EMPTY_SURFACE_DATA);
        applyBackendResult(EMPTY_BACKEND_RESULT);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [applyBackendResult, backendLoadEnabled, includeRelatedBackend, targetId, targetIds]);

  return {
    data,
    loading,
    error,
    sourceErrors,
    reload,
    backendHierarchy,
    backendHierarchyStatus,
    backendHierarchyError,
  };
}
