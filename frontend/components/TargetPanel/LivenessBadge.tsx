/**
 * `LivenessBadge` — renders a target's EAS-stamped liveness verdict
 * (design 2026-07-02-dead-asset-liveness-state Phase 4).
 *
 * `alive` (green) / `dead` (red) / `unreachable` (yellow); a null/absent state
 * (not probed yet) renders nothing so the row stays quiet until EAS has an
 * opinion. `dead` assets are the ones downstream stages drop from the coverage
 * denominator, so surfacing the verdict here explains why an asset is skipped.
 */

import { cn } from "@/lib/utils";

const LIVENESS_CONFIG: Record<string, { label: string; className: string }> = {
  alive: { label: "存活", className: "bg-green-500/10 text-green-300" },
  dead: { label: "死亡", className: "bg-red-500/10 text-red-300" },
  unreachable: { label: "不可達", className: "bg-yellow-500/10 text-yellow-300" },
};

export function LivenessBadge({ state }: { state?: string | null }) {
  const cfg = state ? LIVENESS_CONFIG[state] : undefined;
  if (!cfg) return null;
  return (
    <span className={cn("rounded px-1.5 py-0.5 text-[9px] font-medium", cfg.className)}>
      {cfg.label}
    </span>
  );
}
