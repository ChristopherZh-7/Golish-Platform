import { AlertTriangle, Shield } from "lucide-react";
import { EmptyPanel, Section } from "../SurfaceParts";
import type { SensitiveFinding } from "../types";

export function SensitiveTab({
  findings,
  sensitiveCount,
  loading,
}: {
  findings: SensitiveFinding[];
  sensitiveCount: number;
  loading: boolean;
}) {
  if (sensitiveCount === 0) {
    return (
      <EmptyPanel
        loading={loading}
        icon={<Shield className="w-5 h-5" />}
        title="No sensitive candidates yet"
        body="Sensitive exposure checks will summarize secrets, source maps, leaked keys, and confirmed findings here."
      />
    );
  }
  return (
    <Section title="Sensitive Candidates" subtitle={`${sensitiveCount} candidate signal(s)`}>
      <div className="space-y-1">
        {findings.map((finding) => (
          <div
            key={`${finding.source}:${finding.url}:${finding.label}`}
            className="rounded border border-red-500/20 bg-red-500/5 px-2 py-1.5"
          >
            <div className="flex items-center gap-2">
              <AlertTriangle className="w-3.5 h-3.5 text-red-300" />
              <span className="rounded bg-red-500/10 px-1.5 py-0.5 text-[9px] text-red-300">
                {finding.source}
              </span>
              <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/80">
                {finding.label}
              </span>
              <span className="rounded bg-red-500/10 px-1.5 py-0.5 text-[9px] text-red-300">
                {finding.count}
              </span>
            </div>
            {finding.url && (
              <p className="mt-1 truncate font-mono text-[10px] text-muted-foreground">
                {finding.url}
              </p>
            )}
          </div>
        ))}
      </div>
    </Section>
  );
}
