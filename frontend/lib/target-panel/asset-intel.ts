/**
 * Asset-intel domain helpers for the Target panel.
 *
 * Pure logic extracted from `TargetGroupedView.tsx`:
 *  - the streaming hydrate-activity reducer (`applyStreamEvent`),
 *  - post-run workspace routing (`getNextWorkspaceTabAfterAssetIntelRun`),
 *  - provider status tone.
 */

import type {
  AssetIntelProviderRunStatus,
  AssetIntelProviderRuntimeKind,
  AssetIntelRun,
  AssetIntelStreamEvent,
} from "@/lib/api/asset-intel";
import type { AssetIntelOrgActionKind, WorkspaceTab } from "./types";

export function getNextWorkspaceTabAfterAssetIntelRun(
  action: AssetIntelOrgActionKind,
  _run: AssetIntelRun
): WorkspaceTab | null {
  if (action !== "hydrate_subsidiaries") return null;
  return "activity";
}

/**
 * Per-provider state accumulated from streaming events while a hydrate run is
 * in flight. Indexed by `providerId` inside `HydrateActivity.providers`.
 */
export interface HydrateProviderActivity {
  displayName: string;
  runtime: AssetIntelProviderRuntimeKind | null;
  recentMessages: string[];
  batchCount: number;
  candidateCount: number;
  state: "running" | "completed";
  status?: AssetIntelProviderRunStatus;
}

/**
 * Streaming view of an in-flight hydrate run.
 *
 * Stored in `hydrateActivity[orgId]`. Reset every time the user starts an
 * asset-intel discovery/enrichment run. Cleared once a fresh `AssetIntelRun` is
 * persisted into `hydrateRuns[orgId]`, so the Activity panel always shows
 * either the live stream or the final summary, never both.
 */
export interface HydrateActivity {
  runId: string | null;
  providers: Record<string, HydrateProviderActivity>;
  providerOrder: string[];
}

const HYDRATE_RECENT_MESSAGES_LIMIT = 8;

/**
 * Apply a single streaming event to a hydrate activity snapshot, returning
 * a new snapshot. Pure / immutable so it can be plugged into a `useState`
 * setter callback.
 */
export function applyStreamEvent(
  current: HydrateActivity,
  event: AssetIntelStreamEvent
): HydrateActivity {
  switch (event.kind) {
    case "provider_started": {
      const provider: HydrateProviderActivity = {
        displayName: event.displayName,
        runtime: event.runtime,
        recentMessages: [],
        batchCount: 0,
        candidateCount: 0,
        state: "running",
      };
      return {
        runId: event.runId,
        providers: {
          ...current.providers,
          [event.providerId]: provider,
        },
        providerOrder: current.providerOrder.includes(event.providerId)
          ? current.providerOrder
          : [...current.providerOrder, event.providerId],
      };
    }
    case "provider_progress": {
      const existing = current.providers[event.providerId];
      if (!existing) return current;
      const recent = [...existing.recentMessages, event.message].slice(
        -HYDRATE_RECENT_MESSAGES_LIMIT
      );
      return {
        ...current,
        providers: {
          ...current.providers,
          [event.providerId]: { ...existing, recentMessages: recent },
        },
      };
    }
    case "provider_batch": {
      const existing = current.providers[event.providerId];
      if (!existing) return current;
      const orgs = Array.isArray(event.candidates?.organizations)
        ? event.candidates.organizations.length
        : 0;
      const targets = Array.isArray(event.candidates?.targets)
        ? event.candidates.targets.length
        : 0;
      const added = orgs + targets;
      return {
        ...current,
        providers: {
          ...current.providers,
          [event.providerId]: {
            ...existing,
            batchCount: existing.batchCount + 1,
            candidateCount: existing.candidateCount + added,
          },
        },
      };
    }
    case "provider_completed": {
      const existing = current.providers[event.providerId];
      if (!existing) return current;
      return {
        ...current,
        providers: {
          ...current.providers,
          [event.providerId]: {
            ...existing,
            state: "completed",
            status: event.status,
            candidateCount: event.candidateCount,
          },
        },
      };
    }
    default:
      return current;
  }
}

export function getProviderStatusClass(status: string): string {
  if (status === "completed") return "border-green-500/30 bg-green-500/5 text-green-300";
  if (status === "checked_empty") return "border-blue-500/30 bg-blue-500/5 text-blue-300";
  if (status === "failed" || status === "unavailable") {
    return "border-red-500/30 bg-red-500/5 text-red-300";
  }
  return "border-border/40 bg-muted/10 text-muted-foreground";
}
