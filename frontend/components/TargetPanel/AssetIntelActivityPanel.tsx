/**
 * `AssetIntelActivityPanel` — the workspace "Activity" tab.
 *
 * Extracted verbatim from `TargetGroupedView.tsx`'s `renderWorkspacePanel`. Shows
 * the live streaming hydrate run (per-provider progress), the discovery/enrich
 * launch buttons, the last persisted run summary, and the available providers.
 */

import { ChevronDown, ChevronRight, Download, Loader2, Network, Wifi } from "lucide-react";
import { useState } from "react";
import type { AssetIntelProviderDescriptor, AssetIntelRun } from "@/lib/api/asset-intel";
import type { OrganizationReconRunSnapshot } from "@/lib/api/organization-recon";
import type { Organization } from "@/lib/api/organizations";
import { getProviderStatusClass, type HydrateActivity } from "@/lib/target-panel/asset-intel";
import { isAssetIntelOrgActionItem } from "@/lib/target-panel/engagement";
import { translateWithFallback } from "@/lib/target-panel/org-fields";
import {
  canExportCurrentReconAssets,
  currentOrganizationReconMessage,
  displayOrganizationReconStatus,
  findReconAssetsWorkbook,
  isOrganizationReconRunning,
  organizationReconLogDetailOperationDisplay,
  organizationReconLogGroupIsRunning,
  organizationReconLogGroupOperationDisplay,
  organizationReconLogGroups,
  organizationReconProgress,
} from "@/lib/target-panel/organization-recon";
import type { AssetIntelOrgActionKind, OrgActionItem } from "@/lib/target-panel/types";
import { cn } from "@/lib/utils";
import type { EngagementMode } from "./NewEngagementDialog";

type OrganizationReconTraceEvent = OrganizationReconRunSnapshot["traceEvents"][number];
const MAX_RECON_DETAIL_LINES = 8;

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
  hasCurrentReconAssets: boolean;
  handleRunOrganizationRecon: (org: Organization) => void;
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
  hasCurrentReconAssets,
  handleRunOrganizationRecon,
  handleExportOrganizationReconAssets,
}: AssetIntelActivityPanelProps) {
  const [expandedReconStages, setExpandedReconStages] = useState<Record<string, boolean>>({});
  const [expandedReconLogs, setExpandedReconLogs] = useState<Record<string, boolean>>({});
  const organizationReconRunning = isOrganizationReconRunning(organizationReconRun);
  const reconAssetsWorkbook = findReconAssetsWorkbook(organizationReconRun);
  const canExportAssets =
    Boolean(reconAssetsWorkbook) ||
    canExportCurrentReconAssets(hydrateRun?.status) ||
    hasCurrentReconAssets;
  const reconProgress = organizationReconProgress(organizationReconRun);
  const reconCurrentMessage = currentOrganizationReconMessage(organizationReconRun);
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
            <p className="text-[10px] font-medium text-foreground">
              {translateWithFallback(
                t,
                "targetWorkspace.organizationRecon.title",
                "组织资产情报流程"
              )}
            </p>
            <p className="mt-0.5 text-[9px] text-muted-foreground">
              {translateWithFallback(
                t,
                "targetWorkspace.organizationRecon.description",
                "按被动收集、主动收集、处理和入库四阶段显示实时状态。"
              )}
            </p>
          </div>
          <div className="flex flex-wrap gap-1">
            <button
              type="button"
              className="inline-flex items-center gap-1 rounded border border-blue-500/30 bg-blue-500/10 px-2 py-1 text-[10px] text-blue-300 hover:bg-blue-500/15 disabled:opacity-50"
              disabled={organizationReconRunning}
              onClick={() => void handleRunOrganizationRecon(selectedOrg)}
            >
              {organizationReconRunning ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <Network className="h-3 w-3" />
              )}
              {translateWithFallback(
                t,
                "targetWorkspace.organizationRecon.runButton",
                "运行分阶段情报"
              )}
            </button>
            {canExportAssets && (
              <button
                type="button"
                className="inline-flex items-center gap-1 rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-[10px] text-emerald-300 hover:bg-emerald-500/15"
                title={
                  reconAssetsWorkbook
                    ? translateWithFallback(
                        t,
                        "targetWorkspace.organizationRecon.exportWorkbookTitle",
                        "下载 Stage 4 生成的资产 Excel"
                      )
                    : translateWithFallback(
                        t,
                        "targetWorkspace.organizationRecon.exportCurrentTitle",
                        "导出当前组织已收集资产"
                      )
                }
                onClick={() =>
                  void handleExportOrganizationReconAssets(selectedOrg, organizationReconRun)
                }
              >
                <Download className="h-3 w-3" />
                {translateWithFallback(
                  t,
                  "targetWorkspace.organizationRecon.exportButton",
                  "导出 Excel"
                )}
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
          <div className="mt-2 space-y-2">
            <div className="flex items-center justify-between text-[10px] text-muted-foreground">
              <span>
                {translateWithFallback(t, "targetWorkspace.organizationRecon.runLabel", "运行")}{" "}
                {organizationReconRun.runId.slice(0, 8)}
              </span>
              <span className="uppercase tracking-wide">
                {organizationReconStatusLabel(t, organizationReconRun.status)}
              </span>
            </div>
            <div className="rounded border border-border/30 bg-background/30 p-2">
              <div className="flex items-center justify-between text-[9px] text-muted-foreground">
                <span>
                  {translateWithFallback(
                    t,
                    "targetWorkspace.organizationRecon.progress",
                    "执行进度"
                  )}
                </span>
                <span>{reconProgress}%</span>
              </div>
              <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-muted/40">
                <div
                  className="h-full rounded-full bg-cyan-400 transition-all"
                  style={{ width: `${reconProgress}%` }}
                />
              </div>
              {reconCurrentMessage && (
                <p className="mt-1 text-[9px] text-cyan-200">
                  {translateWithFallback(
                    t,
                    "targetWorkspace.organizationRecon.currentStep",
                    "当前步骤"
                  )}
                  : {reconCurrentMessage}
                </p>
              )}
            </div>
            {organizationReconRun.stages.map((stage) => {
              const displayStatus = displayOrganizationReconStatus(stage.status);
              const stageTasks = organizationReconRun.tasks.filter(
                (task) => task.stage === stage.stage
              );
              const stageEvents = organizationReconRun.traceEvents.filter(
                (event) => event.stage === stage.stage
              );
              const stageLogGroups = organizationReconLogGroups(stageEvents);
              const hasErrors = stageTasks.some((task) => task.errors.length > 0);
              const isExpanded =
                expandedReconStages[stage.stage] ??
                (stage.status === "running" || stage.status === "failed" || hasErrors);
              return (
                <div
                  key={`${organizationReconRun.runId}:${stage.stage}`}
                  className={cn(
                    "rounded border p-2 text-[10px]",
                    getProviderStatusClass(displayStatus)
                  )}
                >
                  <button
                    type="button"
                    className="flex w-full items-center justify-between gap-2 text-left"
                    onClick={() =>
                      setExpandedReconStages((current) => ({
                        ...current,
                        [stage.stage]: !isExpanded,
                      }))
                    }
                  >
                    <span className="inline-flex items-center gap-1.5 font-medium">
                      {isExpanded ? (
                        <ChevronDown className="h-3 w-3 opacity-70" />
                      ) : (
                        <ChevronRight className="h-3 w-3 opacity-70" />
                      )}
                      {organizationReconStageLabel(t, stage.stage)}
                    </span>
                    <span>{organizationReconStatusLabel(t, displayStatus)}</span>
                  </button>
                  {isExpanded && (
                    <div className="mt-2 space-y-2 border-t border-current/10 pt-2">
                      {stageLogGroups.length > 0 ? (
                        stageLogGroups.map((group) => {
                          const logKey = `${organizationReconRun.runId}:${group.event.id}`;
                          const hasDetails = group.details.length > 0;
                          const operation = organizationReconLogGroupOperationDisplay(group);
                          const groupRunning = organizationReconLogGroupIsRunning(group);
                          const visibleDetails =
                            group.details.length > MAX_RECON_DETAIL_LINES
                              ? group.details.slice(-MAX_RECON_DETAIL_LINES)
                              : group.details;
                          const hiddenDetailCount = group.details.length - visibleDetails.length;
                          const logExpanded =
                            expandedReconLogs[logKey] ?? (groupRunning && hasDetails);
                          return (
                            <div
                              key={logKey}
                              className={cn(
                                "rounded border bg-background/25 p-2",
                                operationToneBorderClass(operation.tone)
                              )}
                            >
                              <button
                                type="button"
                                className="flex w-full items-start gap-2 text-left"
                                onClick={() => {
                                  if (!hasDetails) return;
                                  setExpandedReconLogs((current) => ({
                                    ...current,
                                    [logKey]: !logExpanded,
                                  }));
                                }}
                              >
                                <span className="mt-0.5 inline-flex w-3 shrink-0 justify-center">
                                  {hasDetails ? (
                                    logExpanded ? (
                                      <ChevronDown className="h-3 w-3 opacity-70" />
                                    ) : (
                                      <ChevronRight className="h-3 w-3 opacity-70" />
                                    )
                                  ) : null}
                                </span>
                                <span className="min-w-0 flex-1">
                                  <span className="flex flex-wrap items-center gap-1">
                                    <span
                                      className={cn(
                                        "rounded border px-1.5 py-0.5 font-sans text-[8px] font-medium",
                                        operationToneBadgeClass(operation.tone)
                                      )}
                                    >
                                      {translateWithFallback(
                                        t,
                                        operation.labelKey,
                                        operation.fallbackLabel
                                      )}
                                    </span>
                                    <span
                                      className={cn(
                                        "rounded border px-1.5 py-0.5 font-sans text-[8px]",
                                        operationToneBadgeClass(operation.tone)
                                      )}
                                    >
                                      {translateWithFallback(
                                        t,
                                        operation.statusKey,
                                        operation.fallbackStatus
                                      )}
                                    </span>
                                    {group.event.taskId && (
                                      <span className="rounded border border-current/15 bg-muted/10 px-1 font-mono text-[8px] opacity-70">
                                        {group.event.taskId}
                                      </span>
                                    )}
                                  </span>
                                  <span
                                    className={cn(
                                      "mt-1 block font-mono text-[9px] whitespace-pre-wrap break-words",
                                      group.event.level === "error"
                                        ? "text-red-200"
                                        : group.event.level === "warning"
                                          ? "text-amber-200"
                                          : ""
                                    )}
                                  >
                                    {group.event.message}
                                  </span>
                                </span>
                              </button>
                              {logExpanded && hasDetails && (
                                <div className="mt-2 space-y-1 border-t border-current/10 pt-2">
                                  <div className="text-[9px] font-medium opacity-75">
                                    {translateWithFallback(
                                      t,
                                      "targetWorkspace.organizationRecon.logOutput",
                                      "实时输出"
                                    )}
                                  </div>
                                  <div className="max-h-44 space-y-1 overflow-auto">
                                    {hiddenDetailCount > 0 && (
                                      <div className="rounded border border-current/15 bg-muted/10 px-2 py-1 text-[9px] text-muted-foreground">
                                        {translateWithFallback(
                                          t,
                                          "targetWorkspace.organizationRecon.collapsedOutput",
                                          "已折叠早期输出"
                                        )}
                                        : {hiddenDetailCount}
                                      </div>
                                    )}
                                    {visibleDetails.map((detail) => (
                                      <ReconDetailLine
                                        key={detail.id}
                                        detail={detail}
                                        group={group}
                                        t={t}
                                      />
                                    ))}
                                  </div>
                                </div>
                              )}
                            </div>
                          );
                        })
                      ) : (
                        <p className="text-[9px] text-muted-foreground/70">
                          {translateWithFallback(
                            t,
                            "targetWorkspace.organizationRecon.noStageLogs",
                            "暂无阶段日志"
                          )}
                        </p>
                      )}
                      {stageTasks.some((task) => task.errors.length > 0) && (
                        <div className="space-y-1">
                          <div className="text-[9px] font-medium opacity-75">
                            {translateWithFallback(
                              t,
                              "targetWorkspace.organizationRecon.taskErrors",
                              "错误"
                            )}
                          </div>
                          {stageTasks.flatMap((task) =>
                            task.errors.map((error, index) => (
                              <div
                                key={`${task.taskId}:error:${index}`}
                                className="rounded border border-red-500/25 bg-red-500/5 px-2 py-1 font-mono text-[9px] whitespace-pre-wrap break-words text-red-200"
                              >
                                {error.code}: {error.message}
                              </div>
                            ))
                          )}
                        </div>
                      )}
                    </div>
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

function organizationReconStageLabel(t: (key: string) => string, stage: string): string {
  return translateWithFallback(t, `targetWorkspace.organizationRecon.stages.${stage}`, stage);
}

function organizationReconStatusLabel(t: (key: string) => string, status: string): string {
  return translateWithFallback(
    t,
    `targetWorkspace.organizationRecon.status.${displayOrganizationReconStatus(status)}`,
    displayOrganizationReconStatus(status)
  );
}

function ReconDetailLine({
  detail,
  group,
  t,
}: {
  detail: OrganizationReconTraceEvent;
  group: ReturnType<typeof organizationReconLogGroups>[number];
  t: (key: string) => string;
}) {
  const operation = organizationReconLogDetailOperationDisplay(detail, group);
  return (
    <div
      className={cn(
        "rounded border px-2 py-1 font-mono text-[9px] whitespace-pre-wrap break-words",
        operationToneDetailClass(operation.tone)
      )}
    >
      <div className="mb-1 flex flex-wrap items-center gap-1 font-sans">
        <span
          className={cn(
            "rounded border px-1.5 py-0.5 text-[8px] font-medium",
            operationToneBadgeClass(operation.tone)
          )}
        >
          {translateWithFallback(t, operation.labelKey, operation.fallbackLabel)}
        </span>
        <span
          className={cn(
            "rounded border px-1.5 py-0.5 text-[8px]",
            operationToneBadgeClass(operation.tone)
          )}
        >
          {translateWithFallback(t, operation.statusKey, operation.fallbackStatus)}
        </span>
      </div>
      {detail.message}
    </div>
  );
}

function operationToneBorderClass(tone: string): string {
  switch (tone) {
    case "running":
      return "border-blue-500/30";
    case "completed":
      return "border-emerald-500/30";
    case "warning":
    case "empty":
      return "border-amber-500/30";
    case "error":
      return "border-red-500/30";
    default:
      return "border-current/15";
  }
}

function operationToneBadgeClass(tone: string): string {
  switch (tone) {
    case "running":
      return "border-blue-500/30 bg-blue-500/10 text-blue-200";
    case "completed":
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-200";
    case "warning":
    case "empty":
      return "border-amber-500/30 bg-amber-500/10 text-amber-200";
    case "error":
      return "border-red-500/30 bg-red-500/10 text-red-200";
    default:
      return "border-current/15 bg-muted/10 text-muted-foreground";
  }
}

function operationToneDetailClass(tone: string): string {
  switch (tone) {
    case "running":
      return "border-blue-500/25 bg-blue-500/5 text-blue-100";
    case "completed":
      return "border-emerald-500/25 bg-emerald-500/5 text-emerald-100";
    case "warning":
    case "empty":
      return "border-amber-500/25 bg-amber-500/5 text-amber-100";
    case "error":
      return "border-red-500/25 bg-red-500/5 text-red-100";
    default:
      return "border-current/15 bg-muted/10";
  }
}
