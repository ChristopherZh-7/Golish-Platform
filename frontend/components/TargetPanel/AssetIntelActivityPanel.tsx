/**
 * `AssetIntelActivityPanel` — the workspace "Activity" tab.
 *
 * Extracted verbatim from `TargetGroupedView.tsx`'s `renderWorkspacePanel`. Shows
 * the live streaming hydrate run (per-provider progress), the discovery/enrich
 * launch buttons, the last persisted run summary, and the available providers.
 */

import { Download, Loader2, Network, Shield, Wifi } from "lucide-react";
import type { AssetIntelProviderDescriptor, AssetIntelRun } from "@/lib/api/asset-intel";
import type { OrganizationReconRunSnapshot } from "@/lib/api/organization-recon";
import type { Organization } from "@/lib/api/organizations";
import { getProviderStatusClass, type HydrateActivity } from "@/lib/target-panel/asset-intel";
import { isAssetIntelOrgActionItem } from "@/lib/target-panel/engagement";
import { translateWithFallback } from "@/lib/target-panel/org-fields";
import {
  canExportCurrentReconAssets,
  findReconAssetsWorkbook,
  isOrganizationReconRunning,
} from "@/lib/target-panel/organization-recon";
import type { AssetIntelOrgActionKind, OrgActionItem } from "@/lib/target-panel/types";
import { cn } from "@/lib/utils";
import type { EngagementMode } from "./NewEngagementDialog";

interface AssetIntelActivityPanelProps {
  t: (key: string) => string;
  selectedOrg: Organization;
  selectedMode: EngagementMode | null;
  selectedOrgActions: { primary: OrgActionItem; secondary?: OrgActionItem };
  isHydratingSelected: boolean;
  hydratingAction: AssetIntelOrgActionKind | null;
  selectedActivity: HydrateActivity | undefined;
  hydrateRun: AssetIntelRun | undefined;
  hydrateError: string | undefined;
  assetProviders: AssetIntelProviderDescriptor[];
  handleRunAssetIntel: (org: Organization, action: AssetIntelOrgActionKind) => void;
  organizationReconRun: OrganizationReconRunSnapshot | undefined;
  organizationReconError: string | undefined;
  hasInScopeTargets: boolean;
  handleRunOrganizationRecon: (org: Organization, allowActive: boolean) => void;
  handleExportOrganizationReconAssets: (
    org: Organization,
    run?: OrganizationReconRunSnapshot
  ) => void;
}

export function AssetIntelActivityPanel({
  t,
  selectedOrg,
  selectedMode,
  selectedOrgActions,
  isHydratingSelected,
  hydratingAction,
  selectedActivity,
  hydrateRun,
  hydrateError,
  assetProviders,
  handleRunAssetIntel,
  organizationReconRun,
  organizationReconError,
  hasInScopeTargets,
  handleRunOrganizationRecon,
  handleExportOrganizationReconAssets,
}: AssetIntelActivityPanelProps) {
  const organizationReconRunning = isOrganizationReconRunning(organizationReconRun);
  const reconAssetsWorkbook = findReconAssetsWorkbook(organizationReconRun);
  const canExportAssets =
    Boolean(reconAssetsWorkbook) || canExportCurrentReconAssets(hydrateRun?.status);
  return (
    <section className="rounded border border-border/35 bg-muted/5 p-3">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h4 className="text-xs font-medium text-foreground">
            {translateWithFallback(t, "targetWorkspace.activity.title", "Asset intel activity")}
          </h4>
          <p className="mt-1 text-[10px] text-muted-foreground/70">
            {translateWithFallback(
              t,
              "targetWorkspace.activity.description",
              "Discover subsidiaries first, then enrich approved organizations with asset fields."
            )}
          </p>
        </div>
        {selectedMode === "discover_assets" && (
          <div className="flex flex-wrap justify-end gap-1">
            {[selectedOrgActions.primary, selectedOrgActions.secondary]
              .filter(isAssetIntelOrgActionItem)
              .map((action) => (
                <button
                  key={`activity:${action.kind}`}
                  type="button"
                  className={cn(
                    "inline-flex items-center gap-1 rounded border px-2 py-1 text-[10px]",
                    action.kind === "hydrate_subsidiaries"
                      ? "border-blue-500/30 bg-blue-500/10 text-blue-300 hover:bg-blue-500/15"
                      : "border-cyan-500/30 bg-cyan-500/10 text-cyan-300 hover:bg-cyan-500/15",
                    isHydratingSelected && "opacity-70"
                  )}
                  disabled={isHydratingSelected}
                  onClick={() => void handleRunAssetIntel(selectedOrg, action.kind)}
                >
                  {isHydratingSelected && hydratingAction === action.kind ? (
                    <Loader2 className="w-3 h-3 animate-spin" />
                  ) : action.kind === "hydrate_subsidiaries" ? (
                    <Network className="w-3 h-3" />
                  ) : (
                    <Wifi className="w-3 h-3" />
                  )}
                  {action.label}
                </button>
              ))}
          </div>
        )}
      </div>

      <div className="mt-3 rounded border border-border/30 bg-background/25 p-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <p className="text-[10px] font-medium text-foreground">Staged organization recon</p>
            <p className="mt-0.5 text-[9px] text-muted-foreground">
              Online collection is explicit. Active tools additionally require approved in-scope
              targets.
            </p>
          </div>
          <div className="flex flex-wrap gap-1">
            <button
              type="button"
              className="inline-flex items-center gap-1 rounded border border-blue-500/30 bg-blue-500/10 px-2 py-1 text-[10px] text-blue-300 hover:bg-blue-500/15 disabled:opacity-50"
              disabled={organizationReconRunning}
              onClick={() => void handleRunOrganizationRecon(selectedOrg, false)}
            >
              {organizationReconRunning ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <Network className="h-3 w-3" />
              )}
              Run staged recon
            </button>
            <button
              type="button"
              className="inline-flex items-center gap-1 rounded border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-[10px] text-amber-300 hover:bg-amber-500/15 disabled:opacity-50"
              disabled={organizationReconRunning || !hasInScopeTargets}
              title={
                hasInScopeTargets
                  ? "Run online collection and authorized active tools"
                  : "Add an in-scope target to enable active collection"
              }
              onClick={() => void handleRunOrganizationRecon(selectedOrg, true)}
            >
              <Shield className="h-3 w-3" />
              Run with active tools
            </button>
            {canExportAssets && (
              <button
                type="button"
                className="inline-flex items-center gap-1 rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-[10px] text-emerald-300 hover:bg-emerald-500/15"
                title={
                  reconAssetsWorkbook
                    ? `Download ${reconAssetsWorkbook.bytes} bytes from Stage 4 processing`
                    : "Export current organization assets from the latest completed enrichment"
                }
                onClick={() =>
                  void handleExportOrganizationReconAssets(selectedOrg, organizationReconRun)
                }
              >
                <Download className="h-3 w-3" />
                Export Excel
              </button>
            )}
          </div>
        </div>
        {organizationReconError && (
          <div className="mt-2 rounded border border-red-500/30 bg-red-500/5 p-2 text-[10px] text-red-300">
            {organizationReconError}
          </div>
        )}
        {organizationReconRun && (
          <div className="mt-2 space-y-1">
            <div className="flex items-center justify-between text-[10px] text-muted-foreground">
              <span>run {organizationReconRun.runId.slice(0, 8)}</span>
              <span className="uppercase tracking-wide">{organizationReconRun.status}</span>
            </div>
            {organizationReconRun.stages.map((stage) => {
              const errors = organizationReconRun.tasks
                .filter((task) => task.stage === stage.stage)
                .flatMap((task) => task.errors);
              return (
                <div
                  key={`${organizationReconRun.runId}:${stage.stage}`}
                  className={cn(
                    "rounded border p-2 text-[10px]",
                    getProviderStatusClass(stage.status)
                  )}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-medium">{stage.stage}</span>
                    <span>{stage.status}</span>
                  </div>
                  {errors.length > 0 && (
                    <p className="mt-1 font-mono text-[9px] opacity-85">
                      {errors.map((error) => error.code).join(", ")}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {hydrateError && (
        <div className="mt-3 rounded border border-red-500/30 bg-red-500/5 p-2 text-[11px] text-red-300">
          {hydrateError}
        </div>
      )}

      {isHydratingSelected && (
        <div className="mt-3 space-y-2">
          <div className="flex items-center gap-2 rounded border border-blue-500/25 bg-blue-500/5 p-2 text-[11px] text-blue-300">
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
            {translateWithFallback(
              t,
              "targetWorkspace.activity.runningProviders",
              "Running asset intel providers"
            )}
            {selectedActivity?.runId && (
              <span className="ml-auto text-[9px] uppercase tracking-wide text-blue-300/70">
                run {selectedActivity.runId.slice(0, 8)}
              </span>
            )}
          </div>
          {selectedActivity?.providerOrder.map((providerId) => {
            const activity = selectedActivity.providers[providerId];
            if (!activity) return null;
            return (
              <div
                key={`activity:${providerId}`}
                className={cn(
                  "rounded border bg-background/40 p-2",
                  activity.state === "completed" && activity.status
                    ? getProviderStatusClass(activity.status.status)
                    : "border-blue-500/25"
                )}
              >
                <div className="flex items-center justify-between gap-2 text-[11px]">
                  <span className="font-medium">{activity.displayName}</span>
                  <span className="text-[10px] uppercase tracking-wide opacity-75">
                    {activity.state === "running"
                      ? (activity.runtime ?? "running")
                      : (activity.status?.status ?? "completed")}
                  </span>
                </div>
                <div className="mt-1 flex items-center gap-3 text-[10px] text-muted-foreground">
                  <span>{activity.candidateCount} candidates</span>
                  <span>{activity.batchCount} batches</span>
                </div>
                {activity.recentMessages.length > 0 && (
                  <ul className="mt-1 space-y-0.5 text-[10px] text-muted-foreground/90 max-h-32 overflow-auto">
                    {activity.recentMessages.map((line, idx) => (
                      <li
                        key={`${providerId}:msg:${idx}`}
                        className="font-mono whitespace-pre-wrap break-words"
                      >
                        {line}
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            );
          })}
        </div>
      )}

      {!isHydratingSelected && !hydrateRun && !hydrateError && (
        <div className="mt-3 rounded border border-dashed border-border/35 p-3 text-center">
          <p className="text-[11px] text-muted-foreground">
            {translateWithFallback(t, "targetWorkspace.activity.noRun", "No asset intel run yet.")}
          </p>
        </div>
      )}

      {assetProviders.length > 0 && (
        <div className="mt-3 rounded border border-border/30 bg-background/25 p-2">
          <p className="text-[10px] font-medium text-muted-foreground">
            {translateWithFallback(
              t,
              "targetWorkspace.activity.availableProviders",
              "Available providers"
            )}
          </p>
          <div className="mt-1 flex flex-wrap gap-1">
            {assetProviders.map((provider) => (
              <span
                key={provider.id}
                className="rounded border border-border/35 bg-muted/10 px-1.5 py-0.5 text-[9px] text-muted-foreground"
              >
                {provider.displayName}
              </span>
            ))}
          </div>
        </div>
      )}

      {hydrateRun && (
        <div className="mt-3 space-y-2">
          <div className="flex items-center justify-between rounded border border-border/30 bg-background/35 p-2">
            <span className="text-[10px] text-muted-foreground">
              {translateWithFallback(t, "targetWorkspace.activity.lastRun", "Last run")}
            </span>
            <span className="text-[10px] text-foreground">{hydrateRun.status}</span>
          </div>
          {hydrateRun.providerStatus.map((provider) => (
            <div
              key={`${hydrateRun.runId}:${provider.providerId}`}
              className={cn(
                "rounded border p-2 text-[11px]",
                getProviderStatusClass(provider.status)
              )}
            >
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium">{provider.providerId}</span>
                <span>{provider.status}</span>
              </div>
              <p className="mt-1 text-[10px] opacity-80">{provider.message}</p>
            </div>
          ))}
        </div>
      )}
      <div className="mt-3 rounded border border-border/30 bg-background/25 p-2 text-[10px] text-muted-foreground">
        {translateWithFallback(
          t,
          "targetWorkspace.activity.candidateScopeHint",
          "Candidates from asset intel runs stay out of active scan scope until approved and promoted."
        )}
      </div>
    </section>
  );
}
