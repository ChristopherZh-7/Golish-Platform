import { Braces, ChevronRight, Code2, FileCode2, Globe, ScanSearch, Server } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { PortInfo, Target } from "@/lib/pentest/types";
import type { Fingerprint } from "@/lib/security-analysis";
import { cn } from "@/lib/utils";
import { EmptyInline, Metric, Section } from "../SurfaceParts";

export function SurfaceTabView({
  target,
  httpPorts,
  endpointCount,
  endpointParamCount,
  jsCount,
  fingerprints,
  loading,
  relatedDomains,
  onSelectDomain,
}: {
  target: Target;
  httpPorts: PortInfo[];
  endpointCount: number;
  endpointParamCount: number;
  jsCount: number;
  fingerprints: Fingerprint[];
  loading: boolean;
  // When the subject is an IP/host, the domains that resolve to it. The rows are
  // only clickable when the parent passes an explicit drill-in handler.
  relatedDomains?: Target[];
  onSelectDomain?: (id: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2.5">
      <div className="grid items-start grid-cols-[minmax(0,1.1fr)_minmax(260px,0.9fr)] gap-2.5">
        <Section
          title="Services"
          subtitle={`${target.ports.length} ports · ${httpPorts.length} web`}
        >
          {target.ports.length === 0 ? (
            <EmptyInline loading={loading} label="No service evidence yet" />
          ) : (
            <div className="overflow-hidden rounded border border-border/25">
              <table className="w-full text-[11px]">
                <thead className="border-b border-border/25 bg-muted/10 text-muted-foreground">
                  <tr>
                    <th className="px-2 py-1.5 text-left font-medium">Port</th>
                    <th className="px-2 py-1.5 text-left font-medium">Service</th>
                    <th className="px-2 py-1.5 text-left font-medium">Web</th>
                    <th className="px-2 py-1.5 text-left font-medium">Signals</th>
                  </tr>
                </thead>
                <tbody>
                  {target.ports.map((port) => (
                    <tr
                      key={`${port.port}:${port.protocol}`}
                      className="border-b border-border/15 last:border-0"
                    >
                      <td className="px-2 py-2 font-mono text-accent">
                        {port.port}/{port.protocol || "tcp"}
                      </td>
                      <td className="px-2 py-2 text-foreground/75">
                        {[port.service, port.webserver].filter(Boolean).join(" · ") || "unknown"}
                      </td>
                      <td className="px-2 py-2 text-muted-foreground">
                        {port.http_status != null ? (
                          <>
                            <span className="font-mono text-green-300">{port.http_status}</span>{" "}
                            {port.http_title || port.url || "HTTP"}
                          </>
                        ) : (
                          "—"
                        )}
                      </td>
                      <td className="px-2 py-2">
                        <div className="flex flex-wrap gap-1">
                          {(port.technologies || []).slice(0, 3).map((tech) => (
                            <span
                              key={`${port.port}:${tech}`}
                              className="rounded bg-blue-500/10 px-1.5 py-0.5 text-[9px] text-blue-300"
                            >
                              {tech}
                            </span>
                          ))}
                          {port.state && port.state !== "open" && (
                            <span className="rounded bg-yellow-500/10 px-1.5 py-0.5 text-[9px] text-yellow-300">
                              {port.state}
                            </span>
                          )}
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Section>
        <div className="space-y-2.5">
          <Section title="Surface Summary" subtitle="current local evidence">
            <div className="grid grid-cols-2 gap-1.5">
              <Metric
                icon={<Server className="w-3.5 h-3.5" />}
                label="Ports"
                value={target.ports.length}
              />
              <Metric
                icon={<Globe className="w-3.5 h-3.5" />}
                label="Web"
                value={httpPorts.length}
              />
              <Metric icon={<Code2 className="w-3.5 h-3.5" />} label="API" value={endpointCount} />
              <Metric
                icon={<Braces className="w-3.5 h-3.5" />}
                label="Params"
                value={endpointParamCount}
              />
              <Metric icon={<FileCode2 className="w-3.5 h-3.5" />} label="JS" value={jsCount} />
              <Metric
                icon={<ScanSearch className="w-3.5 h-3.5" />}
                label="Fingerprints"
                value={fingerprints.length}
              />
            </div>
          </Section>
          <Section title="Fingerprints" subtitle={`${fingerprints.length} detected`}>
            {fingerprints.length === 0 ? (
              <EmptyInline
                loading={loading}
                label="Fingerprint details appear after baseline recon"
              />
            ) : (
              <div className="space-y-1.5">
                {fingerprints.slice(0, 10).map((fingerprint) => (
                  <div
                    key={fingerprint.id}
                    className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
                  >
                    <div className="flex items-center gap-2 text-[11px]">
                      <span className="rounded bg-purple-500/10 px-1.5 py-0.5 text-[9px] text-purple-300">
                        {fingerprint.category}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-foreground/85">
                        {fingerprint.name}
                      </span>
                      {fingerprint.version && (
                        <span className="font-mono text-[10px] text-muted-foreground">
                          {fingerprint.version}
                        </span>
                      )}
                      <span className="text-[9px] text-muted-foreground">
                        {Math.round(fingerprint.confidence)}%
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </Section>
        </div>
      </div>

      {relatedDomains && (
        <Section
          title={t("targets.surfaceDomainsTitle")}
          subtitle={t("targets.surfaceDomainsHint")}
        >
          {relatedDomains.length === 0 ? (
            <EmptyInline loading={false} label={t("targets.surfaceDomainsEmpty")} />
          ) : (
            <div className="space-y-1">
              {relatedDomains.map((domain) => {
                const content = (
                  <>
                    <div className="flex items-center gap-2">
                      <Globe className="h-3.5 w-3.5 flex-shrink-0 text-blue-400/70" />
                      <span
                        className={cn(
                          "min-w-0 flex-1 truncate text-[12px] text-foreground",
                          onSelectDomain && "group-hover:text-accent"
                        )}
                      >
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
                        <ChevronRight className="h-3.5 w-3.5 flex-shrink-0 text-muted-foreground/40 group-hover:text-muted-foreground" />
                      )}
                    </div>
                  </>
                );

                return onSelectDomain ? (
                  <button
                    key={domain.id}
                    type="button"
                    onClick={() => onSelectDomain(domain.id)}
                    className="group flex w-full flex-col gap-1 rounded border border-border/20 bg-background/30 px-2 py-1.5 text-left transition-colors hover:border-accent/40 hover:bg-muted/15"
                  >
                    {content}
                  </button>
                ) : (
                  <div
                    key={domain.id}
                    className="flex w-full flex-col gap-1 rounded border border-border/20 bg-background/30 px-2 py-1.5 text-left"
                  >
                    {content}
                  </div>
                );
              })}
            </div>
          )}
        </Section>
      )}
    </div>
  );
}
