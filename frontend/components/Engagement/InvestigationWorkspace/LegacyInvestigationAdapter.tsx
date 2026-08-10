import type { ReactNode } from "react";
import type { InvestigationModePolicyView } from "@/lib/generated/InvestigationModePolicyView";

export type LegacyField<T> = { kind: "available"; value: T } | { kind: "legacy_unavailable" };

export function LegacyFieldView<T>({
  field,
  render,
}: {
  field: LegacyField<T>;
  render: (value: T) => ReactNode;
}) {
  return field.kind === "legacy_unavailable" ? (
    <span
      data-state="legacy-unavailable"
      className="rounded border border-amber-400/30 bg-amber-400/[0.06] px-1.5 py-0.5 text-[10px] text-amber-200"
    >
      legacy_unavailable
    </span>
  ) : (
    render(field.value)
  );
}

/** Renders server-frozen policy. This component never recreates the mode matrix. */
export function LegacyInvestigationAdapter({
  modePolicy,
  children,
}: {
  modePolicy: InvestigationModePolicyView;
  children?: ReactNode;
}) {
  return (
    <section className="space-y-2 rounded border border-border/30 bg-muted/10 p-3">
      <div className="flex flex-wrap items-center gap-2 text-[10px]">
        <span className="rounded border border-border/30 px-1.5 py-0.5">
          Writer · {modePolicy.canonicalWriter}
        </span>
        <span className="rounded border border-border/30 px-1.5 py-0.5">
          Compare · {modePolicy.comparePolicy}
        </span>
        <span className="rounded border border-border/30 px-1.5 py-0.5">
          Projection · {modePolicy.legacyProjectionPolicy}
        </span>
      </div>
      {modePolicy.allowLegacyMutation ? (
        children ?? <p className="text-[11px] text-muted-foreground">Legacy controls are available for this frozen operation.</p>
      ) : (
        <p className="text-[11px] text-muted-foreground">
          Legacy mutation is disabled by the server-frozen investigation contract.
        </p>
      )}
    </section>
  );
}
