import { ScanSearch } from "lucide-react";
import type { Fingerprint } from "@/lib/security-analysis";
import { EmptyInline, Section } from "../SurfaceParts";

function confidencePercent(confidence: number): number {
  if (!Number.isFinite(confidence)) return 0;
  const normalized = confidence >= 0 && confidence <= 1 ? confidence * 100 : confidence;
  return Math.round(Math.min(100, Math.max(0, normalized)));
}

export function FingerprintList({
  fingerprints,
  loading = false,
  limit = 50,
  title = "Fingerprints",
  emptyLabel = "No service/version fingerprint yet (nmap -sV / whatweb / httpx)",
}: {
  fingerprints: Fingerprint[];
  loading?: boolean;
  limit?: number;
  title?: string;
  emptyLabel?: string;
}) {
  return (
    <Section title={title} subtitle={`${fingerprints.length} detected`}>
      {fingerprints.length === 0 ? (
        <EmptyInline loading={loading} label={emptyLabel} />
      ) : (
        <div className="space-y-1.5">
          {fingerprints.slice(0, limit).map((fp) => (
            <div key={fp.id} className="rounded border border-border/20 bg-muted/5 px-2 py-1.5">
              <div className="flex items-center gap-2 text-[11px]">
                <ScanSearch className="h-3.5 w-3.5 flex-shrink-0 text-purple-300/80" />
                <span className="rounded bg-purple-500/10 px-1.5 py-0.5 text-[9px] text-purple-300">
                  {fp.category}
                </span>
                <span className="min-w-0 flex-1 truncate text-foreground/85">{fp.name}</span>
                {fp.version && (
                  <span className="font-mono text-[10px] text-muted-foreground">{fp.version}</span>
                )}
                <span className="text-[9px] text-muted-foreground">
                  {confidencePercent(fp.confidence)}%
                </span>
              </div>
              {fp.cpe && (
                <p
                  className="mt-1 truncate font-mono text-[9px] text-muted-foreground"
                  title={fp.cpe}
                >
                  {fp.cpe}
                </p>
              )}
            </div>
          ))}
          {fingerprints.length > limit && (
            <p className="text-[9px] text-muted-foreground">+{fingerprints.length - limit} more</p>
          )}
        </div>
      )}
    </Section>
  );
}
