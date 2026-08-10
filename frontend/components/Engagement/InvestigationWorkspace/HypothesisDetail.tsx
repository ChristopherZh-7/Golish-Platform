import { AlertTriangle, Loader2 } from "lucide-react";
import type { InvestigationHypothesisDetailView } from "@/lib/api/investigation";
import { InvestigationAuditDrawer } from "./InvestigationAuditDrawer";
import { LegacyFieldView } from "./LegacyInvestigationAdapter";
import type { ProjectionResource } from "./useInvestigationProjection";

export function HypothesisDetail({
  resource,
  onRetry,
}: {
  resource: ProjectionResource<InvestigationHypothesisDetailView>;
  onRetry?: () => void;
}) {
  if (resource.status === "loading" && !resource.data) {
    return (
      <div className="flex items-center gap-2 p-4 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading hypothesis detail…
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
          <p>Select a hypothesis to inspect its claims, gaps and lineage.</p>
        )}
        {resource.errorMessage && onRetry && (
          <button type="button" className="text-accent hover:underline" onClick={onRetry}>
            Retry detail
          </button>
        )}
      </div>
    );
  }

  const detail = resource.data;
  const hypothesis = detail.hypothesis;
  return (
    <article className="space-y-3 p-4">
      <header>
        <p className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Hypothesis
        </p>
        <h3 className="mt-1 text-sm font-semibold text-foreground/90">{hypothesis.predicateSummary}</h3>
        <p className="mt-1 text-[11px] text-muted-foreground">
          At-time subject · {hypothesis.targetTypeAtTime} · {hypothesis.targetValueAtTime}
        </p>
      </header>

      <div className="flex flex-wrap gap-1.5 text-[10px]">
        {[hypothesis.epistemicState, hypothesis.lifecycleState, hypothesis.planningReadiness].map(
          (value) => (
            <span key={value} className="rounded border border-border/30 px-1.5 py-0.5">
              {value}
            </span>
          )
        )}
      </div>

      <section>
        <h4 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Verification objectives
        </h4>
        {detail.verificationObjectiveSummaries.length === 0 ? (
          <p className="mt-1 text-[11px] text-muted-foreground">No objective has been recorded.</p>
        ) : (
          <ul className="mt-1 space-y-1 text-[11px]">
            {detail.verificationObjectiveSummaries.map((objective) => (
              <li key={objective} className="rounded bg-muted/15 px-2 py-1.5">
                {objective}
              </li>
            ))}
          </ul>
        )}
      </section>

      <dl className="grid grid-cols-2 gap-2 text-[10px] sm:grid-cols-4">
        {[
          ["Support", detail.supportRefIds.length],
          ["Contradictions", detail.contradictionRefIds.length],
          ["Context", detail.applicationContextRefIds.length],
          ["Gaps", detail.gapRefIds.length],
        ].map(([label, count]) => (
          <div key={label} className="rounded border border-border/25 bg-muted/10 p-2">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="mt-1 text-base font-semibold tabular-nums">{count}</dd>
          </div>
        ))}
      </dl>

      <div className="flex flex-wrap items-center gap-2 text-[10px]">
        <span className="text-muted-foreground">Legacy projection</span>
        <LegacyFieldView
          field={
            hypothesis.legacyProjectionStatus
              ? { kind: "available", value: hypothesis.legacyProjectionStatus }
              : { kind: "legacy_unavailable" }
          }
          render={(value) => <span className="rounded border border-border/30 px-1.5 py-0.5">{value}</span>}
        />
      </div>

      <InvestigationAuditDrawer
        fields={[
          { label: "Root id", value: hypothesis.rootId },
          { label: "Revision id", value: hypothesis.revisionId },
          { label: "Predecessor revision", value: detail.predecessorRevisionId },
          { label: "Lineage revisions", value: detail.lineageRevisionIds.join(", ") || null },
          { label: "Support refs", value: detail.supportRefIds.join(", ") || null },
          { label: "Conflict refs", value: detail.contradictionRefIds.join(", ") || null },
          { label: "Context refs", value: detail.applicationContextRefIds.join(", ") || null },
          { label: "Gap refs", value: detail.gapRefIds.join(", ") || null },
          { label: "Observed as of", value: resource.stamp?.observedAsOf },
          { label: "Authority epoch", value: resource.stamp?.authorityEpochSetHash },
        ]}
      />
    </article>
  );
}
