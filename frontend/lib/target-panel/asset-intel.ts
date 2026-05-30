/**
 * Asset-intel domain helpers for the Target panel.
 *
 * Pure logic extracted from `TargetGroupedView.tsx`:
 *  - discovery/enrichment candidate filtering (`getCandidate*`),
 *  - the streaming hydrate-activity reducer (`applyStreamEvent`),
 *  - post-run workspace routing (`getNextWorkspaceTabAfterAssetIntelRun`),
 *  - provider status tone + evidence raw-row extraction.
 */

import type {
  AssetIntelProviderDescriptor,
  AssetIntelProviderRunStatus,
  AssetIntelProviderRuntimeKind,
  AssetIntelRun,
  AssetIntelStreamEvent,
} from "@/lib/api/asset-intel";
import type { OrganizationCandidate, OrganizationCandidates } from "@/lib/api/organizations";
import type { AssetIntelOrgActionKind, EngagementRecord, WorkspaceTab } from "./types";

export function getCandidateCounts(
  engagement: EngagementRecord | null | undefined,
  allowedSources?: ReadonlySet<string>
): {
  organizations: number;
  targets: number;
} {
  const candidates = engagement?.candidates;
  if (!candidates || typeof candidates !== "object" || Array.isArray(candidates)) {
    return { organizations: 0, targets: 0 };
  }
  const record = candidates as { organizations?: unknown; targets?: unknown };
  if (allowedSources) {
    return {
      organizations: getCandidateItems(engagement, "organizations", allowedSources).length,
      targets: getCandidateItems(engagement, "targets", allowedSources).length,
    };
  }
  return {
    organizations: Array.isArray(record.organizations) ? record.organizations.length : 0,
    targets: Array.isArray(record.targets) ? record.targets.length : 0,
  };
}

function normalizeCandidateSource(source: unknown): string {
  return typeof source === "string" ? source.trim().toLowerCase() : "";
}

export function getCandidateItems(
  engagement: EngagementRecord | null | undefined,
  kind: "organizations" | "targets",
  allowedSources?: ReadonlySet<string>
): OrganizationCandidate[] {
  const candidates = engagement?.candidates;
  if (!candidates || typeof candidates !== "object" || Array.isArray(candidates)) return [];
  const items = (candidates as Record<string, unknown>)[kind];
  if (!Array.isArray(items)) return [];
  const typed = items as OrganizationCandidate[];
  if (!allowedSources) return typed;
  return typed.filter((item) => {
    const source = normalizeCandidateSource(item.source);
    return !source || allowedSources.has(source);
  });
}

export function getCandidateSourceFilter(
  providers: AssetIntelProviderDescriptor[],
  phase: "discovery" | "enrichment" | null
): ReadonlySet<string> | undefined {
  if (!phase) return undefined;
  const sources = new Set<string>();
  for (const provider of providers) {
    const isDiscovery = provider.capabilities.includes("subsidiaries");
    if ((phase === "discovery" && isDiscovery) || (phase === "enrichment" && !isDiscovery)) {
      sources.add(provider.id.trim().toLowerCase());
      sources.add(provider.displayName.trim().toLowerCase());
    }
  }
  return sources.size > 0 ? sources : undefined;
}

export function getVisibleCandidateBuckets(
  engagement: EngagementRecord | null | undefined,
  phase: "discovery" | "enrichment" | null,
  allowedSources?: ReadonlySet<string>
): OrganizationCandidates {
  return {
    organizations: getCandidateItems(engagement, "organizations", allowedSources),
    targets: phase === "discovery" ? [] : getCandidateItems(engagement, "targets", allowedSources),
  };
}

export function getNextWorkspaceTabAfterAssetIntelRun(
  action: AssetIntelOrgActionKind,
  run: AssetIntelRun
): WorkspaceTab | null {
  if (action !== "hydrate_subsidiaries") return null;
  if (run.status !== "completed") return "activity";
  const candidateCount =
    (run.candidates.organizations?.length ?? 0) + (run.candidates.targets?.length ?? 0);
  return candidateCount > 0 ? "candidates" : "activity";
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

/**
 * Keys we surface from `candidate.evidence.raw` in the inline "Details" panel.
 * Picked from ENScan_GO + 0.zone common output shapes so the user can sanity
 * check why a row was promoted to a candidate (e.g. invest scale, registration
 * code, address, legal rep, ICP info, app store link). Anything else is hidden
 * to keep the panel scannable; full raw JSON is still on disk via evidence.run.
 */
const EVIDENCE_RAW_KEY_WHITELIST: Array<{ field: string; label: string }> = [
  { field: "name", label: "Name" },
  { field: "company_name", label: "Company" },
  { field: "reg_code", label: "Credit code" },
  { field: "credit_code", label: "Credit code" },
  { field: "scale", label: "Ownership %" },
  { field: "pid", label: "Provider company id" },
  { field: "legal", label: "Legal representative" },
  { field: "legal_person", label: "Legal representative" },
  { field: "industry", label: "Industry" },
  { field: "addr", label: "Address" },
  { field: "address", label: "Address" },
  { field: "reg_date", label: "Registered at" },
  { field: "establish_date", label: "Established" },
  { field: "phone", label: "Phone" },
  { field: "email", label: "Email" },
  { field: "domain", label: "Domain" },
  { field: "url", label: "URL" },
  { field: "link", label: "App URL" },
  { field: "app_id", label: "App id" },
  { field: "app_url", label: "App URL" },
  { field: "type", label: "Type" },
  { field: "entity_type", label: "Entity type" },
  { field: "status", label: "Status" },
];

export interface EvidenceRawRow {
  field: string;
  label: string;
  value: string;
}

/**
 * Extract a flat (field, label, value) list from a candidate's evidence.raw
 * payload, keeping only the curated keys above. Returns [] when raw is
 * missing / not an object — callers should treat that as "no details to show".
 */
export function getEvidenceRawRows(evidence: unknown): EvidenceRawRow[] {
  if (!evidence || typeof evidence !== "object" || Array.isArray(evidence)) return [];
  const raw = (evidence as { raw?: unknown }).raw;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return [];
  const record = raw as Record<string, unknown>;
  const seen = new Set<string>();
  const out: EvidenceRawRow[] = [];
  for (const entry of EVIDENCE_RAW_KEY_WHITELIST) {
    if (seen.has(entry.label)) continue;
    const value = record[entry.field];
    if (value === null || value === undefined) continue;
    const text =
      typeof value === "string"
        ? value.trim()
        : typeof value === "number" || typeof value === "boolean"
          ? String(value)
          : null;
    if (!text) continue;
    seen.add(entry.label);
    out.push({ field: entry.field, label: entry.label, value: text });
  }
  return out;
}
