import { Radar } from "lucide-react";
import { getReconIntelSummary } from "@/lib/tools";
import { cn } from "@/lib/utils";

/**
 * Inline chip line that surfaces which passive-intel providers ran for a
 * `recon_enrich_assets` / `recon_discover_subsidiaries` tool call — in
 * particular an explicit OSINT badge when an ENScan provider ran. This makes
 * OSINT execution visible directly on the tool row (it otherwise only lands in
 * backend.log via the `recon_enrich_assets` provider path and never shows as a
 * visible tool call). Renders nothing for any other tool or a result without a
 * provider list.
 */
export function ReconIntelSummaryLine({
  name,
  result,
  className,
}: {
  name: string;
  result: unknown;
  className?: string;
}) {
  const summary = getReconIntelSummary(name, result);
  if (!summary) return null;

  const counts: string[] = [];
  if (summary.targets > 0) counts.push(`${summary.targets} assets`);
  if (summary.organizations > 0) counts.push(`${summary.organizations} orgs`);
  if (summary.promotedChildren > 0) counts.push(`${summary.promotedChildren} subsidiaries`);

  return (
    <div className={cn("flex flex-wrap items-center gap-1 text-[10px]", className)}>
      {summary.osint && (
        <span className="inline-flex items-center gap-0.5 rounded bg-[var(--ansi-magenta)]/15 px-1 py-0.5 font-medium text-[var(--ansi-magenta)]">
          <Radar className="h-2.5 w-2.5" />
          OSINT
        </span>
      )}
      {summary.providers.map((provider) => (
        <span
          key={provider}
          className="rounded bg-muted/40 px-1 py-0.5 font-mono text-muted-foreground/80"
          title={provider}
        >
          {provider}
        </span>
      ))}
      {counts.length > 0 && <span className="text-muted-foreground/60">{counts.join(" · ")}</span>}
    </div>
  );
}
