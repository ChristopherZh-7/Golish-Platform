import { useCallback, useEffect, useState } from "react";
import { type DirectoryEntry, listDirectoryEntries } from "@/lib/pentest/api";
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

async function loadTargetSurfaceData(targetId: string): Promise<TargetSurfaceData> {
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

export function useTargetSurfaceData(targetId: string | null | undefined) {
  const [data, setData] = useState<TargetSurfaceData>(EMPTY_SURFACE_DATA);
  const [loading, setLoading] = useState(Boolean(targetId));
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    if (!targetId) {
      setData(EMPTY_SURFACE_DATA);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);
    try {
      setData(await loadTargetSurfaceData(targetId));
    } catch (err) {
      setError(String(err));
      setData(EMPTY_SURFACE_DATA);
    } finally {
      setLoading(false);
    }
  }, [targetId]);

  useEffect(() => {
    let cancelled = false;
    if (!targetId) {
      setData(EMPTY_SURFACE_DATA);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);
    loadTargetSurfaceData(targetId)
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
  }, [targetId]);

  return { data, loading, error, reload };
}
