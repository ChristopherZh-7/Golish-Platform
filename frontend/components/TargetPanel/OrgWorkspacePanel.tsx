/**
 * `OrgWorkspacePanel` — the right-hand workspace shown when an org (not a single
 * target) is selected.
 *
 * Extracted from `TargetGroupedView.tsx`'s `renderWorkspacePanel`. Owns the org
 * header, the tab nav, and the Fields / Overview / Scope tabs inline; delegates
 * the two heaviest tabs to `AssetIntelActivityPanel` and `CandidateReviewList`.
 */

import { Building2, Crosshair, Globe } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import type { AssetIntelProviderDescriptor, AssetIntelRun } from "@/lib/api/asset-intel";
import type { OrganizationReconRunSnapshot } from "@/lib/api/organization-recon";
import type { Organization, OrganizationCandidate } from "@/lib/api/organizations";
import type { Target } from "@/lib/pentest/types";
import {
  getCandidateSourceFilter,
  getVisibleCandidateBuckets,
  type HydrateActivity,
} from "@/lib/target-panel/asset-intel";
import {
  ENGAGEMENT_BADGES,
  getEngagementDetails,
  getEngagementRecord,
  getOrgActionModel,
  getWorkspaceModel,
} from "@/lib/target-panel/engagement";
import {
  getOrgFieldGroups,
  translateOrgFieldGroups,
  translateWithFallback,
} from "@/lib/target-panel/org-fields";
import type { AssetIntelOrgActionKind, WorkspaceTab } from "@/lib/target-panel/types";
import { cn } from "@/lib/utils";
import { AssetIntelActivityPanel } from "./AssetIntelActivityPanel";
import { CandidateReviewList } from "./CandidateReviewList";
import type { EngagementMode } from "./NewEngagementDialog";
import { OrgFieldRow } from "./OrgFieldRow";
import { TYPE_ICONS } from "./targetTypeIcons";

interface OrgWorkspacePanelProps {
  selectedOrg: Organization | null;
  selectedMode: EngagementMode | null;
  selectedTargets: Target[];
  t: (key: string) => string;
  workspaceTab: WorkspaceTab;
  setWorkspaceTab: Dispatch<SetStateAction<WorkspaceTab>>;
  assetProviders: AssetIntelProviderDescriptor[];
  hydrateRuns: Record<string, AssetIntelRun>;
  hydrateErrors: Record<string, string>;
  hydrateActivity: Record<string, HydrateActivity>;
  organizationReconRuns: Record<string, OrganizationReconRunSnapshot>;
  organizationReconErrors: Record<string, string>;
  hydratingOrgId: string | null;
  hydratingAction: AssetIntelOrgActionKind | null;
  candidateUpdatingId: string | null;
  candidatePromotingId: string | null;
  expandedCandidateIds: Set<string>;
  setExpandedCandidateIds: Dispatch<SetStateAction<Set<string>>>;
  setEditingTargetId: Dispatch<SetStateAction<string | null>>;
  handleRunAssetIntel: (org: Organization, action: AssetIntelOrgActionKind) => void;
  handleRunOrganizationRecon: (org: Organization, allowActive: boolean) => void;
  handleExportOrganizationReconAssets: (
    org: Organization,
    run?: OrganizationReconRunSnapshot
  ) => void;
  handlePromoteCandidate: (candidate: OrganizationCandidate) => void;
  handleCandidateStatus: (
    candidate: OrganizationCandidate,
    status: "approved" | "rejected"
  ) => void;
}

export function OrgWorkspacePanel({
  selectedOrg,
  selectedMode,
  selectedTargets,
  t,
  workspaceTab,
  setWorkspaceTab,
  assetProviders,
  hydrateRuns,
  hydrateErrors,
  hydrateActivity,
  organizationReconRuns,
  organizationReconErrors,
  hydratingOrgId,
  hydratingAction,
  candidateUpdatingId,
  candidatePromotingId,
  expandedCandidateIds,
  setExpandedCandidateIds,
  setEditingTargetId,
  handleRunAssetIntel,
  handleRunOrganizationRecon,
  handleExportOrganizationReconAssets,
  handlePromoteCandidate,
  handleCandidateStatus,
}: OrgWorkspacePanelProps) {
  if (!selectedOrg) {
    return (
      <div className="h-full flex items-center justify-center text-center text-muted-foreground px-8">
        <div>
          <Building2 className="w-8 h-8 mx-auto mb-2 opacity-30" />
          <p className="text-xs">
            {translateWithFallback(
              t,
              "targetWorkspace.empty.selectOrg",
              "Select or create an organization to start."
            )}
          </p>
        </div>
      </div>
    );
  }

  const workspace = getWorkspaceModel(selectedMode);
  const engagementRecord = getEngagementRecord(selectedOrg);
  const engagementDetails = getEngagementDetails(engagementRecord);
  const hydrateRun = hydrateRuns[selectedOrg.id];
  const hydrateError = hydrateErrors[selectedOrg.id];
  const selectedActivity = hydrateActivity[selectedOrg.id];
  const organizationReconRun = organizationReconRuns[selectedOrg.id];
  const organizationReconError = organizationReconErrors[selectedOrg.id];
  const isHydratingSelected = hydratingOrgId === selectedOrg.id;
  const selectedOrgIsChild = Boolean(selectedOrg.parent_id);
  const candidatePhase =
    selectedMode === "discover_assets" ? (selectedOrgIsChild ? "enrichment" : "discovery") : null;
  const candidateSourceFilter = getCandidateSourceFilter(assetProviders, candidatePhase);
  const visibleCandidates = getVisibleCandidateBuckets(
    engagementRecord,
    candidatePhase,
    candidateSourceFilter
  );
  const organizationCandidates = visibleCandidates.organizations;
  const targetCandidates = visibleCandidates.targets;
  const candidateCounts = {
    organizations: organizationCandidates.length,
    targets: targetCandidates.length,
  };
  const inScopeCount = selectedTargets.filter((target) => target.scope === "in").length;
  const outScopeCount = selectedTargets.filter((target) => target.scope === "out").length;
  const badge = selectedMode ? ENGAGEMENT_BADGES[selectedMode] : null;
  const fieldGroups = translateOrgFieldGroups(getOrgFieldGroups(selectedOrg), t);
  const selectedOrgActions = getOrgActionModel(selectedMode, {
    isChild: selectedOrgIsChild,
  });

  return (
    <div className="h-full overflow-y-auto p-3 space-y-3">
      <section className="border-b border-border/30 pb-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-[9px] uppercase tracking-wide text-muted-foreground/55">
              {workspace.eyebrow}
            </p>
            <div className="mt-1 flex items-center gap-1.5 min-w-0">
              <Building2 className="w-3 h-3 text-accent/80 flex-shrink-0" />
              <h3 className="text-[12px] font-medium text-foreground truncate">
                {selectedOrg.name}
              </h3>
              {badge && (
                <span className={cn("text-[9px] px-1.5 py-0.5 rounded", badge.className)}>
                  {badge.label}
                </span>
              )}
            </div>
            <p className="mt-1 text-[10px] text-muted-foreground/70 leading-relaxed">
              {workspace.description}
            </p>
            {engagementDetails.length > 0 && (
              <div className="mt-2 flex flex-wrap gap-1">
                {engagementDetails.map(([label, value]) => (
                  <span
                    key={`${label}:${value}`}
                    className="rounded border border-border/35 bg-background/40 px-1.5 py-0.5 text-[9px] text-muted-foreground"
                  >
                    {label}: <span className="text-foreground/80">{value}</span>
                  </span>
                ))}
              </div>
            )}
          </div>
        </div>

        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          <span className="rounded bg-muted/20 px-1.5 py-0.5 text-[10px] text-muted-foreground">
            {translateWithFallback(t, "targetWorkspace.metrics.targets", "Targets")}{" "}
            <span className="text-foreground">{selectedTargets.length}</span>
          </span>
          <span className="rounded bg-green-500/10 px-1.5 py-0.5 text-[10px] text-green-400">
            {translateWithFallback(t, "targetWorkspace.metrics.in", "In")}{" "}
            <span className="text-green-300">{inScopeCount}</span>
          </span>
          <span className="rounded bg-muted/20 px-1.5 py-0.5 text-[10px] text-muted-foreground">
            {translateWithFallback(t, "targetWorkspace.metrics.out", "Out")}{" "}
            <span className="text-foreground/75">{outScopeCount}</span>
          </span>
        </div>
      </section>

      <nav className="flex items-center gap-1 border-b border-border/30 pb-2">
        {[
          ["overview", translateWithFallback(t, "targetWorkspace.tabs.overview", "Overview")],
          ["fields", translateWithFallback(t, "targetWorkspace.tabs.fields", "Fields")],
          ["scope", translateWithFallback(t, "targetWorkspace.tabs.scope", "Scope")],
          ["targets", translateWithFallback(t, "targetWorkspace.tabs.targets", "Targets")],
          ["candidates", translateWithFallback(t, "targetWorkspace.tabs.candidates", "Candidates")],
          ["activity", translateWithFallback(t, "targetWorkspace.tabs.activity", "Activity")],
        ].map(([id, label]) => (
          <button
            key={id}
            type="button"
            className={cn(
              "px-2 py-1 rounded text-[10px] transition-colors",
              workspaceTab === id
                ? "bg-accent/15 text-accent"
                : "text-muted-foreground hover:bg-muted/30 hover:text-foreground"
            )}
            onClick={() => setWorkspaceTab(id as WorkspaceTab)}
          >
            {label}
          </button>
        ))}
      </nav>

      {workspaceTab === "activity" && (
        <AssetIntelActivityPanel
          t={t}
          selectedOrg={selectedOrg}
          selectedMode={selectedMode}
          selectedOrgActions={selectedOrgActions}
          isHydratingSelected={isHydratingSelected}
          hydratingAction={hydratingAction}
          selectedActivity={selectedActivity}
          hydrateRun={hydrateRun}
          hydrateError={hydrateError}
          assetProviders={assetProviders}
          handleRunAssetIntel={handleRunAssetIntel}
          organizationReconRun={organizationReconRun}
          organizationReconError={organizationReconError}
          hasInScopeTargets={inScopeCount > 0}
          handleRunOrganizationRecon={handleRunOrganizationRecon}
          handleExportOrganizationReconAssets={handleExportOrganizationReconAssets}
        />
      )}

      {workspaceTab === "fields" && (
        <section className="rounded border border-border/35 bg-muted/5 p-3">
          <h4 className="text-xs font-medium text-foreground">
            {translateWithFallback(t, "targetWorkspace.fieldsPanel.title", "Intel fields")}
          </h4>
          <p className="mt-1 text-[10px] text-muted-foreground/70">
            {translateWithFallback(
              t,
              "targetWorkspace.fieldsPanel.description",
              "Field coverage by group. Editing will be a separate compact mode."
            )}
          </p>
          <div className="mt-3 space-y-2">
            {fieldGroups.map((group) => (
              <div
                key={group.title}
                className="rounded border border-border/30 bg-background/35 p-2.5"
              >
                <div className="flex items-center justify-between">
                  <p className="text-[11px] font-medium text-foreground">{group.title}</p>
                  <span className="text-[10px] text-muted-foreground">
                    {group.fields.filter((item) => item.filled).length}/{group.fields.length}
                  </span>
                </div>
                <div className="mt-2 space-y-2">
                  {group.fields.map((item) => (
                    <OrgFieldRow key={item.key} field={item} />
                  ))}
                </div>
              </div>
            ))}
          </div>
        </section>
      )}

      {(workspaceTab === "overview" || workspaceTab === "targets") && (
        <section className="rounded border border-border/35 bg-muted/5 p-3">
          <div className="flex items-center justify-between gap-2">
            <div>
              <h4 className="text-xs font-medium text-foreground">{workspace.title}</h4>
              <p className="text-[10px] text-muted-foreground/70 mt-1">
                {translateWithFallback(
                  t,
                  "targetWorkspace.overview.placeholder",
                  "Mode-aware workspace skeleton. Backend orchestration and coverage panels come next."
                )}
              </p>
            </div>
          </div>

          {selectedTargets.length === 0 ? (
            <div className="mt-3 rounded border border-dashed border-border/35 p-3 text-center">
              <Crosshair className="w-5 h-5 mx-auto text-muted-foreground/35 mb-1.5" />
              <p className="text-[11px] text-muted-foreground">
                {translateWithFallback(
                  t,
                  "targetWorkspace.overview.noTargets",
                  "No targets linked to this organization yet."
                )}
              </p>
            </div>
          ) : (
            <div className="mt-3 space-y-1">
              {selectedTargets.slice(0, 8).map((target) => (
                <button
                  key={target.id}
                  type="button"
                  className="w-full flex items-center gap-2 rounded px-2 py-1 text-left hover:bg-muted/30"
                  onClick={() => setEditingTargetId(target.id)}
                >
                  {TYPE_ICONS[target.type] || <Globe className="w-3.5 h-3.5" />}
                  <span className="text-xs font-mono text-foreground truncate flex-1">
                    {target.value}
                  </span>
                  <span
                    className={cn(
                      "text-[9px] px-1.5 py-0.5 rounded",
                      target.scope === "in"
                        ? "bg-green-500/10 text-green-400"
                        : "bg-muted/40 text-muted-foreground"
                    )}
                  >
                    {target.scope}
                  </span>
                </button>
              ))}
            </div>
          )}
        </section>
      )}

      {workspaceTab === "candidates" && (
        <CandidateReviewList
          t={t}
          candidateCounts={candidateCounts}
          organizationCandidates={organizationCandidates}
          targetCandidates={targetCandidates}
          candidateUpdatingId={candidateUpdatingId}
          candidatePromotingId={candidatePromotingId}
          expandedCandidateIds={expandedCandidateIds}
          setExpandedCandidateIds={setExpandedCandidateIds}
          handlePromoteCandidate={handlePromoteCandidate}
          handleCandidateStatus={handleCandidateStatus}
        />
      )}

      {workspaceTab === "scope" && (
        <section className="rounded border border-border/35 bg-muted/5 p-3">
          <h4 className="text-xs font-medium text-foreground">
            {translateWithFallback(t, "targetWorkspace.scope.title", "Scope")}
          </h4>
          <p className="mt-1 text-[10px] text-muted-foreground/70">
            {translateWithFallback(
              t,
              "targetWorkspace.scope.description",
              "Scope rules and authorization windows will be edited here."
            )}
          </p>
        </section>
      )}
    </div>
  );
}
