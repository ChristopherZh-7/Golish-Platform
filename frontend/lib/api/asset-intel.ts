import { listen } from "@/lib/tauri-listen";
import { invoke } from "./client";
import type { OrganizationCandidates } from "./organizations";

/**
 * Tauri event channel used for all Asset Intel streaming events.
 *
 * Must stay in sync with the backend constant `ASSET_INTEL_EVENT`
 * in `backend/crates/golish/src/tools/asset_intel.rs`.
 */
export const ASSET_INTEL_EVENT = "asset-intel:event";

export type AssetIntelCapability =
  | "subsidiaries"
  | "domains"
  | "icp"
  | "apps"
  | "mini_programs"
  | "social_accounts"
  | "contacts";

export type AssetIntelProviderStatus = "available" | "unavailable" | "deprecated";

export interface AssetIntelIntegrationRequirement {
  toolId: string;
  groupIds: string[];
}

export interface AssetIntelProviderDescriptor {
  id: string;
  displayName: string;
  requiresIntegration: AssetIntelIntegrationRequirement | null;
  capabilities: AssetIntelCapability[];
  status: AssetIntelProviderStatus;
}

export interface AssetIntelHydrateConfig {
  minOwnershipPercent?: string | null;
  depth?: string | null;
  includeBranches?: boolean | null;
  createCandidates?: boolean | null;
}

export interface AssetIntelHydrateArgs {
  organizationId: string;
  companyName?: string | null;
  providerIds?: string[];
  config?: AssetIntelHydrateConfig;
}

export type AssetIntelRunStatus = "completed" | "partial" | "failed";
export type AssetIntelProviderRunState = "completed" | "checked_empty" | "unavailable" | "failed";

export interface AssetIntelProviderRunStatus {
  providerId: string;
  status: AssetIntelProviderRunState;
  message: string;
}

export interface AssetIntelRun {
  runId: string;
  status: AssetIntelRunStatus;
  providerStatus: AssetIntelProviderRunStatus[];
  candidates: OrganizationCandidates;
  evidence: unknown[];
}

export async function listProviders(): Promise<AssetIntelProviderDescriptor[]> {
  return invoke<AssetIntelProviderDescriptor[]>("asset_intel_list_providers");
}

/**
 * Legacy single-shot hydrate (every auto provider). Kept for backward
 * compatibility; new UI should call `hydrateSubsidiaries` followed by
 * `enrichOrganization` / `enrichBatch` to get the two-phase semantics
 * described in `docs/design/2026-05-22-asset-intel-two-phase-hydrate.md`.
 */
export async function hydrate(args: AssetIntelHydrateArgs): Promise<AssetIntelRun> {
  return invoke<AssetIntelRun>("asset_intel_hydrate", {
    args: {
      organizationId: args.organizationId,
      companyName: args.companyName ?? null,
      providerIds: args.providerIds ?? [],
      config: args.config ?? {},
    },
  });
}

/**
 * Phase 1 of the two-phase hydrate flow — discovery only.
 *
 * Runs providers whose `capabilities` set contains `"subsidiaries"`
 * (currently enscan-go). Returns organization + target candidates written
 * under the master organization's id. The user then reviews + promotes
 * the org candidates before calling `enrichBatch` to fan out 0.zone-style
 * enrichment to each promoted child.
 */
export async function hydrateSubsidiaries(args: AssetIntelHydrateArgs): Promise<AssetIntelRun> {
  return invoke<AssetIntelRun>("asset_intel_hydrate_subsidiaries", {
    args: {
      organizationId: args.organizationId,
      companyName: args.companyName ?? null,
      providerIds: args.providerIds ?? [],
      config: args.config ?? {},
    },
  });
}

/**
 * Args for `enrichOrganization`. Intentionally has no `companyName` — the
 * backend always uses the org's canonical name so 0.zone (and friends) get
 * queried with the same identity the user approved during promotion.
 */
export interface AssetIntelEnrichOrganizationArgs {
  organizationId: string;
  providerIds?: string[];
  config?: AssetIntelHydrateConfig;
}

/**
 * Phase 2 of the two-phase hydrate flow — enrichment of a single org.
 *
 * Runs providers whose `capabilities` set does **not** contain
 * `"subsidiaries"` (currently 0.zone). Candidates + master profile fields
 * land on the targeted org, not on its parent.
 */
export async function enrichOrganization(
  args: AssetIntelEnrichOrganizationArgs
): Promise<AssetIntelRun> {
  return invoke<AssetIntelRun>("asset_intel_enrich_organization", {
    args: {
      organizationId: args.organizationId,
      providerIds: args.providerIds ?? [],
      config: args.config ?? {},
    },
  });
}

/**
 * Args for `enrichBatch`. `includeSelf` defaults to `true` so clicking
 * "批量补字段" on the master org enriches both the master + every direct
 * child in one shot.
 */
export interface AssetIntelEnrichBatchArgs {
  parentOrganizationId: string;
  includeSelf?: boolean | null;
  providerIds?: string[];
  config?: AssetIntelHydrateConfig;
}

/**
 * One skipped org in an `enrichBatch` result. `reason` is a short
 * machine-readable token (`empty_name` / `no_enrichment_provider` /
 * `provider_select_error: …` / `run_failed: …`) — frontend should map
 * known prefixes to human copy.
 */
export interface AssetIntelEnrichBatchSkip {
  organizationId: string;
  reason: string;
}

export interface AssetIntelEnrichBatchResult {
  runs: AssetIntelRun[];
  skipped: AssetIntelEnrichBatchSkip[];
}

/**
 * Phase 2 of the two-phase hydrate flow — batch enrichment of a parent +
 * every direct child. Failures on one org don't abort the batch; they are
 * captured in `skipped` with a reason token.
 */
export async function enrichBatch(
  args: AssetIntelEnrichBatchArgs
): Promise<AssetIntelEnrichBatchResult> {
  return invoke<AssetIntelEnrichBatchResult>("asset_intel_enrich_batch", {
    args: {
      parentOrganizationId: args.parentOrganizationId,
      includeSelf: args.includeSelf ?? true,
      providerIds: args.providerIds ?? [],
      config: args.config ?? {},
    },
  });
}

/**
 * One disambiguation match returned by `asset_intel_lookup_company`. The
 * UI shows this in a candidate list before the user commits a canonical
 * company name + credit code to a full hydrate run.
 */
export interface LookupCompanyMatch {
  providerId: string;
  name: string;
  creditCode?: string | null;
  industry?: string | null;
  legalRepresentative?: string | null;
  address?: string | null;
  registeredAt?: string | null;
  confidence: number;
  evidence?: unknown;
}

export interface AssetIntelLookupRequest {
  keyword: string;
  providerIds?: string[];
  limit?: number | null;
}

export interface AssetIntelLookupResult {
  runId: string;
  matches: LookupCompanyMatch[];
  providerStatus: AssetIntelProviderRunStatus[];
}

/**
 * Run a quick disambiguation lookup across every provider that declares a
 * `lookup` descriptor. Use before `hydrate` to let the user pick a single
 * canonical company so the full scan doesn't fan out on a fuzzy keyword.
 */
export async function lookupCompany(
  args: AssetIntelLookupRequest
): Promise<AssetIntelLookupResult> {
  return invoke<AssetIntelLookupResult>("asset_intel_lookup_company", {
    args: {
      keyword: args.keyword,
      providerIds: args.providerIds ?? [],
      limit: args.limit ?? null,
    },
  });
}

/**
 * Streaming event payload emitted by the backend during an Asset Intel run.
 *
 * Discriminated by `kind`. A single run may emit any number of progress /
 * batch events between `provider_started` and `provider_completed`.
 */
export type AssetIntelStreamSource = "stdout" | "stderr" | "system";
export type AssetIntelBatchSource = "stdout" | "artifact" | "http";
export type AssetIntelProviderRuntimeKind = "cli_json" | "http_json";

export type AssetIntelStreamEvent =
  | {
      kind: "provider_started";
      runId: string;
      providerId: string;
      displayName: string;
      runtime: AssetIntelProviderRuntimeKind;
    }
  | {
      kind: "provider_progress";
      runId: string;
      providerId: string;
      message: string;
      stream: AssetIntelStreamSource;
    }
  | {
      kind: "provider_batch";
      runId: string;
      providerId: string;
      candidates: OrganizationCandidates;
      source: AssetIntelBatchSource;
      artifact?: string | null;
      requestId?: string | null;
    }
  | {
      kind: "provider_completed";
      runId: string;
      providerId: string;
      status: AssetIntelProviderRunStatus;
      candidateCount: number;
    };

/**
 * Subscribe to Asset Intel streaming events.
 *
 * Returns an unsubscribe function. Callers typically attach this in a
 * `useEffect` and clean up on unmount. The callback fires in event order.
 */
export async function listenStream(
  onEvent: (event: AssetIntelStreamEvent) => void
): Promise<() => void> {
  const unlisten = await listen<AssetIntelStreamEvent>(ASSET_INTEL_EVENT, (envelope) => {
    onEvent(envelope.payload);
  });
  return unlisten;
}
