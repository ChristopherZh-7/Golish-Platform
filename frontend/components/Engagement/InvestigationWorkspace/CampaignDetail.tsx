import { AlertTriangle, Loader2 } from "lucide-react";
import type { InvestigationCampaignDetailResponse } from "@/lib/api/investigation";
import { InvestigationAuditDrawer } from "./InvestigationAuditDrawer";
import type { ProjectionResource } from "./useInvestigationProjection";

export function CampaignDetail({
  resource,
  onRetry,
}: {
  resource: ProjectionResource<InvestigationCampaignDetailResponse>;
  onRetry?: () => void;
}) {
  if (resource.status === "loading" && !resource.data) {
    return (
      <div className="flex items-center gap-2 p-4 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading Campaign detail…
      </div>
    );
  }
  if (!resource.data) {
    return (
      <div className="space-y-2 p-4 text-xs text-muted-foreground">
        {resource.errorMessage ? (
          <div role="alert" className="flex gap-2 text-red-300">
            <AlertTriangle className="h-3.5 w-3.5" /> {resource.errorMessage}
          </div>
        ) : (
          <p>Select a Campaign to inspect its rounds, actions, oracles and residual lineage.</p>
        )}
        {resource.errorMessage && onRetry && (
          <button type="button" className="text-accent hover:underline" onClick={onRetry}>
            Retry detail
          </button>
        )}
      </div>
    );
  }

  const campaign = resource.data.campaign;
  return (
    <article className="space-y-3 p-4">
      <header className="flex flex-wrap items-start gap-2">
        <div>
          <p className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Wave {campaign.waveOrdinal} · Campaign {campaign.campaignOrdinal}
          </p>
          <h3 className="mt-1 text-sm font-semibold">Verification Campaign</h3>
        </div>
        <div className="ml-auto flex gap-1.5 text-[10px]">
          <span className="rounded border border-border/30 px-1.5 py-0.5">{campaign.state}</span>
          <span className="rounded border border-border/30 px-1.5 py-0.5">
            {campaign.coverageStatus}
          </span>
        </div>
      </header>

      <section className="rounded border border-border/25 bg-muted/10 p-2.5">
        <h4 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Team topology
        </h4>
        <p className="mt-1 text-[11px] text-muted-foreground">not recorded</p>
      </section>

      <section>
        <h4 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Round strategy timeline
        </h4>
        {campaign.redactedRoundSummaries.length === 0 ? (
          <p className="mt-1 rounded border border-border/25 p-3 text-[11px] text-muted-foreground">
            No durable redacted round summary was recorded.
          </p>
        ) : (
          <ol className="mt-1 space-y-1 text-[11px]">
            {campaign.redactedRoundSummaries.map((summary, index) => (
              <li
                key={`${index}:${summary}`}
                className="rounded border border-border/25 bg-muted/10 p-2"
              >
                <span className="mr-2 text-muted-foreground">Round {index + 1}</span>
                {summary}
              </li>
            ))}
          </ol>
        )}
      </section>

      <dl className="grid grid-cols-3 gap-2 text-[10px]">
        <div className="rounded border border-border/25 p-2">
          <dt className="text-muted-foreground">Authorized actions</dt>
          <dd className="mt-1 text-base font-semibold tabular-nums">
            {campaign.authorizedActionCount}
          </dd>
        </div>
        <div className="rounded border border-border/25 p-2">
          <dt className="text-muted-foreground">Blocked actions</dt>
          <dd className="mt-1 text-base font-semibold tabular-nums">
            {campaign.blockedActionCount}
          </dd>
        </div>
        <div className="rounded border border-border/25 p-2">
          <dt className="text-muted-foreground">Open residuals</dt>
          <dd className="mt-1 text-base font-semibold tabular-nums">
            {campaign.openResidualIds.length}
          </dd>
        </div>
      </dl>

      <InvestigationAuditDrawer
        fields={[
          { label: "Campaign id", value: campaign.campaignId },
          { label: "Hypothesis revision", value: campaign.hypothesisRevisionId },
          { label: "Round ids", value: campaign.roundIds.join(", ") || null },
          { label: "Prepared action ids", value: campaign.preparedActionIds.join(", ") || null },
          { label: "Residual ids", value: campaign.openResidualIds.join(", ") || null },
          { label: "Observed as of", value: campaign.authorityTime.observedAsOf },
          { label: "Effective valid until", value: campaign.authorityTime.effectiveValidUntil },
          { label: "Authority epoch", value: campaign.authorityTime.authorityEpochHash },
          { label: "Temporal status", value: campaign.authorityTime.temporalStatus },
        ]}
      />
    </article>
  );
}
