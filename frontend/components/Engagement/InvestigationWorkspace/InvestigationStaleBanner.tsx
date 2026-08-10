import { AlertTriangle, RefreshCw } from "lucide-react";
import type { ProjectionResource } from "./useInvestigationProjection";

export function InvestigationStaleBanner({
  resources,
  onReload,
}: {
  resources: Array<ProjectionResource<unknown>>;
  onReload: () => void;
}) {
  const stale = resources.find((resource) => resource.status === "stale");
  if (!stale) return null;

  return (
    <div
      role="status"
      className="flex flex-wrap items-center gap-2 border-b border-amber-400/25 bg-amber-400/[0.07] px-3 py-2 text-[11px] text-amber-200"
    >
      <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
      <span>
        The retained projection is stale. Server observation: {stale.stamp?.observedAsOf ?? "not recorded"}.
        A fresh first-page read is required.
      </span>
      <button
        type="button"
        className="ml-auto inline-flex items-center gap-1 rounded border border-amber-300/30 px-2 py-1"
        onClick={onReload}
      >
        <RefreshCw className="h-3 w-3" /> Reload
      </button>
    </div>
  );
}
