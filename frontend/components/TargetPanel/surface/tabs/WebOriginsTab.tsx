import { Braces, Code2, FileCode2, Globe, Link2, ShieldCheck } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { EmptyInline, Metric, Section } from "../SurfaceParts";
import type { NetworkEndpointVM, WebOriginContentRef, WebOriginVM } from "../surfaceHierarchy";
import { WhatWebAssessmentBadge } from "../WhatWebAssessmentBadge";
import type { WhatWebAssessment } from "../whatWebAssessment";
import { FingerprintList } from "./FingerprintList";
import { SitemapTab } from "./SitemapTab";

type OriginDetailTab =
  | "overview"
  | "sitemap"
  | "apis"
  | "js"
  | "params"
  | "fingerprints"
  | "evidence";

const INFERRED_CONFIDENCE_TITLE =
  "Inferred from URL / target metadata. Backend does not yet provide web_origin_id.";

const DETAIL_TABS: Array<{ id: OriginDetailTab; label: string }> = [
  { id: "overview", label: "Overview" },
  { id: "sitemap", label: "Sitemap" },
  { id: "apis", label: "APIs" },
  { id: "js", label: "JS" },
  { id: "params", label: "Params" },
  { id: "fingerprints", label: "Fingerprints" },
  { id: "evidence", label: "Evidence" },
];

const CONTENT_COUNT_SOURCE_LABEL: Record<WebOriginVM["contentCountSource"], string> = {
  backend_content_counts: "backend_content_counts",
  frontend_content_inferred: "frontend_content_inferred",
};

const IDENTITY_SOURCE_LABEL: Record<WebOriginVM["source"], string> = {
  backend_identity: "backend_identity",
  frontend_inferred: "frontend_inferred",
};

const BACKEND_CONTENT_NOT_LOADED_HINT =
  "Backend reports legacy content for this origin, but row-level content is not loaded in this view yet.";
const BACKEND_CONTENT_MORE_THAN_ROWS_HINT =
  "Backend reports legacy content for this origin, but row-level content is still loaded from the legacy frontend data sources.";
const BACKEND_REFS_LABEL =
  "From backend content index (lightweight refs — full request/response rows are not loaded in this view).";

function refsOfKind(origin: WebOriginVM, kinds: string[]): WebOriginContentRef[] {
  return origin.contentRefs.filter((ref) => kinds.includes(ref.kind));
}

function frontendRowTotal(origin: WebOriginVM): number {
  return (
    origin.urls.length +
    origin.apiEndpoints.length +
    origin.jsResources.length +
    origin.params.length +
    origin.directoryEntries.length +
    origin.evidence.length
  );
}

function backendContentTotal(origin: WebOriginVM): number {
  const { counts } = origin;
  return (
    counts.urls +
    counts.apis +
    counts.js +
    counts.params +
    counts.directoryEntries +
    counts.evidence
  );
}

/**
 * Which "backend reports more than we render" hint (if any) applies to a
 * backend-counted origin. Returns `null` for frontend-counted origins or when
 * the loaded rows already match the backend counts.
 */
function originContentHint(origin: WebOriginVM): string | null {
  if (origin.contentCountSource !== "backend_content_counts") return null;
  if (backendContentTotal(origin) <= 0) return null;
  if (frontendRowTotal(origin) === 0) {
    // When we have lightweight refs we render them instead of a dead-end hint.
    return origin.contentRefs.length > 0 ? null : BACKEND_CONTENT_NOT_LOADED_HINT;
  }
  if (backendContentTotal(origin) > frontendRowTotal(origin)) {
    return BACKEND_CONTENT_MORE_THAN_ROWS_HINT;
  }
  return null;
}

function BackendRefList({ refs, title }: { refs: WebOriginContentRef[]; title: string }) {
  return (
    <Section title={title} subtitle={`${refs.length} ref(s)`}>
      <p className="mb-1.5 text-[10px] text-muted-foreground">{BACKEND_REFS_LABEL}</p>
      <div className="overflow-hidden rounded border border-border/25">
        <table className="w-full text-[11px]">
          <thead className="border-b border-border/25 bg-muted/10 text-muted-foreground">
            <tr>
              <th className="px-2 py-1.5 text-left font-medium">Kind</th>
              <th className="px-2 py-1.5 text-left font-medium">Method</th>
              <th className="px-2 py-1.5 text-left font-medium">Status</th>
              <th className="px-2 py-1.5 text-left font-medium">URL</th>
              <th className="px-2 py-1.5 text-left font-medium">Source</th>
            </tr>
          </thead>
          <tbody>
            {refs.map((ref) => (
              <tr key={`${ref.kind}:${ref.id}`} className="border-b border-border/15 last:border-0">
                <td className="px-2 py-2 font-mono text-accent/80">{ref.kind}</td>
                <td className="px-2 py-2 font-mono text-blue-300">{ref.method ?? "-"}</td>
                <td className="px-2 py-2 font-mono text-muted-foreground">
                  {ref.statusCode ?? "-"}
                </td>
                <td className="min-w-0 px-2 py-2">
                  <span className="block truncate font-mono text-foreground/80" title={ref.url}>
                    {ref.url}
                  </span>
                </td>
                <td className="px-2 py-2 text-muted-foreground">{ref.source ?? "-"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Section>
  );
}

function endpointLabel(endpoint: NetworkEndpointVM): string {
  return `${endpoint.ip || "unknown"}:${endpoint.port}/${endpoint.transport}`;
}

function isNetworkEndpoint(value: NetworkEndpointVM | undefined): value is NetworkEndpointVM {
  return Boolean(value);
}

function CountCell({ value, title }: { value: number; title?: string }) {
  return (
    <span className="font-mono text-[10px] tabular-nums text-foreground/75" title={title}>
      {value}
    </span>
  );
}

function ConfidenceBadge({ value }: { value: "confirmed" | "inferred" }) {
  return (
    <span
      className={cn(
        "rounded px-1.5 py-0.5 text-[9px]",
        value === "confirmed"
          ? "bg-green-500/10 text-green-300"
          : "bg-yellow-500/10 text-yellow-300"
      )}
      title={value === "inferred" ? INFERRED_CONFIDENCE_TITLE : undefined}
    >
      {value}
    </span>
  );
}

function OriginOverview({
  origin,
  endpoints,
}: {
  origin: WebOriginVM;
  endpoints: NetworkEndpointVM[];
}) {
  return (
    <div className="space-y-2.5">
      <Section title="Origin Overview" subtitle={origin.origin}>
        <div className="grid grid-cols-2 gap-1.5 lg:grid-cols-4">
          <Metric
            icon={<Link2 className="h-3.5 w-3.5" />}
            label="URLs"
            value={origin.counts.urls}
          />
          <Metric
            icon={<Code2 className="h-3.5 w-3.5" />}
            label="APIs"
            value={origin.counts.apis}
          />
          <Metric
            icon={<FileCode2 className="h-3.5 w-3.5" />}
            label="JS"
            value={origin.counts.js}
          />
          <Metric
            icon={<Braces className="h-3.5 w-3.5" />}
            label="Params"
            value={origin.counts.params}
          />
        </div>
        <dl className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[10px] text-muted-foreground">
          <div className="flex items-center gap-1">
            <dt className="text-muted-foreground/70">Identity source:</dt>
            <dd className="font-mono text-foreground/80">{IDENTITY_SOURCE_LABEL[origin.source]}</dd>
          </div>
          <div className="flex items-center gap-1">
            <dt className="text-muted-foreground/70">Content count source:</dt>
            <dd className="font-mono text-foreground/80">
              {CONTENT_COUNT_SOURCE_LABEL[origin.contentCountSource]}
            </dd>
          </div>
          {origin.counts.directoryEntries > 0 && (
            <div className="flex items-center gap-1">
              <dt className="text-muted-foreground/70">Directory entries:</dt>
              <dd className="font-mono text-foreground/80">{origin.counts.directoryEntries}</dd>
            </div>
          )}
          {origin.counts.passiveLogs > 0 && (
            <div className="flex items-center gap-1">
              <dt className="text-muted-foreground/70">Passive logs:</dt>
              <dd className="font-mono text-foreground/80">{origin.counts.passiveLogs}</dd>
            </div>
          )}
        </dl>
      </Section>
      <Section title="Observed Endpoint" subtitle={`${origin.endpointIds.length} linked`}>
        {endpoints.length === 0 ? (
          <EmptyInline loading={false} label="No reliable IP:port relation was inferred." />
        ) : (
          <div className="flex flex-wrap gap-1.5">
            {endpoints.map((endpoint) => (
              <span
                key={endpoint.id}
                className="inline-flex items-center gap-1 rounded border border-border/25 bg-background/25 px-1.5 py-0.5 font-mono text-[10px] text-foreground/80"
              >
                {endpointLabel(endpoint)}
                <ConfidenceBadge value={endpoint.confidence} />
              </span>
            ))}
          </div>
        )}
      </Section>
    </div>
  );
}

function ApiList({ origin }: { origin: WebOriginVM }) {
  const apiRefs = origin.apiEndpoints.length === 0 ? refsOfKind(origin, ["api"]) : [];
  if (origin.apiEndpoints.length === 0 && apiRefs.length > 0) {
    return <BackendRefList refs={apiRefs} title="APIs" />;
  }
  return (
    <Section title="APIs" subtitle={`${origin.apiEndpoints.length} endpoint(s)`}>
      {origin.apiEndpoints.length === 0 ? (
        <EmptyInline loading={false} label="No API endpoints assigned to this origin." />
      ) : (
        <div className="overflow-hidden rounded border border-border/25">
          <table className="w-full text-[11px]">
            <thead className="border-b border-border/25 bg-muted/10 text-muted-foreground">
              <tr>
                <th className="px-2 py-1.5 text-left font-medium">Method</th>
                <th className="px-2 py-1.5 text-left font-medium">Path</th>
                <th className="px-2 py-1.5 text-left font-medium">Params</th>
                <th className="px-2 py-1.5 text-left font-medium">Source</th>
              </tr>
            </thead>
            <tbody>
              {origin.apiEndpoints.map((endpoint) => (
                <tr key={endpoint.id} className="border-b border-border/15 last:border-0">
                  <td className="px-2 py-2 font-mono text-blue-300">{endpoint.method || "GET"}</td>
                  <td className="min-w-0 px-2 py-2">
                    <span className="block truncate font-mono text-foreground/80">
                      {endpoint.path || endpoint.url}
                    </span>
                  </td>
                  <td className="px-2 py-2">
                    <CountCell
                      value={Array.isArray(endpoint.params) ? endpoint.params.length : 0}
                    />
                  </td>
                  <td className="px-2 py-2 text-muted-foreground">
                    {endpoint.source || "api_endpoint"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Section>
  );
}

function JsList({ origin }: { origin: WebOriginVM }) {
  const jsRefs = origin.jsResources.length === 0 ? refsOfKind(origin, ["js"]) : [];
  if (origin.jsResources.length === 0 && jsRefs.length > 0) {
    return <BackendRefList refs={jsRefs} title="JavaScript" />;
  }
  return (
    <Section title="JavaScript" subtitle={`${origin.jsResources.length} resource(s)`}>
      {origin.jsResources.length === 0 ? (
        <EmptyInline loading={false} label="No JS resources assigned to this origin." />
      ) : (
        <div className="space-y-1">
          {origin.jsResources.map((js) => (
            <div key={js.id} className="rounded border border-border/20 bg-muted/5 px-2 py-1.5">
              <div className="flex min-w-0 items-center gap-2 text-[11px]">
                <FileCode2 className="h-3.5 w-3.5 flex-shrink-0 text-[var(--ansi-yellow)]/85" />
                <span className="min-w-0 flex-1 truncate font-mono text-foreground/85">
                  {js.filename || js.url}
                </span>
                <span className="text-[9px] text-muted-foreground">
                  {js.endpointsFound.length} endpoints
                </span>
              </div>
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

function ParamList({ origin }: { origin: WebOriginVM }) {
  return (
    <Section title="Params" subtitle={`${origin.params.length} parameter(s)`}>
      {origin.params.length === 0 ? (
        <EmptyInline loading={false} label="No params assigned to this origin." />
      ) : (
        <div className="space-y-1">
          {origin.params.map((param) => (
            <div key={param.id} className="rounded border border-border/20 bg-muted/5 px-2 py-1.5">
              <div className="flex min-w-0 items-center gap-2 text-[11px]">
                <Braces className="h-3.5 w-3.5 flex-shrink-0 text-accent/75" />
                <span className="font-mono text-accent">{param.name}</span>
                <span className="rounded bg-muted/25 px-1.5 py-0.5 font-mono text-[9px] text-blue-300">
                  {param.method}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground">
                  {param.url}
                </span>
              </div>
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

function EvidenceList({ origin }: { origin: WebOriginVM }) {
  return (
    <Section title="Evidence" subtitle={`${origin.evidence.length} linked item(s)`}>
      {origin.evidence.length === 0 ? (
        <EmptyInline loading={false} label="No origin-linked evidence yet." />
      ) : (
        <div className="space-y-1">
          {origin.evidence.map((item) => (
            <div key={item.id} className="rounded border border-border/20 bg-muted/5 px-2 py-1.5">
              <div className="flex min-w-0 items-center gap-2 text-[11px]">
                <ShieldCheck className="h-3.5 w-3.5 flex-shrink-0 text-green-300/80" />
                <span className="rounded bg-muted/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                  {item.source}
                </span>
                <span className="min-w-0 flex-1 truncate text-foreground/80">{item.label}</span>
                <ConfidenceBadge value={item.confidence} />
              </div>
              {(item.url || item.capturePath) && (
                <p className="mt-1 truncate font-mono text-[9px] text-muted-foreground">
                  {item.url || item.capturePath}
                </p>
              )}
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

function OriginDetail({
  origin,
  endpoints,
  loading,
  projectPath,
}: {
  origin: WebOriginVM;
  endpoints: NetworkEndpointVM[];
  loading: boolean;
  projectPath: string | null;
}) {
  const [activeDetailTab, setActiveDetailTab] = useState<OriginDetailTab>("overview");
  const contentHint = originContentHint(origin);
  const sitemapRefs =
    origin.urls.length === 0 ? refsOfKind(origin, ["api", "js", "directory"]) : [];

  useEffect(() => {
    setActiveDetailTab("overview");
  }, [origin.id]);

  return (
    <section className="rounded border border-border/25 bg-background/15">
      <div className="flex min-h-8 items-center justify-between gap-2 border-b border-border/20 px-2.5 py-1.5">
        <div className="min-w-0">
          <h4 className="truncate font-mono text-[11px] font-medium text-foreground">
            {origin.origin}
          </h4>
          <p className="mt-0.5 text-[9px] text-muted-foreground">
            {origin.scheme} · {origin.hostType} · port {origin.port} · {origin.source}
          </p>
        </div>
        <ConfidenceBadge value={origin.confidence} />
      </div>
      <div className="border-b border-border/20 px-2.5 py-1.5">
        <div className="flex flex-wrap gap-1">
          {DETAIL_TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              onClick={() => setActiveDetailTab(tab.id)}
              className={cn(
                "rounded px-2 py-0.5 text-[10px] transition-colors",
                activeDetailTab === tab.id
                  ? "bg-muted/30 text-foreground"
                  : "text-muted-foreground hover:bg-muted/20 hover:text-foreground"
              )}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>
      {contentHint ? (
        <div className="border-b border-yellow-500/20 bg-yellow-500/5 px-2.5 py-1.5 text-[10px] text-yellow-100/85">
          {contentHint}
        </div>
      ) : (
        origin.backendIdentityOnly && (
          <div className="border-b border-yellow-500/20 bg-yellow-500/5 px-2.5 py-1.5 text-[10px] text-yellow-100/85">
            Backend identity exists, but legacy web content has not been linked to this origin yet.
          </div>
        )
      )}
      <div className="p-2.5">
        {activeDetailTab === "overview" && <OriginOverview origin={origin} endpoints={endpoints} />}
        {activeDetailTab === "sitemap" &&
          (origin.urls.length === 0 && sitemapRefs.length > 0 ? (
            <BackendRefList refs={sitemapRefs} title="Sitemap" />
          ) : (
            <div className="h-[460px] min-h-[360px]">
              <SitemapTab
                items={origin.urls}
                jsResults={origin.jsResources}
                loading={loading}
                projectPath={projectPath}
                originLabel={origin.origin}
              />
            </div>
          ))}
        {activeDetailTab === "apis" && <ApiList origin={origin} />}
        {activeDetailTab === "js" && <JsList origin={origin} />}
        {activeDetailTab === "params" && <ParamList origin={origin} />}
        {activeDetailTab === "fingerprints" && (
          <FingerprintList fingerprints={origin.fingerprints} loading={loading} />
        )}
        {activeDetailTab === "evidence" && <EvidenceList origin={origin} />}
      </div>
    </section>
  );
}

export function WebOriginsTab({
  webOrigins,
  endpoints,
  loading,
  selectedOriginId,
  onSelectOrigin,
  projectPath,
  assessmentByOrigin,
}: {
  webOrigins: WebOriginVM[];
  endpoints: NetworkEndpointVM[];
  loading: boolean;
  selectedOriginId: string | null;
  onSelectOrigin: (id: string) => void;
  projectPath: string | null;
  assessmentByOrigin: ReadonlyMap<string, WhatWebAssessment>;
}) {
  const endpointsById = useMemo(
    () => new Map(endpoints.map((endpoint) => [endpoint.id, endpoint])),
    [endpoints]
  );
  const selectedOrigin =
    webOrigins.find((origin) => origin.id === selectedOriginId) ?? webOrigins[0] ?? null;
  const selectedEndpoints = selectedOrigin
    ? selectedOrigin.endpointIds.map((id) => endpointsById.get(id)).filter(isNetworkEndpoint)
    : [];

  useEffect(() => {
    if (!selectedOrigin && webOrigins[0]) onSelectOrigin(webOrigins[0].id);
  }, [onSelectOrigin, selectedOrigin, webOrigins]);

  return (
    <div className="space-y-2.5">
      <Section
        title="Web Origins"
        subtitle={`${webOrigins.length} origin(s), grouped by scheme://host:port`}
      >
        {webOrigins.length === 0 ? (
          <EmptyInline
            loading={loading}
            label="No Web Origin can be inferred from complete URLs yet."
          />
        ) : (
          <div className="overflow-x-auto rounded border border-border/25">
            <table className="min-w-[1180px] w-full text-[11px]">
              <thead className="border-b border-border/25 bg-muted/10 text-muted-foreground">
                <tr>
                  <th className="px-2 py-1.5 text-left font-medium">Origin</th>
                  <th className="px-2 py-1.5 text-left font-medium">WhatWeb</th>
                  <th className="px-2 py-1.5 text-left font-medium">Scheme</th>
                  <th className="px-2 py-1.5 text-left font-medium">Host</th>
                  <th className="px-2 py-1.5 text-left font-medium">Host Type</th>
                  <th className="px-2 py-1.5 text-left font-medium">Port</th>
                  <th className="px-2 py-1.5 text-left font-medium">Endpoints</th>
                  <th className="px-2 py-1.5 text-left font-medium">URL</th>
                  <th className="px-2 py-1.5 text-left font-medium">API</th>
                  <th className="px-2 py-1.5 text-left font-medium">JS</th>
                  <th className="px-2 py-1.5 text-left font-medium">Params</th>
                  <th className="px-2 py-1.5 text-left font-medium">FP</th>
                  <th className="px-2 py-1.5 text-left font-medium">Evidence</th>
                  <th className="px-2 py-1.5 text-left font-medium">Confidence</th>
                  <th className="px-2 py-1.5 text-left font-medium">Source</th>
                </tr>
              </thead>
              <tbody>
                {webOrigins.map((origin) => {
                  const selected = origin.id === selectedOrigin?.id;
                  const observed = origin.endpointIds
                    .map((id) => endpointsById.get(id))
                    .filter(isNetworkEndpoint)
                    .map((endpoint) => endpointLabel(endpoint));
                  return (
                    <tr
                      key={origin.id}
                      className={cn(
                        "cursor-pointer border-b border-border/15 last:border-0 hover:bg-muted/15",
                        selected && "bg-accent/10"
                      )}
                      onClick={() => onSelectOrigin(origin.id)}
                    >
                      <td className="min-w-0 px-2 py-2">
                        <div className="flex min-w-0 items-center gap-1.5">
                          <Globe className="h-3.5 w-3.5 flex-shrink-0 text-accent/75" />
                          <span className="block truncate font-mono text-foreground/85">
                            {origin.origin}
                          </span>
                        </div>
                      </td>
                      <td className="px-2 py-2">
                        <WhatWebAssessmentBadge assessment={assessmentByOrigin.get(origin.id)} />
                      </td>
                      <td className="px-2 py-2 text-muted-foreground">{origin.scheme}</td>
                      <td className="max-w-[180px] px-2 py-2">
                        <span className="block truncate font-mono text-muted-foreground">
                          {origin.host}
                        </span>
                      </td>
                      <td className="px-2 py-2 text-muted-foreground">{origin.hostType}</td>
                      <td className="px-2 py-2 font-mono text-muted-foreground">{origin.port}</td>
                      <td className="px-2 py-2">
                        <span
                          className="block font-mono text-[10px] tabular-nums text-foreground/75"
                          title={observed.join(", ")}
                        >
                          {observed.length}
                        </span>
                      </td>
                      <td className="px-2 py-2">
                        <CountCell
                          value={origin.counts.urls}
                          title={`Directory entries: ${origin.counts.directoryEntries}`}
                        />
                      </td>
                      <td className="px-2 py-2">
                        <CountCell value={origin.counts.apis} />
                      </td>
                      <td className="px-2 py-2">
                        <CountCell value={origin.counts.js} />
                      </td>
                      <td className="px-2 py-2">
                        <CountCell value={origin.counts.params} />
                      </td>
                      <td className="px-2 py-2">
                        <CountCell
                          value={origin.fingerprints.length}
                          title="Service/version fingerprints on this origin"
                        />
                      </td>
                      <td className="px-2 py-2">
                        <CountCell
                          value={origin.counts.evidence}
                          title={`Passive logs: ${origin.counts.passiveLogs}`}
                        />
                      </td>
                      <td className="px-2 py-2">
                        <ConfidenceBadge value={origin.confidence} />
                      </td>
                      <td
                        className="px-2 py-2 text-muted-foreground"
                        title={`Identity: ${IDENTITY_SOURCE_LABEL[origin.source]} · Counts: ${CONTENT_COUNT_SOURCE_LABEL[origin.contentCountSource]}`}
                      >
                        {origin.source}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Section>
      {selectedOrigin && (
        <OriginDetail
          origin={selectedOrigin}
          endpoints={selectedEndpoints as NetworkEndpointVM[]}
          loading={loading}
          projectPath={projectPath}
        />
      )}
    </div>
  );
}
