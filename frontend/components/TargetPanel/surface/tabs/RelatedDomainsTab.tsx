import { ChevronRight, Globe } from "lucide-react";
import { cn } from "@/lib/utils";
import { EmptyInline, Section } from "../SurfaceParts";
import type { RelatedDomainVM, WebOriginVM } from "../surfaceHierarchy";

export function RelatedDomainsTab({
  relatedDomains,
  webOrigins,
  loading,
  onSelectDomain,
  onSelectOrigin,
}: {
  relatedDomains: RelatedDomainVM[];
  webOrigins: WebOriginVM[];
  loading: boolean;
  onSelectDomain?: (id: string) => void;
  onSelectOrigin: (id: string) => void;
}) {
  const originsById = new Map(webOrigins.map((origin) => [origin.id, origin]));

  return (
    <div className="space-y-2.5">
      <Section title="Related Domains" subtitle="domain/url targets grouped under the selected IP">
        {relatedDomains.length === 0 ? (
          <EmptyInline
            loading={loading}
            label="No related domain targets resolve to this IP yet."
          />
        ) : (
          <div className="space-y-1">
            {relatedDomains.map((domain) => (
              <div
                key={domain.id}
                className="rounded border border-border/20 bg-background/25 px-2 py-1.5"
              >
                <div className="flex min-w-0 items-center gap-2">
                  <Globe className="h-3.5 w-3.5 flex-shrink-0 text-blue-400/75" />
                  <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                    {domain.value}
                  </span>
                  <span
                    className={cn(
                      "flex-shrink-0 rounded px-1 py-0.5 text-[9px] font-semibold uppercase leading-none",
                      domain.scope === "in"
                        ? "bg-green-500/10 text-green-300"
                        : "bg-red-500/10 text-red-300"
                    )}
                  >
                    {domain.scope}
                  </span>
                  {onSelectDomain && (
                    <button
                      type="button"
                      onClick={() => onSelectDomain(domain.id)}
                      className="inline-flex h-5 items-center gap-0.5 rounded border border-border/25 bg-background/25 px-1.5 text-[9px] text-muted-foreground hover:border-accent/30 hover:text-foreground"
                    >
                      Open target
                      <ChevronRight className="h-3 w-3" />
                    </button>
                  )}
                </div>
                <div className="mt-1.5 flex flex-wrap gap-1">
                  {domain.webOriginIds.length === 0 ? (
                    <span className="text-[10px] text-muted-foreground">
                      No Web Origin with this host was inferred.
                    </span>
                  ) : (
                    domain.webOriginIds.map((originId) => {
                      const origin = originsById.get(originId);
                      if (!origin) return null;
                      return (
                        <button
                          key={originId}
                          type="button"
                          onClick={() => onSelectOrigin(originId)}
                          className="rounded border border-border/25 bg-muted/10 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground hover:border-accent/30 hover:text-foreground"
                        >
                          {origin.origin}
                        </button>
                      );
                    })
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </Section>
    </div>
  );
}
