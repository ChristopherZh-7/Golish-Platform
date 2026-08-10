import { Activity, Workflow } from "lucide-react";
import { type ReactNode, useId } from "react";
import { cn } from "@/lib/utils";

const STAGE_RUN_DETAIL_LABELS: Readonly<Record<string, string>> = {
  recon: "Recon",
  target_intel: "Recon",
  external_attack_surface: "Recon",
  enumeration: "Enumeration",
  application_understanding: "Application Understanding",
  vulnerability: "Vulnerability",
  vuln_triage: "Vulnerability",
  verification: "Verification",
  attack_candidate: "Candidate",
};

export interface StageRunDetailShellProps {
  stageKey: string;
  operationId?: string | null;
  statusLabel: string;
  children: ReactNode;
  sideRail?: ReactNode;
  metricSlots?: ReactNode;
}

function normalizeStageKey(stageKey: string): string {
  return stageKey
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, "_");
}

export function resolveStageRunDetailLabel(stageKey: string): string {
  const normalized = normalizeStageKey(stageKey);
  if (!normalized) return "Stage run";
  const knownLabel = STAGE_RUN_DETAIL_LABELS[normalized];
  if (knownLabel) return knownLabel;
  return normalized
    .split("_")
    .filter(Boolean)
    .map((word) => `${word.charAt(0).toUpperCase()}${word.slice(1)}`)
    .join(" ");
}

export function StageRunDetailShell({
  stageKey,
  operationId,
  statusLabel,
  children,
  sideRail,
  metricSlots,
}: StageRunDetailShellProps) {
  const headingId = useId();
  const stageLabel = resolveStageRunDetailLabel(stageKey);
  const normalizedOperationId = operationId?.trim() || null;

  return (
    <section
      aria-labelledby={headingId}
      data-stage-key={stageKey}
      data-testid="stage-run-detail-shell"
      className="grid h-full min-h-0 grid-rows-[auto_auto_minmax(0,1fr)] overflow-hidden rounded-xl border border-border/70 bg-card text-foreground shadow-lg shadow-black/10"
    >
      <header className="flex min-h-10 items-center gap-2.5 border-b border-border/50 bg-card px-3 py-1.5">
        <span className="grid h-6 w-6 shrink-0 place-items-center rounded-md bg-foreground text-background">
          <Workflow className="h-3.5 w-3.5" aria-hidden="true" />
        </span>
        <div className="flex min-w-0 flex-1 items-baseline gap-2">
          <h2 id={headingId} className="shrink-0 truncate text-sm font-medium">
            {stageLabel}
          </h2>
          {normalizedOperationId && (
            <code
              className="hidden truncate font-mono text-[9px] text-muted-foreground/55 sm:block"
              title={normalizedOperationId}
            >
              {normalizedOperationId}
            </code>
          )}
        </div>
        <span
          role="status"
          className="inline-flex max-w-full items-center gap-1 rounded-full border border-sky-400/25 bg-sky-400/10 px-2 py-0.5 text-[10px] text-sky-100"
        >
          <Activity className="h-3 w-3 shrink-0" aria-hidden="true" />
          <span className="truncate">{statusLabel}</span>
        </span>
      </header>

      {metricSlots !== undefined && metricSlots !== null && (
        <section
          aria-label="Stage metrics"
          data-testid="stage-run-detail-metrics"
          className="grid gap-1.5 border-b border-border/50 bg-card/70 p-2 sm:grid-cols-2 lg:grid-cols-4"
        >
          {metricSlots}
        </section>
      )}

      <div
        className={cn(
          "h-full min-h-0 overflow-hidden",
          sideRail !== undefined &&
            sideRail !== null &&
            "grid md:grid-cols-[clamp(12rem,20vw,16rem)_minmax(0,1fr)]"
        )}
      >
        {sideRail !== undefined && sideRail !== null && (
          <aside
            aria-label="Stage agents"
            data-testid="stage-run-detail-side-rail"
            className="min-h-0 min-w-0 overflow-hidden border-b border-border/50 bg-background/45 md:border-r md:border-b-0"
          >
            {sideRail}
          </aside>
        )}
        <div
          data-testid="stage-run-detail-body"
          className="h-full min-h-0 min-w-0 overflow-hidden bg-card/35"
        >
          {children}
        </div>
      </div>
    </section>
  );
}
