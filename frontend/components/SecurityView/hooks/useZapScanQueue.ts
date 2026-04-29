import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  zapGetAlerts,
  zapPauseScan,
  zapResumeScan,
  zapScanProgress,
  zapStartScan,
  zapStartSpider,
  zapStopScan,
} from "@/lib/pentest/zap-api";
import {
  clearCompletedScanQueueEntries,
  importZapAlerts,
  listScanQueue,
  removeScanQueueEntry,
  saveScanQueueToDb,
  type ScanEndpoint,
} from "@/lib/pentest/scan-queue";

export interface UseZapScanQueueOptions {
  /** Active project path; queue persistence is keyed by it. */
  projectPath: string | null;
  /** A target URL pushed by an external caller (e.g. context menu). Will be added to the queue once. */
  initialUrl?: string | null;
  /** Bulk URLs pushed by an external caller. Added in order, dedup'd against current queue. */
  initialBatchUrls?: string[];
  /** Notify the parent that the consumed URL/batch was processed (so it can be cleared). */
  onUrlConsumed?: () => void;
  /** Optional active-scan policy id passed through to `zapStartScan`. */
  selectedPolicy?: string;
  /** Hook called once before each scan begins (e.g. inject vault credentials into ZAP). */
  beforeScan?: () => Promise<void> | void;
}

export interface UseZapScanQueueReturn {
  endpoints: ScanEndpoint[];
  selectedUrl: string | null;
  setSelectedUrl: (url: string | null) => void;
  selectedEndpoint: ScanEndpoint | undefined;
  scanning: boolean;

  totalAlerts: number;
  completedCount: number;
  scanningCount: number;
  pausedCount: number;
  queuedCount: number;

  addEndpoint: (url: string) => boolean;
  removeEndpoint: (url: string) => void;
  scanSelected: () => Promise<void>;
  scanAll: () => Promise<void>;
  stopAll: () => Promise<void>;
  pauseAll: () => Promise<void>;
  resumeAll: () => Promise<void>;
  clearCompleted: () => void;
  clearAll: () => Promise<void>;
}

/**
 * Encapsulates the ZAP scan-queue state machine: persisted endpoints, polling for
 * progress + alerts, scan/stop/pause/resume orchestration, and findings import on
 * completion.
 *
 * This is the orchestration core that used to live inline in `ScannerPanel.tsx`
 * (~250 lines mixed with rendering). Extracting it keeps the panel focused on
 * presentation and lets the same flow be reused by future scan UIs.
 */
export function useZapScanQueue({
  projectPath,
  initialUrl,
  initialBatchUrls,
  onUrlConsumed,
  selectedPolicy = "",
  beforeScan,
}: UseZapScanQueueOptions): UseZapScanQueueReturn {
  const [endpoints, setEndpoints] = useState<ScanEndpoint[]>([]);
  const [selectedUrl, setSelectedUrl] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Initial load from the persisted queue.
  useEffect(() => {
    listScanQueue(projectPath).then((restored) => {
      if (restored.length > 0) setEndpoints(restored);
    });
  }, [projectPath]);

  // External "add this URL" trigger.
  useEffect(() => {
    if (!initialUrl) return;
    setEndpoints((prev) => {
      if (prev.some((e) => e.url === initialUrl)) return prev;
      const ep: ScanEndpoint = {
        url: initialUrl,
        scanId: null,
        progress: 0,
        status: "queued",
        alerts: [],
        addedAt: Date.now(),
      };
      const next = [...prev, ep];
      saveScanQueueToDb(next, projectPath);
      return next;
    });
    setSelectedUrl(initialUrl);
    onUrlConsumed?.();
  }, [initialUrl, onUrlConsumed, projectPath]);

  // External "add this batch of URLs" trigger.
  useEffect(() => {
    if (!initialBatchUrls?.length) return;
    const now = Date.now();
    setEndpoints((prev) => {
      const existing = new Set(prev.map((e) => e.url));
      const newEps = initialBatchUrls
        .filter((url) => !existing.has(url))
        .map(
          (url, i) =>
            ({
              url,
              scanId: null,
              progress: 0,
              status: "queued" as const,
              alerts: [],
              addedAt: now + i,
            }) satisfies ScanEndpoint
        );
      if (newEps.length === 0) return prev;
      const next = [...prev, ...newEps];
      saveScanQueueToDb(next, projectPath);
      return next;
    });
    onUrlConsumed?.();
    // intentionally only watching initialBatchUrls — same semantics as before
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialBatchUrls]);

  const addEndpoint = useCallback(
    (url: string): boolean => {
      const trimmed = url.trim();
      if (!trimmed) return false;
      if (endpoints.some((e) => e.url === trimmed)) {
        setSelectedUrl(trimmed);
        return false;
      }
      const ep: ScanEndpoint = {
        url: trimmed,
        scanId: null,
        progress: 0,
        status: "queued",
        alerts: [],
        addedAt: Date.now(),
      };
      setEndpoints((prev) => {
        const next = [...prev, ep];
        saveScanQueueToDb(next, projectPath);
        return next;
      });
      setSelectedUrl(trimmed);
      return true;
    },
    [endpoints, projectPath]
  );

  const removeEndpoint = useCallback(
    (url: string) => {
      setEndpoints((prev) => {
        const next = prev.filter((e) => e.url !== url);
        saveScanQueueToDb(next, projectPath);
        return next;
      });
      removeScanQueueEntry(url, projectPath);
      setSelectedUrl((cur) => (cur === url ? null : cur));
    },
    [projectPath]
  );

  const scanSingleEndpoint = useCallback(
    async (url: string) => {
      setEndpoints((prev) =>
        prev.map((e) =>
          e.url === url ? { ...e, status: "spidering", progress: 0, alerts: [] } : e
        )
      );
      if (beforeScan) await beforeScan();
      try {
        await zapStartSpider(url);
        setEndpoints((prev) =>
          prev.map((e) => (e.url === url ? { ...e, status: "scanning" } : e))
        );
        const id = await zapStartScan(url, undefined, undefined, null, selectedPolicy || null);
        setEndpoints((prev) => prev.map((e) => (e.url === url ? { ...e, scanId: id } : e)));
      } catch {
        setEndpoints((prev) => prev.map((e) => (e.url === url ? { ...e, status: "error" } : e)));
      }
    },
    [beforeScan, selectedPolicy]
  );

  const scanAll = useCallback(async () => {
    setScanning(true);
    const queued = endpoints.filter((e) => e.status === "queued");
    for (const ep of queued) {
      await scanSingleEndpoint(ep.url);
    }
    setScanning(false);
  }, [endpoints, scanSingleEndpoint]);

  const scanSelected = useCallback(async () => {
    if (!selectedUrl) return;
    setScanning(true);
    await scanSingleEndpoint(selectedUrl);
    setScanning(false);
  }, [selectedUrl, scanSingleEndpoint]);

  // Poll active scans.
  // Subscribed key is "url:scanId" of every active row so we restart the timer
  // exactly when the active set changes.
  const activeKey = endpoints
    .filter((e) => e.status === "scanning" || e.status === "paused")
    .map((e) => `${e.url}:${e.scanId}`)
    .join(",");
  // biome-ignore lint/correctness/useExhaustiveDependencies: see comment above
  useEffect(() => {
    const active = endpoints.filter(
      (e) => (e.status === "scanning" || e.status === "paused") && e.scanId
    );
    if (active.length === 0) return;
    const poll = async () => {
      for (const ep of active) {
        try {
          const [prog, alerts, msgCount] = await Promise.all([
            zapScanProgress(ep.scanId as string),
            zapGetAlerts(ep.url, 0, 200),
            invoke<number>("zap_scan_message_count", { scanId: ep.scanId as string }).catch(
              () => 0
            ),
          ]);
          const isComplete = prog.progress >= 100;
          if (isComplete && alerts.length > 0) {
            importZapAlerts(alerts, "ZAP Active Scan", projectPath);
          }
          setEndpoints((prev) => {
            const next = prev.map((e) => {
              if (e.url !== ep.url) return e;
              const newStatus = isComplete
                ? ("complete" as const)
                : e.status === "paused"
                  ? ("paused" as const)
                  : ("scanning" as const);
              return {
                ...e,
                progress: prog.progress,
                alerts,
                messageCount: msgCount,
                status: newStatus,
              };
            });
            saveScanQueueToDb(next, projectPath);
            return next;
          });
        } catch {
          /* ignore */
        }
      }
    };
    poll();
    intervalRef.current = setInterval(poll, 2000);
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [activeKey, projectPath]);

  const stopAll = useCallback(async () => {
    for (const ep of endpoints.filter(
      (e) => (e.status === "scanning" || e.status === "paused") && e.scanId
    )) {
      await zapStopScan(ep.scanId as string).catch(() => {});
    }
    setEndpoints((prev) =>
      prev.map((e) =>
        e.status === "scanning" || e.status === "spidering" || e.status === "paused"
          ? { ...e, status: "queued" }
          : e
      )
    );
    setScanning(false);
  }, [endpoints]);

  const pauseAll = useCallback(async () => {
    for (const ep of endpoints.filter((e) => e.status === "scanning" && e.scanId)) {
      await zapPauseScan(ep.scanId as string).catch(() => {});
    }
    setEndpoints((prev) => {
      const next = prev.map((e) =>
        e.status === "scanning" ? { ...e, status: "paused" as const } : e
      );
      saveScanQueueToDb(next, projectPath);
      return next;
    });
  }, [endpoints, projectPath]);

  const resumeAll = useCallback(async () => {
    for (const ep of endpoints.filter((e) => e.status === "paused" && e.scanId)) {
      await zapResumeScan(ep.scanId as string).catch(() => {});
    }
    setEndpoints((prev) => {
      const next = prev.map((e) =>
        e.status === "paused" ? { ...e, status: "scanning" as const } : e
      );
      saveScanQueueToDb(next, projectPath);
      return next;
    });
  }, [endpoints, projectPath]);

  const clearCompleted = useCallback(() => {
    setEndpoints((prev) => {
      const next = prev.filter((e) => e.status !== "complete");
      saveScanQueueToDb(next, projectPath);
      return next;
    });
    clearCompletedScanQueueEntries(projectPath);
  }, [projectPath]);

  const clearAll = useCallback(async () => {
    for (const ep of endpoints.filter(
      (e) => (e.status === "scanning" || e.status === "paused") && e.scanId
    )) {
      await zapStopScan(ep.scanId as string).catch(() => {});
    }
    setEndpoints([]);
    setSelectedUrl(null);
    setScanning(false);
    saveScanQueueToDb([], projectPath);
  }, [endpoints, projectPath]);

  const selectedEndpoint = endpoints.find((e) => e.url === selectedUrl);
  const totalAlerts = endpoints.reduce((acc, e) => acc + e.alerts.length, 0);
  const completedCount = endpoints.filter((e) => e.status === "complete").length;
  const scanningCount = endpoints.filter(
    (e) => e.status === "scanning" || e.status === "spidering" || e.status === "paused"
  ).length;
  const pausedCount = endpoints.filter((e) => e.status === "paused").length;
  const queuedCount = endpoints.filter((e) => e.status === "queued").length;

  return {
    endpoints,
    selectedUrl,
    setSelectedUrl,
    selectedEndpoint,
    scanning,
    totalAlerts,
    completedCount,
    scanningCount,
    pausedCount,
    queuedCount,
    addEndpoint,
    removeEndpoint,
    scanSelected,
    scanAll,
    stopAll,
    pauseAll,
    resumeAll,
    clearCompleted,
    clearAll,
  };
}
