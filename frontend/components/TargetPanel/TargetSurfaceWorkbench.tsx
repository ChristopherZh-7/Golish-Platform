import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  Code2,
  Database,
  FileCode2,
  Globe,
  Loader2,
  Network,
  Radar,
  RefreshCw,
  Search,
  Server,
  Shield,
} from "lucide-react";
import { useMemo, useState } from "react";
import type { DirectoryEntry } from "@/lib/pentest/api";
import type { PortInfo, Target } from "@/lib/pentest/types";
import type {
  ApiEndpoint,
  Fingerprint,
  JsAnalysisResult,
  PassiveScanLog,
  TargetAsset,
  TimelineEntry,
} from "@/lib/security-analysis";
import { cn } from "@/lib/utils";
import { useTargetSurfaceData } from "./hooks/useTargetSurfaceData";

type SurfaceTab = "identity" | "surface" | "sitemap" | "js-api" | "sensitive" | "evidence";

const SURFACE_TABS: Array<{ id: SurfaceTab; label: string }> = [
  { id: "identity", label: "Identity" },
  { id: "surface", label: "Surface" },
  { id: "sitemap", label: "Sitemap" },
  { id: "js-api", label: "JS / API" },
  { id: "sensitive", label: "Sensitive" },
  { id: "evidence", label: "Evidence" },
];

export function TargetSurfaceWorkbench({
  target,
  onUpdateNotes,
}: {
  target: Target;
  onUpdateNotes: (id: string, notes: string) => void;
}) {
  const [activeTab, setActiveTab] = useState<SurfaceTab>("surface");
  const { data, loading, error, reload } = useTargetSurfaceData(target.id);

  const httpPorts = useMemo(() => target.ports.filter((port) => isHttpPort(port)), [target.ports]);
  const apiEndpoints = data.endpoints;
  const jsResults = data.jsResults;
  const sitemapItems = useMemo(
    () => buildSitemapItems(data.assets, data.directoryEntries),
    [data.assets, data.directoryEntries]
  );
  const sensitiveFindings = useMemo(
    () => buildSensitiveFindings(jsResults, data.passiveScans),
    [jsResults, data.passiveScans]
  );
  const sensitiveCount = useMemo(
    () => sensitiveFindings.reduce((count, item) => count + item.count, 0),
    [sensitiveFindings]
  );
  const tabCounts: Partial<Record<SurfaceTab, number>> = {
    surface: target.ports.length + data.fingerprints.length,
    sitemap: sitemapItems.length,
    "js-api": apiEndpoints.length + jsResults.length,
    sensitive: sensitiveCount,
    evidence: data.timeline.length || data.logs.length,
  };
  const lastEvidenceLabel = useMemo(
    () => formatLatestEvidence(data.timeline[0]?.createdAt, data.logs[0]?.createdAt),
    [data.timeline, data.logs]
  );

  return (
    <div className="h-full min-h-0 flex flex-col bg-background/20">
      <header className="border-b border-border/25 px-3 py-2">
        <div className="flex items-start justify-between gap-2.5">
          <div className="min-w-0">
            <div className="flex items-center gap-2 min-w-0">
              <Globe className="w-3.5 h-3.5 text-accent flex-shrink-0" />
              <h3 className="truncate text-[13px] font-semibold text-foreground">{target.value}</h3>
              <span
                className={cn(
                  "rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase leading-none",
                  target.scope === "in"
                    ? "bg-green-500/10 text-green-300"
                    : "bg-red-500/10 text-red-300"
                )}
              >
                {target.scope} scope
              </span>
            </div>
            <div className="mt-0.5 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
              <span>{target.type}</span>
              <span>{target.source || "manual"}</span>
              {target.real_ip && <span className="font-mono">{target.real_ip}</span>}
              {target.cdn_waf && <span>{target.cdn_waf}</span>}
              <span>
                {loading
                  ? "refreshing surface data"
                  : lastEvidenceLabel
                    ? `latest evidence ${lastEvidenceLabel}`
                    : "surface data from local evidence"}
              </span>
            </div>
          </div>
          <div className="flex flex-wrap justify-end gap-1">
            <StageButton icon={<Radar className="w-3 h-3" />} label="Run baseline recon" />
            <StageButton icon={<FileCode2 className="w-3 h-3" />} label="Collect JS" muted />
            <StageButton icon={<Search className="w-3 h-3" />} label="Match vulns" muted />
            <button
              type="button"
              className="inline-flex h-6 items-center gap-1 rounded border border-border/30 bg-background/20 px-1.5 text-[10px] text-muted-foreground hover:bg-muted/25 hover:text-foreground"
              onClick={() => void reload()}
              title="Refresh local target surface data"
            >
              {loading ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : (
                <RefreshCw className="w-3 h-3" />
              )}
            </button>
          </div>
        </div>
        {error && (
          <div className="mt-2 rounded border border-red-500/25 bg-red-500/5 px-2 py-1.5 text-[10px] text-red-300">
            {error}
          </div>
        )}
      </header>

      <nav className="flex items-center gap-0.5 border-b border-border/25 px-3 py-1.5">
        {SURFACE_TABS.map((tab) => (
          <button
            key={tab.id}
            type="button"
            className={cn(
              "inline-flex items-center gap-1 rounded px-2 py-0.5 text-[10px] transition-colors",
              activeTab === tab.id
                ? "bg-muted/30 text-foreground"
                : "text-muted-foreground hover:bg-muted/20 hover:text-foreground"
            )}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.label}
            {tabCounts[tab.id] ? (
              <span
                className={cn(
                  "rounded px-1 py-0.5 text-[8px] tabular-nums",
                  activeTab === tab.id
                    ? "bg-background/40 text-foreground"
                    : "bg-muted/25 text-muted-foreground"
                )}
              >
                {tabCounts[tab.id]}
              </span>
            ) : null}
          </button>
        ))}
      </nav>

      <div className="min-h-0 flex-1 overflow-y-auto p-2.5">
        {activeTab === "identity" && <IdentityTab target={target} onUpdateNotes={onUpdateNotes} />}
        {activeTab === "surface" && (
          <SurfaceTabView
            target={target}
            httpPorts={httpPorts}
            endpointCount={apiEndpoints.length}
            jsCount={jsResults.length}
            fingerprints={data.fingerprints}
            loading={loading}
          />
        )}
        {activeTab === "sitemap" && <SitemapTab items={sitemapItems} loading={loading} />}
        {activeTab === "js-api" && (
          <JsApiTab endpoints={apiEndpoints} jsResults={jsResults} loading={loading} />
        )}
        {activeTab === "sensitive" && (
          <SensitiveTab
            findings={sensitiveFindings}
            sensitiveCount={sensitiveCount}
            loading={loading}
          />
        )}
        {activeTab === "evidence" && (
          <EvidenceTab
            target={target}
            timeline={data.timeline}
            logs={data.logs}
            loading={loading}
          />
        )}
      </div>
    </div>
  );
}

function StageButton({
  icon,
  label,
  muted = false,
}: {
  icon: React.ReactNode;
  label: string;
  muted?: boolean;
}) {
  return (
    <button
      type="button"
      className={cn(
        "inline-flex h-6 items-center gap-1 rounded border px-1.5 text-[10px] transition-colors",
        muted
          ? "border-border/30 bg-background/20 text-muted-foreground hover:bg-muted/25 hover:text-foreground"
          : "border-accent/25 bg-accent/10 text-accent hover:bg-accent/15"
      )}
    >
      {icon}
      {label}
    </button>
  );
}

function IdentityTab({
  target,
  onUpdateNotes,
}: {
  target: Target;
  onUpdateNotes: (id: string, notes: string) => void;
}) {
  return (
    <div className="space-y-2.5">
      <Section title="Identity" subtitle="scope, source, and ownership signals">
        <div className="grid grid-cols-2 gap-1.5">
          <Kv label="Target" value={target.value} mono />
          <Kv label="Type" value={target.type} />
          <Kv label="Source" value={target.source || "manual"} />
          <Kv label="Scope" value={target.scope} />
          <Kv label="Resolved IP" value={target.real_ip || "—"} mono />
          <Kv label="CDN / WAF" value={target.cdn_waf || "—"} />
          <Kv label="OS" value={target.os_info || "—"} />
          <Kv label="Owner" value={target.owner || "—"} />
        </div>
      </Section>
      <Section title="Notes" subtitle="target-level operator context">
        <textarea
          className="min-h-24 w-full resize-y rounded border border-border/35 bg-background/40 px-2 py-1.5 text-[11px] outline-none focus:border-accent"
          placeholder="Target notes"
          defaultValue={target.notes}
          onBlur={(event) => {
            if (event.target.value !== target.notes) {
              onUpdateNotes(target.id, event.target.value);
            }
          }}
        />
      </Section>
    </div>
  );
}

function SurfaceTabView({
  target,
  httpPorts,
  endpointCount,
  jsCount,
  fingerprints,
  loading,
}: {
  target: Target;
  httpPorts: PortInfo[];
  endpointCount: number;
  jsCount: number;
  fingerprints: Fingerprint[];
  loading: boolean;
}) {
  return (
    <div className="grid items-start grid-cols-[minmax(0,1.1fr)_minmax(260px,0.9fr)] gap-2.5">
      <Section title="Services" subtitle={`${target.ports.length} ports · ${httpPorts.length} web`}>
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
            <Metric icon={<Globe className="w-3.5 h-3.5" />} label="Web" value={httpPorts.length} />
            <Metric icon={<Code2 className="w-3.5 h-3.5" />} label="API" value={endpointCount} />
            <Metric icon={<FileCode2 className="w-3.5 h-3.5" />} label="JS" value={jsCount} />
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
  );
}

function SitemapTab({ items, loading }: { items: SitemapItem[]; loading: boolean }) {
  if (items.length === 0) {
    return (
      <EmptyPanel
        loading={loading}
        icon={<Network className="w-5 h-5" />}
        title="No sitemap data yet"
        body="Run baseline recon or a crawler-backed collection step to populate paths, robots, and sitemap entries."
      />
    );
  }

  return (
    <Section title="Sitemap / Paths" subtitle={`${items.length} discovered`}>
      <div className="space-y-1">
        {items.slice(0, 80).map((item) => (
          <div
            key={`${item.source}:${item.url}`}
            className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
          >
            <div className="flex items-center gap-2 text-[11px]">
              <span className="rounded bg-muted/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                {item.source}
              </span>
              {item.statusCode != null && (
                <span className="font-mono text-[10px] text-green-300">{item.statusCode}</span>
              )}
              <span className="min-w-0 flex-1 truncate font-mono text-foreground/80">
                {item.url}
              </span>
              {item.contentType && (
                <span className="text-[9px] text-muted-foreground">{item.contentType}</span>
              )}
            </div>
          </div>
        ))}
      </div>
    </Section>
  );
}

function JsApiTab({
  endpoints,
  jsResults,
  loading,
}: {
  endpoints: ApiEndpoint[];
  jsResults: JsAnalysisResult[];
  loading: boolean;
}) {
  if (endpoints.length === 0 && jsResults.length === 0) {
    return (
      <EmptyPanel
        loading={loading}
        icon={<FileCode2 className="w-5 h-5" />}
        title="No JS or API evidence yet"
        body="Collect JavaScript to extract API endpoints, source maps, routes, and front-end security signals."
      />
    );
  }
  return (
    <div className="grid items-start grid-cols-2 gap-2.5">
      <Section title="API Endpoints" subtitle={`${endpoints.length} extracted`}>
        <div className="space-y-1">
          {endpoints.slice(0, 20).map((endpoint) => (
            <div
              key={endpoint.id}
              className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
            >
              <div className="flex items-center gap-2">
                <span className="w-10 text-right font-mono text-[10px] text-blue-300">
                  {endpoint.method}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/80">
                  {endpoint.path || endpoint.url}
                </span>
                <span className="rounded bg-muted/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                  {endpoint.riskLevel || "info"}
                </span>
              </div>
            </div>
          ))}
        </div>
      </Section>
      <Section title="JavaScript Files" subtitle={`${jsResults.length} analyzed`}>
        <div className="space-y-1">
          {jsResults.slice(0, 20).map((result) => (
            <div key={result.id} className="rounded border border-border/20 bg-muted/5 px-2 py-1.5">
              <div className="flex items-center gap-2">
                <FileCode2 className="w-3.5 h-3.5 text-yellow-300" />
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/80">
                  {result.filename || result.url}
                </span>
                {result.sourceMaps && (
                  <span className="rounded bg-yellow-500/10 px-1.5 py-0.5 text-[9px] text-yellow-300">
                    map
                  </span>
                )}
              </div>
            </div>
          ))}
        </div>
      </Section>
    </div>
  );
}

function SensitiveTab({
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

function EvidenceTab({
  target,
  timeline,
  logs,
  loading,
}: {
  target: Target;
  timeline: TimelineEntry[];
  logs: Array<{
    id: number;
    action: string;
    status: string;
    toolName: string | null;
    createdAt: number;
  }>;
  loading: boolean;
}) {
  return (
    <div className="space-y-2.5">
      <Section title="Source Evidence" subtitle="why this target exists">
        <div className="rounded border border-border/20 bg-muted/5 px-2 py-1.5 text-[11px]">
          <div className="flex items-center gap-2">
            <CheckCircle2 className="w-3.5 h-3.5 text-green-300" />
            <span className="text-foreground">{target.source || "manual"}</span>
            <span className="text-muted-foreground">· scope={target.scope}</span>
          </div>
        </div>
      </Section>
      <Section title="Operation Logs" subtitle={`${logs.length} latest event(s)`}>
        {timeline.length === 0 && logs.length === 0 ? (
          <EmptyInline loading={loading} label="No operation logs for this target yet" />
        ) : (
          <div className="space-y-1">
            {timeline.slice(0, 30).map((entry, index) => (
              <div
                key={`${entry.source}:${entry.event}:${entry.createdAt}:${index}`}
                className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
              >
                <div className="flex items-center gap-2 text-[11px]">
                  <Activity className="w-3.5 h-3.5 text-accent/70" />
                  <span className="rounded bg-muted/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                    {entry.source}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-foreground/80">
                    {entry.details || entry.event}
                  </span>
                  {entry.toolName && (
                    <span className="rounded bg-accent/10 px-1.5 py-0.5 text-[9px] text-accent">
                      {entry.toolName}
                    </span>
                  )}
                  <span className="text-[9px] text-muted-foreground">
                    {formatTime(entry.createdAt)}
                  </span>
                </div>
              </div>
            ))}
            {timeline.length === 0 &&
              logs.map((log) => (
                <div
                  key={log.id}
                  className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
                >
                  <div className="flex items-center gap-2 text-[11px]">
                    <Database className="w-3.5 h-3.5 text-accent/70" />
                    <span className="min-w-0 flex-1 truncate text-foreground/80">{log.action}</span>
                    {log.toolName && (
                      <span className="rounded bg-accent/10 px-1.5 py-0.5 text-[9px] text-accent">
                        {log.toolName}
                      </span>
                    )}
                    <span className="text-[9px] text-muted-foreground">
                      {new Date(log.createdAt).toLocaleTimeString()}
                    </span>
                  </div>
                </div>
              ))}
          </div>
        )}
      </Section>
    </div>
  );
}

function Section({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded border border-border/25 bg-background/15">
      <div className="flex h-8 items-center justify-between border-b border-border/20 px-2.5">
        <h4 className="text-[11px] font-medium text-foreground">{title}</h4>
        {subtitle && <span className="text-[9px] text-muted-foreground">{subtitle}</span>}
      </div>
      <div className="p-2.5">{children}</div>
    </section>
  );
}

function Kv({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="rounded border border-border/20 bg-muted/5 p-2">
      <p className="text-[10px] text-muted-foreground">{label}</p>
      <p className={cn("mt-1 truncate text-[11px] text-foreground", mono && "font-mono")}>
        {value}
      </p>
    </div>
  );
}

function Metric({ icon, label, value }: { icon: React.ReactNode; label: string; value: number }) {
  return (
    <div className="rounded border border-border/20 bg-muted/5 p-2">
      <div className="flex items-center gap-1.5 text-muted-foreground">
        {icon}
        <span className="text-[10px]">{label}</span>
      </div>
      <p className="mt-0.5 text-base font-semibold tabular-nums text-foreground">{value}</p>
    </div>
  );
}

function EmptyInline({ label, loading }: { label: string; loading?: boolean }) {
  return (
    <div className="rounded border border-dashed border-border/25 bg-background/10 p-3 text-center text-[11px] text-muted-foreground">
      {loading ? (
        <span className="inline-flex items-center gap-2">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Loading target surface data
        </span>
      ) : (
        label
      )}
    </div>
  );
}

function EmptyPanel({
  icon,
  title,
  body,
  loading,
}: {
  icon: React.ReactNode;
  title: string;
  body: string;
  loading?: boolean;
}) {
  return (
    <div className="flex min-h-[180px] items-center justify-center rounded border border-dashed border-border/25 bg-background/10 p-6 text-center">
      <div className="max-w-sm">
        <div className="mx-auto mb-2.5 flex h-8 w-8 items-center justify-center rounded bg-muted/15 text-muted-foreground">
          {loading ? <Loader2 className="w-5 h-5 animate-spin" /> : icon}
        </div>
        <h4 className="text-xs font-medium text-foreground">{title}</h4>
        <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">{body}</p>
      </div>
    </div>
  );
}

function isHttpPort(port: PortInfo): boolean {
  const service = port.service?.toLowerCase() ?? "";
  return service.includes("http") || port.http_status != null || Boolean(port.http_title);
}

interface SitemapItem {
  url: string;
  source: string;
  statusCode: number | null;
  contentType: string;
}

function buildSitemapItems(
  assets: TargetAsset[],
  directoryEntries: DirectoryEntry[]
): SitemapItem[] {
  const seen = new Set<string>();
  const out: SitemapItem[] = [];
  for (const entry of directoryEntries) {
    if (!entry.url || seen.has(entry.url)) continue;
    seen.add(entry.url);
    out.push({
      url: entry.url,
      source: entry.tool || "directory",
      statusCode: entry.status_code,
      contentType: entry.content_type,
    });
  }
  for (const asset of assets) {
    if (!asset.value || seen.has(asset.value)) continue;
    const type = asset.assetType.toLowerCase();
    if (!type.includes("path") && !type.includes("url") && !type.includes("sitemap")) continue;
    seen.add(asset.value);
    out.push({
      url: asset.value,
      source: asset.assetType,
      statusCode: null,
      contentType: String(asset.metadata?.content_type ?? ""),
    });
  }
  return out;
}

interface SensitiveFinding {
  source: string;
  label: string;
  url: string;
  count: number;
}

function buildSensitiveFindings(
  jsResults: JsAnalysisResult[],
  passiveScans: PassiveScanLog[]
): SensitiveFinding[] {
  const findings: SensitiveFinding[] = [];
  for (const result of jsResults) {
    const secrets = Array.isArray(result.secretsFound) ? result.secretsFound : [];
    if (secrets.length > 0) {
      findings.push({
        source: "js",
        label: result.filename || result.url,
        url: result.url,
        count: secrets.length,
      });
    }
    if (result.sourceMaps) {
      findings.push({
        source: "sourcemap",
        label: result.filename || result.url,
        url: result.url,
        count: 1,
      });
    }
  }
  for (const scan of passiveScans) {
    if (!["vulnerable", "potential"].includes(scan.result)) continue;
    findings.push({
      source: scan.toolUsed || "passive",
      label: scan.testType || scan.evidence || scan.result,
      url: scan.url,
      count: 1,
    });
  }
  return findings;
}

function formatLatestEvidence(timelineCreatedAt?: string, logCreatedAt?: number): string | null {
  if (timelineCreatedAt) return formatTime(timelineCreatedAt);
  if (logCreatedAt) return formatTime(logCreatedAt);
  return null;
}

function formatTime(value: string | number): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
