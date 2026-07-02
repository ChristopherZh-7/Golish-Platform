import { Network, ShieldCheck } from "lucide-react";
import { cn } from "@/lib/utils";
import { EmptyInline, Section } from "../SurfaceParts";
import type { NetworkEndpointVM, WebOriginVM } from "../surfaceHierarchy";

const INFERRED_CONFIDENCE_TITLE =
  "Inferred from URL / target metadata. Backend does not yet provide web_origin_id.";

export function NetworkEndpointsTab({
  endpoints,
  webOrigins,
  loading,
  selectedOriginId,
  onSelectOrigin,
}: {
  endpoints: NetworkEndpointVM[];
  webOrigins: WebOriginVM[];
  loading: boolean;
  selectedOriginId: string | null;
  onSelectOrigin: (id: string) => void;
}) {
  const originsById = new Map(webOrigins.map((origin) => [origin.id, origin]));

  return (
    <div className="space-y-2.5">
      <Section
        title="Network Endpoints"
        subtitle={`${endpoints.length} IP:port endpoint(s), derived from current frontend data`}
      >
        {endpoints.length === 0 ? (
          <EmptyInline loading={loading} label="No endpoint evidence yet" />
        ) : (
          <div className="overflow-x-auto rounded border border-border/25">
            <table className="min-w-[920px] w-full text-[11px]">
              <thead className="border-b border-border/25 bg-muted/10 text-muted-foreground">
                <tr>
                  <th className="px-2 py-1.5 text-left font-medium">IP</th>
                  <th className="px-2 py-1.5 text-left font-medium">Port</th>
                  <th className="px-2 py-1.5 text-left font-medium">Transport</th>
                  <th className="px-2 py-1.5 text-left font-medium">State</th>
                  <th className="px-2 py-1.5 text-left font-medium">Service</th>
                  <th className="px-2 py-1.5 text-left font-medium">TLS</th>
                  <th className="px-2 py-1.5 text-left font-medium">Web Origins</th>
                  <th className="px-2 py-1.5 text-left font-medium">Confidence</th>
                  <th className="px-2 py-1.5 text-left font-medium">Source</th>
                </tr>
              </thead>
              <tbody>
                {endpoints.map((endpoint) => (
                  <tr key={endpoint.id} className="border-b border-border/15 last:border-0">
                    <td className="px-2 py-2">
                      <div className="flex items-center gap-1.5">
                        <Network className="h-3.5 w-3.5 text-accent/75" />
                        <span className="font-mono text-accent">{endpoint.ip || "unknown"}</span>
                      </div>
                    </td>
                    <td className="px-2 py-2 font-mono text-muted-foreground">{endpoint.port}</td>
                    <td className="px-2 py-2 font-mono text-muted-foreground">
                      {endpoint.transport}
                    </td>
                    <td className="px-2 py-2 text-muted-foreground">
                      {endpoint.state || "unknown"}
                    </td>
                    <td className="px-2 py-2 text-foreground/75">
                      {endpoint.service || "unknown"}
                    </td>
                    <td className="px-2 py-2">
                      {endpoint.tls ? (
                        <ShieldCheck className="h-3.5 w-3.5 text-green-300" aria-label="TLS" />
                      ) : (
                        <span className="text-muted-foreground">-</span>
                      )}
                    </td>
                    <td className="px-2 py-2">
                      {endpoint.webOriginIds.length === 0 ? (
                        <span className="text-muted-foreground">No reliable origin link</span>
                      ) : (
                        <div className="flex flex-wrap items-center gap-1">
                          <span className="rounded bg-muted/25 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
                            {endpoint.webOriginIds.length}
                          </span>
                          {endpoint.webOriginIds.map((originId) => {
                            const origin = originsById.get(originId);
                            if (!origin) return null;
                            return (
                              <button
                                key={originId}
                                type="button"
                                onClick={() => onSelectOrigin(originId)}
                                className={cn(
                                  "rounded border px-1.5 py-0.5 font-mono text-[9px] transition-colors",
                                  selectedOriginId === originId
                                    ? "border-accent/40 bg-accent/10 text-accent"
                                    : "border-border/25 bg-background/25 text-muted-foreground hover:border-accent/30 hover:text-foreground"
                                )}
                              >
                                {origin.origin}
                              </button>
                            );
                          })}
                        </div>
                      )}
                    </td>
                    <td className="px-2 py-2">
                      <span
                        className={cn(
                          "rounded px-1.5 py-0.5 text-[9px]",
                          endpoint.confidence === "confirmed"
                            ? "bg-green-500/10 text-green-300"
                            : "bg-yellow-500/10 text-yellow-300"
                        )}
                        title={
                          endpoint.confidence === "inferred" ? INFERRED_CONFIDENCE_TITLE : undefined
                        }
                      >
                        {endpoint.confidence}
                      </span>
                    </td>
                    <td className="px-2 py-2 text-muted-foreground">{endpoint.source}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>
    </div>
  );
}
