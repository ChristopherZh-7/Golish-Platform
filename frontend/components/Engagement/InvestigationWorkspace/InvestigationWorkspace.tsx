import { Database, RefreshCw, X } from "lucide-react";
import { CampaignsTab } from "./CampaignsTab";
import { HypothesesTab } from "./HypothesesTab";
import { InvestigationStaleBanner } from "./InvestigationStaleBanner";
import { InvestigationTimelineTab } from "./InvestigationTimelineTab";
import { LegacyInvestigationAdapter } from "./LegacyInvestigationAdapter";
import { WavesTab } from "./WavesTab";
import {
  type InvestigationWorkspaceApi,
  type ProjectionResource,
  useInvestigationProjection,
} from "./useInvestigationProjection";

/** Legacy Plan D view-local selector. It is no longer a store/Pane route. */
type InvestigationWorkspaceTab = "hypotheses" | "campaigns" | "waves" | "timeline";

interface InvestigationWorkspaceSelection {
  operationId: string;
  defaultTab: InvestigationWorkspaceTab;
  selectedHypothesisId?: string;
  selectedCampaignId?: string;
  refreshSeq: number;
}

const TABS: Array<{ id: InvestigationWorkspaceTab; label: string }> = [
  { id: "hypotheses", label: "Hypotheses" },
  { id: "campaigns", label: "Campaigns" },
  { id: "waves", label: "Waves" },
  { id: "timeline", label: "Timeline" },
];

export interface InvestigationWorkspaceProps {
  sessionId: string;
  selection: InvestigationWorkspaceSelection;
  api?: InvestigationWorkspaceApi;
  onSelectTab?: (tab: InvestigationWorkspaceTab) => void;
  onSelectHypothesis?: (revisionId: string) => void;
  onSelectCampaign?: (campaignId: string) => void;
  onClose?: () => void;
}

function InvestigationWorkspaceProjection({
  sessionId,
  selection,
  api,
  onSelectTab,
  onSelectHypothesis,
  onSelectCampaign,
  onClose,
}: InvestigationWorkspaceProps) {
  const projection = useInvestigationProjection({
    sessionId,
    operationId: selection.operationId,
    refreshSeq: selection.refreshSeq,
    selectedHypothesisId: selection.selectedHypothesisId ?? null,
    selectedCampaignId: selection.selectedCampaignId ?? null,
    api,
  });
  const summary = projection.summary.data;

  return (
    <section className="flex h-full min-h-0 w-full flex-col bg-card" data-testid="investigation-workspace">
      <header className="flex flex-wrap items-center gap-2 border-b border-border/25 px-3 py-2">
        <Database className="h-4 w-4 text-cyan-300" />
        <div className="min-w-0">
          <h2 className="text-xs font-semibold">Investigation Workspace</h2>
          <p className="truncate text-[10px] text-muted-foreground">
            Server-authoritative projection · change {projection.summary.stamp?.changeSeq ?? "pending"}
          </p>
        </div>
        {summary && (
          <div className="ml-auto flex flex-wrap items-center gap-1.5 text-[9px]">
            <span className="rounded border border-border/30 px-1.5 py-0.5">
              {summary.envelope.investigationRolloutMode}
            </span>
            <span className="rounded border border-border/30 px-1.5 py-0.5">
              {summary.envelope.toolTruthContract}
            </span>
            <span className="rounded border border-border/30 px-1.5 py-0.5">
              authority until {summary.envelope.temporalSnapshot.earliestEffectiveValidUntil}
            </span>
          </div>
        )}
        <button
          type="button"
          aria-label="Refresh Investigation Workspace"
          className={summary ? "rounded p-1.5 text-muted-foreground hover:text-foreground" : "ml-auto rounded p-1.5 text-muted-foreground hover:text-foreground"}
          onClick={projection.refreshAll}
        >
          <RefreshCw className="h-3.5 w-3.5" />
        </button>
        {onClose && (
          <button
            type="button"
            aria-label="Close Investigation Workspace"
            className="rounded p-1.5 text-muted-foreground hover:text-foreground"
            onClick={onClose}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </header>

      <InvestigationStaleBanner
        resources={[
          projection.summary as ProjectionResource<unknown>,
          projection.hypotheses as ProjectionResource<unknown>,
          projection.campaigns as ProjectionResource<unknown>,
          projection.timeline as ProjectionResource<unknown>,
        ]}
        onReload={projection.refreshAll}
      />

      {summary && (
        <div className="border-b border-border/20 px-3 py-2">
          <LegacyInvestigationAdapter modePolicy={summary.envelope.modePolicy} />
        </div>
      )}

      <div role="tablist" aria-label="Investigation views" className="flex border-b border-border/25 px-2">
        {TABS.map((tab) => (
          <button
            key={tab.id}
            id={`investigation-tab-${tab.id}`}
            type="button"
            role="tab"
            aria-selected={selection.defaultTab === tab.id}
            aria-controls={`investigation-panel-${tab.id}`}
            className={`border-b-2 px-3 py-2 text-[11px] ${selection.defaultTab === tab.id ? "border-cyan-400 text-foreground" : "border-transparent text-muted-foreground"}`}
            onClick={() => onSelectTab?.(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div
        id={`investigation-panel-${selection.defaultTab}`}
        role="tabpanel"
        aria-labelledby={`investigation-tab-${selection.defaultTab}`}
        className="min-h-0 flex-1"
      >
        {selection.defaultTab === "hypotheses" && (
          <HypothesesTab
            resource={projection.hypotheses}
            detail={projection.hypothesisDetail}
            selectedRevisionId={selection.selectedHypothesisId ?? null}
            onSelect={(revisionId) => onSelectHypothesis?.(revisionId)}
            onLoadMore={projection.loadMoreHypotheses}
            onRetry={projection.refreshAll}
          />
        )}
        {selection.defaultTab === "campaigns" && (
          <CampaignsTab
            operationId={selection.operationId}
            refreshVersion={selection.refreshSeq}
            resource={projection.campaigns}
            detail={projection.campaignDetail}
            selectedCampaignId={selection.selectedCampaignId ?? null}
            onSelect={(campaignId) => onSelectCampaign?.(campaignId)}
            onLoadMore={projection.loadMoreCampaigns}
            onRetry={projection.refreshAll}
          />
        )}
        {selection.defaultTab === "waves" && (
          <WavesTab resource={projection.summary} onRetry={projection.refreshAll} />
        )}
        {selection.defaultTab === "timeline" && (
          <InvestigationTimelineTab
            resource={projection.timeline}
            onLoadMore={projection.loadMoreTimeline}
            onRetry={projection.refreshAll}
          />
        )}
      </div>
    </section>
  );
}

/** Remount all request coordinators when the trusted session/operation selector changes. */
export function InvestigationWorkspace(props: InvestigationWorkspaceProps) {
  return (
    <InvestigationWorkspaceProjection
      key={`${props.sessionId}:${props.selection.operationId}`}
      {...props}
    />
  );
}
