import { ArrowLeft, Globe, Loader2, RefreshCw } from "lucide-react";
import { useMemo, useState } from "react";
import type { Target } from "@/lib/pentest/types";
import { cn } from "@/lib/utils";
import { useTargetSurfaceData } from "./hooks/useTargetSurfaceData";
import { countEndpointParams } from "./surface/endpointParams";
import {
  buildSensitiveFindings,
  buildSitemapItems,
  buildSitemapTree,
  formatLatestEvidence,
  isHttpPort,
} from "./surface/surfaceModel";
import { EvidenceTab } from "./surface/tabs/EvidenceTab";
import { IdentityTab } from "./surface/tabs/IdentityTab";
import { SensitiveTab } from "./surface/tabs/SensitiveTab";
import { SitemapTab } from "./surface/tabs/SitemapTab";
import { SurfaceTabView } from "./surface/tabs/SurfaceTabView";
import { SURFACE_TABS, type SurfaceTab } from "./surface/types";

export function TargetSurfaceWorkbench({
  target,
  onUpdateNotes,
  onBack,
  backLabel,
  relatedDomains,
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
  onSelectDomain?: (id: string) => void;
}) {
  const [activeTab, setActiveTab] = useState<SurfaceTab>("surface");
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
  const relatedTargetIds = useMemo(
    () => safeRelatedDomains?.map((domain) => domain.id).filter(Boolean) ?? [],
    [safeRelatedDomains]
  );
  const { data, loading, error, reload } = useTargetSurfaceData(safeTarget.id, relatedTargetIds);

  const httpPorts = useMemo(
    () => safeTarget.ports.filter((port) => isHttpPort(port)),
    [safeTarget.ports]
  );
  const apiEndpoints = data.endpoints;
  const jsResults = data.jsResults;
  const sitemapItems = useMemo(() => buildSitemapItems(apiEndpoints), [apiEndpoints]);
  const sitemapTree = useMemo(() => buildSitemapTree(sitemapItems), [sitemapItems]);
  const endpointParamCount = useMemo(() => countEndpointParams(apiEndpoints), [apiEndpoints]);
  const sensitiveFindings = useMemo(
    () => buildSensitiveFindings(jsResults, data.passiveScans),
    [jsResults, data.passiveScans]
  );
  const sensitiveCount = useMemo(
    () => sensitiveFindings.reduce((count, item) => count + item.count, 0),
    [sensitiveFindings]
  );
  const tabCounts: Partial<Record<SurfaceTab, number>> = {
    surface: safeTarget.ports.length + data.fingerprints.length + (safeRelatedDomains?.length ?? 0),
    sitemap: sitemapItems.length,
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
        {activeTab === "identity" && (
          <IdentityTab target={safeTarget} onUpdateNotes={onUpdateNotes} />
        )}
        {activeTab === "surface" && (
          <SurfaceTabView
            target={safeTarget}
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
        {activeTab === "sitemap" && (
          <SitemapTab
            items={sitemapItems}
            tree={sitemapTree}
            jsResults={jsResults}
            loading={loading}
          />
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
          />
        )}
      </div>
    </div>
  );
}
