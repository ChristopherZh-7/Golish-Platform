import { ArrowRight, CheckCircle2, CircleDotDashed, Waves } from "lucide-react";
import type { InvestigationSummaryView } from "@/lib/api/investigation";
import { InvestigationAuditDrawer } from "./InvestigationAuditDrawer";
import { WorkspaceAsyncState } from "./WorkspaceAsyncState";
import type { ProjectionResource } from "./useInvestigationProjection";

export function WavesTab({
  resource,
  onRetry,
}: {
  resource: ProjectionResource<InvestigationSummaryView>;
  onRetry: () => void;
}) {
  if (!resource.data) {
    return (
      <WorkspaceAsyncState resource={resource} label="wave summaries" empty={false} onRetry={onRetry} />
    );
  }

  const summary = resource.data;
  return (
    <div className="h-full overflow-y-auto p-4">
      <div className="mx-auto max-w-4xl space-y-4">
        <header className="flex items-center gap-2">
          <Waves className="h-4 w-4 text-cyan-300" />
          <h3 className="text-sm font-semibold">Generation and Wave closure</h3>
        </header>

        <div className="flex flex-wrap items-center gap-2 rounded border border-border/30 bg-muted/10 p-4 text-xs">
          <span className="rounded border border-border/30 px-2 py-1">H(active generation)</span>
          <ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="rounded border border-border/30 px-2 py-1">W(verification)</span>
          <ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="rounded border border-border/30 px-2 py-1">D(consolidation)</span>
          <ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="rounded border border-border/30 px-2 py-1">H(next generation)</span>
        </div>

        <div className="grid gap-3 sm:grid-cols-2">
          <section className="rounded border border-border/30 p-3">
            <div className="flex items-center gap-2 text-xs font-medium">
              {summary.activeGenerationId ? (
                <CircleDotDashed className="h-3.5 w-3.5 text-cyan-300" />
              ) : (
                <CheckCircle2 className="h-3.5 w-3.5 text-emerald-300" />
              )}
              Generation state
            </div>
            <p className="mt-2 text-[11px] text-muted-foreground">
              {summary.activeGenerationId
                ? "An active generation remains under server-authoritative verification."
                : "No active generation is present; consult the fixed-point and history receipts."}
            </p>
          </section>
          <section className="rounded border border-border/30 p-3">
            <div className="text-xs font-medium">Open members</div>
            <p className="mt-2 text-2xl font-semibold tabular-nums">
              {summary.openObligations.length}
            </p>
            <p className="text-[10px] text-muted-foreground">
              Server-projected obligations; zero is not independently promoted to a fixed point by
              the UI.
            </p>
          </section>
        </div>

        <dl className="grid grid-cols-2 gap-2 text-[10px] sm:grid-cols-5">
          {[
            ["Planned", summary.coverageDenominator.planned],
            ["Tested complete", summary.coverageDenominator.testedComplete],
            ["Tested degraded", summary.coverageDenominator.testedDegraded],
            ["Untested", summary.coverageDenominator.untested],
            ["Blocked", summary.coverageDenominator.blocked],
          ].map(([label, value]) => (
            <div key={label} className="rounded border border-border/25 bg-muted/10 p-2">
              <dt className="text-muted-foreground">{label}</dt>
              <dd className="mt-1 text-base font-semibold tabular-nums">{value}</dd>
            </div>
          ))}
        </dl>

        <div className="flex flex-wrap gap-2 text-[10px]">
          <span className="rounded border border-border/30 px-1.5 py-0.5">
            Generations {summary.generations.length}
          </span>
          <span className="rounded border border-border/30 px-1.5 py-0.5">
            Waves {summary.waves.length}
          </span>
          <span className="rounded border border-border/30 px-1.5 py-0.5">
            Control {summary.controlDecision}
          </span>
          <span className="rounded border border-amber-400/30 px-1.5 py-0.5 text-amber-200">
            Coverage {summary.coverageGrade} · {summary.coverageSufficiency}
          </span>
          <span className="rounded border border-border/30 px-1.5 py-0.5">
            All-fresh authority members {summary.authorityTimeMembers.length}
          </span>
        </div>

        <InvestigationAuditDrawer
          title="Generation authority audit"
          fields={[
            { label: "Active generation id", value: summary.activeGenerationId },
            { label: "Generation seal", value: summary.activeGenerationSealHash },
            { label: "Projection change", value: summary.envelope.changeSeq },
            { label: "Observed as of", value: resource.stamp?.observedAsOf },
            { label: "Authority epoch", value: resource.stamp?.authorityEpochSetHash },
            { label: "Effective valid until", value: summary.envelope.temporalSnapshot.earliestEffectiveValidUntil },
          ]}
        />
      </div>
    </div>
  );
}
