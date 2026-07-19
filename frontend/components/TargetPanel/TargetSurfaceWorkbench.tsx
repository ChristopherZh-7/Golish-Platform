import {
  ArrowLeft,
  Braces,
  Code2,
  FileCode2,
  Globe,
  Link2,
  Loader2,
  Network,
  RefreshCw,
  Server,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { PortInfo, Target } from "@/lib/pentest/types";
import { getProjectPath } from "@/lib/projects";
import { surfaceIdentityBackfill } from "@/lib/security-analysis";
import { cn } from "@/lib/utils";
import { useTargetSurfaceData } from "./hooks/useTargetSurfaceData";
import { composeBackendSurfaceHierarchy } from "./surface/backendSurfaceHierarchy";
import { countEndpointParams } from "./surface/endpointParams";
import { EmptyInline, Metric, Section } from "./surface/SurfaceParts";
import { buildSurfaceHierarchy, type SurfaceHierarchyVM } from "./surface/surfaceHierarchy";
import {
  buildSensitiveFindings,
  buildSitemapItems,
  formatLatestEvidence,
  isHttpPort,
} from "./surface/surfaceModel";
import { EvidenceTab } from "./surface/tabs/EvidenceTab";
import { IdentityTab } from "./surface/tabs/IdentityTab";
import { IpFingerprintTab } from "./surface/tabs/IpFingerprintTab";
import { NetworkEndpointsTab } from "./surface/tabs/NetworkEndpointsTab";
import { RelatedDomainsTab } from "./surface/tabs/RelatedDomainsTab";
import { SensitiveTab } from "./surface/tabs/SensitiveTab";
import { SitemapTab } from "./surface/tabs/SitemapTab";
import { SurfaceTabView } from "./surface/tabs/SurfaceTabView";
import { WebOriginsTab } from "./surface/tabs/WebOriginsTab";
import { SURFACE_TABS, type SurfaceTab } from "./surface/types";
import { buildWhatWebAssessments } from "./surface/whatWebAssessment";

type IpSurfaceTab =
  | "overview"
  | "endpoints"
  | "origins"
  | "domains"
  | "fingerprints"
  | "sensitive"
  | "evidence";
type WorkbenchTab = SurfaceTab | IpSurfaceTab;

export const IP_SURFACE_TABS: Array<{ id: IpSurfaceTab; label: string }> = [
  { id: "overview", label: "Overview" },
  { id: "endpoints", label: "Network Endpoints" },
  { id: "origins", label: "Web Origins" },
  { id: "domains", label: "Related Domains" },
  { id: "fingerprints", label: "Fingerprints" },
  { id: "sensitive", label: "Sensitive" },
  { id: "evidence", label: "Evidence" },
];
const IP_TAB_IDS = new Set<WorkbenchTab>(IP_SURFACE_TABS.map((tab) => tab.id));
const LEGACY_TAB_IDS = new Set<WorkbenchTab>(SURFACE_TABS.map((tab) => tab.id));

function unassignedWebDataCount(hierarchy: SurfaceHierarchyVM): number {
  const { unassignedWebData } = hierarchy;
  return (
    unassignedWebData.urls.length +
    unassignedWebData.apis.length +
    unassignedWebData.js.length +
    unassignedWebData.params.length
  );
}

function IpOverviewTab({
  hierarchy,
  loading,
  onSelectOrigin,
}: {
  hierarchy: SurfaceHierarchyVM;
  loading: boolean;
  onSelectOrigin: (id: string) => void;
}) {
  const unassignedCount = unassignedWebDataCount(hierarchy);
  const backendContentCounts = hierarchy.summary.contentCountSource === "backend_content_counts";
  const backendUnassignedCount = hierarchy.summary.contentUnassignedCount ?? 0;
  const backendUnmatchedCount = hierarchy.summary.contentUnmatchedOriginCount ?? 0;
  const showBackendAggregation =
    backendContentCounts && (backendUnassignedCount > 0 || backendUnmatchedCount > 0);
  return (
    <div className="space-y-2.5">
      <Section title="IP Surface Overview" subtitle={hierarchy.rootTarget.value}>
        <div className="grid grid-cols-2 gap-1.5 lg:grid-cols-4">
          <Metric
            icon={<Server className="h-3.5 w-3.5" />}
            label="Endpoints"
            value={hierarchy.summary.endpointCount}
          />
          <Metric
            icon={<Globe className="h-3.5 w-3.5" />}
            label="Origins"
            value={hierarchy.summary.webOriginCount}
          />
          <Metric
            icon={<Link2 className="h-3.5 w-3.5" />}
            label="URLs"
            value={hierarchy.summary.urlCount}
          />
          <Metric
            icon={<Code2 className="h-3.5 w-3.5" />}
            label="APIs"
            value={hierarchy.summary.apiCount}
          />
          <Metric
            icon={<FileCode2 className="h-3.5 w-3.5" />}
            label="JS"
            value={hierarchy.summary.jsCount}
          />
          <Metric
            icon={<Braces className="h-3.5 w-3.5" />}
            label="Params"
            value={hierarchy.summary.paramCount}
          />
          <Metric
            icon={<Network className="h-3.5 w-3.5" />}
            label="Domains"
            value={hierarchy.summary.domainCount}
          />
          <Metric
            icon={<RefreshCw className="h-3.5 w-3.5" />}
            label="Evidence"
            value={hierarchy.summary.evidenceCount}
          />
        </div>
      </Section>

      {showBackendAggregation && (
        <Section title="Backend content aggregation" subtitle="counts only, no synthesized origins">
          <div className="rounded border border-blue-500/25 bg-blue-500/5 px-2 py-1.5 text-[10px] text-blue-100/85">
            Backend content aggregation found {backendUnassignedCount} unassigned items and{" "}
            {backendUnmatchedCount} unmatched-origin items.
          </div>
        </Section>
      )}

      <Section title="Web Origins" subtitle={`${hierarchy.webOrigins.length} inferred root(s)`}>
        {hierarchy.webOrigins.length === 0 ? (
          <EmptyInline
            loading={loading}
            label="No complete Web Origin URLs have been collected yet."
          />
        ) : (
          <div className="space-y-1">
            {hierarchy.webOrigins.slice(0, 8).map((origin) => (
              <button
                key={origin.id}
                type="button"
                onClick={() => onSelectOrigin(origin.id)}
                className="flex w-full min-w-0 items-center gap-2 rounded border border-border/20 bg-muted/5 px-2 py-1.5 text-left hover:border-accent/30 hover:bg-muted/15"
              >
                <Globe className="h-3.5 w-3.5 flex-shrink-0 text-accent/75" />
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/85">
                  {origin.origin}
                </span>
                <span className="rounded bg-muted/25 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                  {origin.counts.urls} URLs
                </span>
                <span
                  className={cn(
                    "rounded px-1.5 py-0.5 text-[9px]",
                    origin.confidence === "confirmed"
                      ? "bg-green-500/10 text-green-300"
                      : "bg-yellow-500/10 text-yellow-300"
                  )}
                >
                  {origin.confidence}
                </span>
              </button>
            ))}
          </div>
        )}
      </Section>

      {unassignedCount > 0 && (
        <Section title="未归属 Web 数据" subtitle={`${unassignedCount} item(s)`}>
          <div className="rounded border border-yellow-500/25 bg-yellow-500/5 px-2 py-1.5 text-[10px] text-yellow-100/85">
            <p>{hierarchy.unassignedWebData.reason}</p>
            <div className="mt-1 flex flex-wrap gap-1">
              <span className="rounded bg-background/25 px-1.5 py-0.5">
                URLs {hierarchy.unassignedWebData.urls.length}
              </span>
              <span className="rounded bg-background/25 px-1.5 py-0.5">
                APIs {hierarchy.unassignedWebData.apis.length}
              </span>
              <span className="rounded bg-background/25 px-1.5 py-0.5">
                JS {hierarchy.unassignedWebData.js.length}
              </span>
              <span className="rounded bg-background/25 px-1.5 py-0.5">
                Params {hierarchy.unassignedWebData.params.length}
              </span>
            </div>
          </div>
        </Section>
      )}
    </div>
  );
}

function mergePortsForHost(target: Target, relatedTargets: Target[] | undefined): PortInfo[] {
  const portsByKey = new Map<string, PortInfo>();
  const addPort = (port: PortInfo) => {
    const key = `${port.port}:${port.protocol ?? "tcp"}`;
    portsByKey.set(key, { ...portsByKey.get(key), ...port });
  };
  for (const port of target.ports) addPort(port);
  if (target.type === "ip") {
    for (const related of relatedTargets ?? []) {
      for (const port of related.ports) addPort(port);
    }
  }
  return [...portsByKey.values()].sort((a, b) => {
    if (a.port !== b.port) return a.port - b.port;
    return (a.protocol ?? "tcp").localeCompare(b.protocol ?? "tcp");
  });
}

export function TargetSurfaceWorkbench({
  target,
  onUpdateNotes,
  onBack,
  backLabel,
  relatedDomains,
  relatedWebTargets,
  onSelectDomain,
}: {
  target: Target;
  onUpdateNotes: (id: string, notes: string) => void;
  // When the workbench was reached by drilling in from a host (IP) panel, the
  // parent passes `onBack` so the user can return to the host's member list.
  onBack?: () => void;
  backLabel?: string;
  // When the subject is an IP/host, the parent passes the domains that resolve
  // to it; the Surface tab renders them as a clickable "domains" block.
  relatedDomains?: Target[];
  // URL targets used only for WebOrigin inference. This may include IP-literal
  // URL targets, which must not be displayed as Related Domains.
  relatedWebTargets?: Target[];
  onSelectDomain?: (id: string) => void;
}) {
  const [activeTab, setActiveTab] = useState<WorkbenchTab>("surface");
  const [selectedOriginId, setSelectedOriginId] = useState<string | null>(null);
  const [backfillRunning, setBackfillRunning] = useState(false);
  const [backfillResult, setBackfillResult] = useState<string | null>(null);
  const safeTarget = useMemo(
    () => ({
      ...target,
      ports: Array.isArray(target.ports) ? target.ports : [],
    }),
    [target]
  );
  const safeRelatedDomains = useMemo(
    () =>
      relatedDomains?.map((domain) => ({
        ...domain,
        ports: Array.isArray(domain.ports) ? domain.ports : [],
      })),
    [relatedDomains]
  );
  const safeRelatedWebTargets = useMemo(
    () =>
      relatedWebTargets?.map((webTarget) => ({
        ...webTarget,
        ports: Array.isArray(webTarget.ports) ? webTarget.ports : [],
      })),
    [relatedWebTargets]
  );
  const relatedTargetIds = useMemo(() => {
    const ids = new Set<string>();
    for (const item of safeRelatedDomains ?? []) if (item.id) ids.add(item.id);
    for (const item of safeRelatedWebTargets ?? []) if (item.id) ids.add(item.id);
    return [...ids];
  }, [safeRelatedDomains, safeRelatedWebTargets]);
  const projectPath = getProjectPath();
  const {
    data,
    loading,
    error,
    reload,
    backendHierarchy,
    backendHierarchyStatus,
    backendHierarchyError,
  } = useTargetSurfaceData(safeTarget.id, relatedTargetIds, {
    loadBackendHierarchy: safeTarget.type === "ip",
    includeRelatedBackend: true,
  });
  const servicePorts = useMemo(
    () => mergePortsForHost(safeTarget, safeRelatedDomains),
    [safeTarget, safeRelatedDomains]
  );
  const serviceTarget = useMemo(
    () =>
      safeTarget.type === "ip"
        ? { ...safeTarget, ports: servicePorts }
        : { ...safeTarget, ports: [] },
    [safeTarget, servicePorts]
  );

  const httpPorts = useMemo(
    () => serviceTarget.ports.filter((port) => isHttpPort(port)),
    [serviceTarget.ports]
  );
  const apiEndpoints = data.endpoints;
  const jsResults = data.jsResults;
  const sitemapItems = useMemo(
    () => buildSitemapItems(apiEndpoints, jsResults, data.directoryEntries),
    [apiEndpoints, data.directoryEntries, jsResults]
  );
  const frontendHierarchy = useMemo(
    () =>
      buildSurfaceHierarchy({
        rootTarget: safeTarget,
        servicePorts,
        relatedDomains: safeRelatedDomains,
        relatedWebTargets: safeRelatedWebTargets,
        assets: data.assets,
        apiEndpoints,
        jsResults,
        directoryEntries: data.directoryEntries,
        fingerprints: data.fingerprints,
        passiveScans: data.passiveScans,
        timeline: data.timeline,
        logs: data.logs,
      }),
    [
      apiEndpoints,
      data,
      jsResults,
      safeRelatedDomains,
      safeRelatedWebTargets,
      safeTarget,
      servicePorts,
    ]
  );
  const hierarchyMerge = useMemo(
    () => composeBackendSurfaceHierarchy(frontendHierarchy, backendHierarchy),
    [backendHierarchy, frontendHierarchy]
  );
  const hierarchy = hierarchyMerge.hierarchy;
  const whatWebAssessments = useMemo(
    () =>
      buildWhatWebAssessments(data.logs, new Set(hierarchy.webOrigins.map((origin) => origin.id))),
    [data.logs, hierarchy.webOrigins]
  );
  const isIpSurface = hierarchy.mode === "ip";
  const endpointParamCount = useMemo(() => countEndpointParams(apiEndpoints), [apiEndpoints]);
  const sensitiveFindings = useMemo(
    () => buildSensitiveFindings(jsResults, data.passiveScans),
    [jsResults, data.passiveScans]
  );
  const sensitiveCount = useMemo(
    () => sensitiveFindings.reduce((count, item) => count + item.count, 0),
    [sensitiveFindings]
  );
  const tabCounts: Partial<Record<WorkbenchTab, number>> = isIpSurface
    ? {
        endpoints: hierarchy.summary.endpointCount,
        origins: hierarchy.summary.webOriginCount,
        domains: hierarchy.summary.domainCount,
        fingerprints: data.fingerprints.length,
        sensitive: sensitiveCount,
        evidence: hierarchy.summary.evidenceCount,
      }
    : {
        surface:
          serviceTarget.ports.length + data.fingerprints.length + (safeRelatedDomains?.length ?? 0),
        sitemap: sitemapItems.length,
        sensitive: sensitiveCount,
        evidence: data.timeline.length || data.logs.length,
      };
  const visibleTabs = isIpSurface ? IP_SURFACE_TABS : SURFACE_TABS;
  const lastEvidenceLabel = useMemo(
    () => formatLatestEvidence(data.timeline[0]?.createdAt, data.logs[0]?.createdAt),
    [data.timeline, data.logs]
  );

  useEffect(() => {
    if (isIpSurface && !IP_TAB_IDS.has(activeTab)) setActiveTab("overview");
    if (!isIpSurface && !LEGACY_TAB_IDS.has(activeTab)) setActiveTab("surface");
  }, [activeTab, isIpSurface]);

  useEffect(() => {
    if (!selectedOriginId) return;
    if (!hierarchy.webOrigins.some((origin) => origin.id === selectedOriginId)) {
      setSelectedOriginId(null);
    }
  }, [hierarchy.webOrigins, selectedOriginId]);

  const runIdentityBackfill = useCallback(async () => {
    setBackfillRunning(true);
    setBackfillResult(null);
    try {
      const summary = await surfaceIdentityBackfill(projectPath, safeTarget.organization_id);
      setBackfillResult(
        `Identity backfill complete: ${summary.createdOrUpdatedNetworkEndpoints} endpoint(s), ${summary.createdOrUpdatedWebOrigins} origin(s), ${summary.createdOrUpdatedObservations} observation(s).`
      );
      await reload();
    } catch (err) {
      setBackfillResult(`Identity backfill failed: ${String(err)}`);
    } finally {
      setBackfillRunning(false);
    }
  }, [projectPath, reload, safeTarget.organization_id]);

  return (
    <div className="h-full min-h-0 flex flex-col bg-background/20">
      <header className="border-b border-border/25 px-3 py-2">
        <div className="flex items-start justify-between gap-2.5">
          <div className="min-w-0">
            <div className="flex items-center gap-2 min-w-0">
              {onBack && (
                <button
                  type="button"
                  onClick={onBack}
                  className="inline-flex h-5 flex-shrink-0 items-center gap-0.5 rounded border border-border/30 bg-background/20 px-1.5 text-[10px] text-muted-foreground hover:bg-muted/25 hover:text-foreground"
                  title={backLabel ?? "Back"}
                >
                  <ArrowLeft className="w-3 h-3" />
                  {backLabel && <span className="max-w-[120px] truncate">{backLabel}</span>}
                </button>
              )}
              <Globe className="w-3.5 h-3.5 text-accent flex-shrink-0" />
              <h3 className="truncate text-[13px] font-semibold text-foreground">
                {safeTarget.value}
              </h3>
              <span
                className={cn(
                  "rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase leading-none",
                  safeTarget.scope === "in"
                    ? "bg-green-500/10 text-green-300"
                    : "bg-red-500/10 text-red-300"
                )}
              >
                {safeTarget.scope} scope
              </span>
            </div>
            <div className="mt-0.5 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
              <span>{safeTarget.type}</span>
              <span>{safeTarget.source || "manual"}</span>
              {safeTarget.real_ip && <span className="font-mono">{safeTarget.real_ip}</span>}
              {safeTarget.cdn_waf && <span>{safeTarget.cdn_waf}</span>}
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
        {visibleTabs.map((tab) => (
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

      {isIpSurface && (
        <div className="flex items-start justify-between gap-2 border-b border-border/20 px-3 py-1.5 text-[10px] text-muted-foreground">
          <div className="min-w-0">
            {backendHierarchyStatus === "loading"
              ? "Loading backend identity hierarchy. Web content is still matched by Web Origin."
              : hierarchyMerge.message}
            {backendHierarchyError && (
              <span className="ml-2 text-red-300/85">{backendHierarchyError}</span>
            )}
            {!backendHierarchyError && hierarchyMerge.fallbackReason && (
              <span className="ml-2">{hierarchyMerge.fallbackReason}</span>
            )}
            {backfillResult && <span className="ml-2 text-accent/85">{backfillResult}</span>}
          </div>
          <button
            type="button"
            onClick={() => void runIdentityBackfill()}
            disabled={backfillRunning}
            title="Build backend NetworkEndpoint / WebOrigin / observation rows from existing legacy data (additive, idempotent)."
            className="inline-flex h-5 flex-shrink-0 items-center gap-1 rounded border border-border/30 bg-background/20 px-1.5 text-[10px] text-muted-foreground hover:bg-muted/25 hover:text-foreground disabled:opacity-50"
          >
            {backfillRunning ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <Network className="h-3 w-3" />
            )}
            {backfillRunning ? "Building identity…" : "Build identity from data"}
          </button>
        </div>
      )}

      <div
        className={cn(
          "min-h-0 flex-1 p-2.5",
          activeTab === "sitemap" ? "overflow-hidden" : "overflow-y-auto"
        )}
      >
        {isIpSurface && activeTab === "overview" && (
          <IpOverviewTab
            hierarchy={hierarchy}
            loading={loading}
            onSelectOrigin={(id) => {
              setSelectedOriginId(id);
              setActiveTab("origins");
            }}
          />
        )}
        {isIpSurface && activeTab === "endpoints" && (
          <NetworkEndpointsTab
            endpoints={hierarchy.endpoints}
            webOrigins={hierarchy.webOrigins}
            loading={loading}
            selectedOriginId={selectedOriginId}
            onSelectOrigin={(id) => {
              setSelectedOriginId(id);
              setActiveTab("origins");
            }}
          />
        )}
        {isIpSurface && activeTab === "origins" && (
          <WebOriginsTab
            webOrigins={hierarchy.webOrigins}
            endpoints={hierarchy.endpoints}
            loading={loading}
            selectedOriginId={selectedOriginId}
            onSelectOrigin={setSelectedOriginId}
            projectPath={projectPath}
            assessmentByOrigin={whatWebAssessments}
          />
        )}
        {isIpSurface && activeTab === "domains" && (
          <RelatedDomainsTab
            relatedDomains={hierarchy.relatedDomains}
            webOrigins={hierarchy.webOrigins}
            loading={loading}
            onSelectDomain={onSelectDomain}
            onSelectOrigin={(id) => {
              setSelectedOriginId(id);
              setActiveTab("origins");
            }}
          />
        )}
        {isIpSurface && activeTab === "fingerprints" && (
          <IpFingerprintTab
            fingerprints={data.fingerprints}
            webOrigins={hierarchy.webOrigins}
            loading={loading}
          />
        )}
        {!isIpSurface && activeTab === "identity" && (
          <IdentityTab target={safeTarget} onUpdateNotes={onUpdateNotes} />
        )}
        {!isIpSurface && activeTab === "surface" && (
          <SurfaceTabView
            target={serviceTarget}
            httpPorts={httpPorts}
            endpointCount={apiEndpoints.length}
            endpointParamCount={endpointParamCount}
            jsCount={jsResults.length}
            fingerprints={data.fingerprints}
            loading={loading}
            relatedDomains={safeRelatedDomains}
            onSelectDomain={onSelectDomain}
          />
        )}
        {!isIpSurface && (
          <div className={cn("h-full min-h-0", activeTab === "sitemap" ? "block" : "hidden")}>
            <SitemapTab
              items={sitemapItems}
              jsResults={jsResults}
              loading={loading}
              projectPath={projectPath}
              unassignedWebData={hierarchy.unassignedWebData}
            />
          </div>
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
            target={safeTarget}
            timeline={data.timeline}
            logs={data.logs}
            loading={loading}
            hierarchy={hierarchy}
          />
        )}
      </div>
    </div>
  );
}
