import { useEffect, useMemo, useState } from "react";
import {
  Check, ChevronDown, ChevronRight, Database, FileCode2, Globe,
  Loader2, Server, Shield, Wifi, Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { QuickNotes } from "@/components/QuickNotes/QuickNotes";
import {
  targetAssetsList,
  apiEndpointsList,
  fingerprintsList,
  jsAnalysisList,
  oplogListByTarget,
  type TargetAsset,
  type ApiEndpoint,
  type Fingerprint,
  type JsAnalysisResult,
  type AuditRow,
} from "@/lib/security-analysis";
import { type Target, type PortInfo } from "@/lib/pentest/types";

export function TargetDetailView({
  target,
  t,
  onUpdateNotes,
  onScan,
}: {
  target: Target;
  t: (key: string) => string;
  onUpdateNotes: (id: string, notes: string) => void;
  onScan?: (target: Target) => void;
}) {
  const [secData, setSecData] = useState<{
    assets: TargetAsset[];
    endpoints: ApiEndpoint[];
    fingerprints: Fingerprint[];
    jsResults: JsAnalysisResult[];
    logs: AuditRow[];
  }>({ assets: [], endpoints: [], fingerprints: [], jsResults: [], logs: [] });
  const [secLoading, setSecLoading] = useState(true);
  const [expandedPorts, setExpandedPorts] = useState<Set<number>>(new Set());
  const [showLogs, setShowLogs] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setSecLoading(true);
      try {
        const [a, e, f, j, l] = await Promise.all([
          targetAssetsList(target.id).catch(() => []),
          apiEndpointsList(target.id).catch(() => []),
          fingerprintsList(target.id).catch(() => []),
          jsAnalysisList(target.id).catch(() => []),
          oplogListByTarget(target.id, 50).catch(() => []),
        ]);
        if (!cancelled) {
          setSecData({
            assets: Array.isArray(a) ? a : [],
            endpoints: Array.isArray(e) ? e : [],
            fingerprints: Array.isArray(f) ? f : [],
            jsResults: Array.isArray(j) ? j : [],
            logs: Array.isArray(l) ? l : [],
          });
        }
      } catch { /* ignore */ }
      if (!cancelled) setSecLoading(false);
    })();
    return () => { cancelled = true; };
  }, [target.id]);

  const togglePort = (port: number) => {
    setExpandedPorts((prev) => {
      const next = new Set(prev);
      if (next.has(port)) next.delete(port); else next.add(port);
      return next;
    });
  };

  const extractPort = (url: string): number | null => {
    try {
      const u = new URL(url);
      if (u.port) return Number.parseInt(u.port);
      return u.protocol === "https:" ? 443 : 80;
    } catch {
      return null;
    }
  };

  const endpointsByPort = useMemo(() => {
    const map = new Map<number, ApiEndpoint[]>();
    for (const ep of secData.endpoints) {
      const port = extractPort(ep.url);
      if (port != null) {
        const arr = map.get(port) || [];
        arr.push(ep);
        map.set(port, arr);
      }
    }
    return map;
  }, [secData.endpoints]);

  const jsByPort = useMemo(() => {
    const map = new Map<number, JsAnalysisResult[]>();
    for (const js of secData.jsResults) {
      const port = extractPort(js.url);
      if (port != null) {
        const arr = map.get(port) || [];
        arr.push(js);
        map.set(port, arr);
      }
    }
    return map;
  }, [secData.jsResults]);

  const allPorts = useMemo(() => {
    const portSet = new Set<number>();
    for (const p of target.ports || []) portSet.add(p.port);
    for (const k of endpointsByPort.keys()) portSet.add(k);
    for (const k of jsByPort.keys()) portSet.add(k);
    return [...portSet].sort((a, b) => a - b);
  }, [target.ports, endpointsByPort, jsByPort]);

  const getPortInfo = (port: number): PortInfo | undefined =>
    target.ports?.find((p) => p.port === port);

  const methodCol: Record<string, string> = {
    GET: "text-green-400", POST: "text-blue-400", PUT: "text-yellow-400",
    DELETE: "text-red-400", PATCH: "text-purple-400",
  };
  const riskCol: Record<string, string> = {
    critical: "bg-red-500/10 text-red-400", high: "bg-orange-500/10 text-orange-400",
    medium: "bg-yellow-500/10 text-yellow-400", low: "bg-blue-500/10 text-blue-400",
  };

  return (
    <div className="mt-2 ml-5 space-y-2" onClick={(e) => e.stopPropagation()}>
      {/* Basic Info */}
      {target.name !== target.value && (
        <div className="text-[11px] text-muted-foreground">
          <span className="font-medium">{t("targets.name")}:</span> {target.name}
        </div>
      )}
      {target.source && target.source !== "manual" && (
        <div className="text-[11px] text-muted-foreground">
          <span className="font-medium">Source:</span> {target.source}
        </div>
      )}

      {/* Summary Bar */}
      {(() => {
        const httpPorts = allPorts.filter((p) => {
          const info = getPortInfo(p);
          return info?.service === "http" || info?.http_status != null;
        });
        return (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground px-2 py-1.5 rounded-md bg-muted/10">
            {allPorts.length > 0 && (
              <span className="flex items-center gap-1"><Wifi className="w-3 h-3 text-emerald-400" />{allPorts.length} Ports</span>
            )}
            {httpPorts.length > 0 && (
              <span className="flex items-center gap-1"><Globe className="w-3 h-3 text-blue-400" />{httpPorts.length} Web</span>
            )}
            {target.real_ip && (
              <span className="font-mono text-emerald-400/70">{target.real_ip}</span>
            )}
            {target.cdn_waf && (
              <span className="text-yellow-400/70">{target.cdn_waf}</span>
            )}
            {target.os_info && (
              <span className="text-pink-400/70">{target.os_info}</span>
            )}
          </div>
        );
      })()}

      {/* Fingerprints from DB */}
      {secData.fingerprints.length > 0 && (
        <div className="space-y-0.5">
          <div className="flex items-center gap-1 text-[11px] text-muted-foreground font-medium">
            <Shield className="w-3 h-3 text-purple-400" />
            Fingerprints ({secData.fingerprints.length})
          </div>
          <div className="pl-4 space-y-0.5">
            {secData.fingerprints.map((fp) => (
              <div key={fp.id} className="flex items-center gap-2 text-[10px]">
                <span className="px-1 py-0.5 rounded bg-purple-500/10 text-purple-400 text-[9px]">{fp.category}</span>
                <span className="text-foreground/70">{fp.name}</span>
                {fp.version && <span className="font-mono text-muted-foreground/40">{fp.version}</span>}
                <div className="flex items-center gap-0.5 ml-auto">
                  <div className="w-6 h-1 rounded-full bg-muted/20 overflow-hidden">
                    <div className={cn("h-full rounded-full", fp.confidence >= 80 ? "bg-green-500" : fp.confidence >= 50 ? "bg-yellow-500" : "bg-red-500")} style={{ width: `${Math.min(100, fp.confidence)}%` }} />
                  </div>
                  <span className="text-[8px] text-muted-foreground/30">{fp.confidence}%</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Scan shortcut */}
      {(target.type === "url" || target.type === "domain" || target.type === "ip") && onScan && (
        <button
          type="button"
          onClick={() => onScan(target)}
          className="flex items-center gap-1.5 px-2.5 py-1.5 text-[11px] font-medium rounded-md border border-blue-500/25 bg-blue-500/[0.06] text-blue-300 hover:bg-blue-500/15 transition-colors w-fit"
        >
          <Zap className="w-3 h-3" />
          Scan Target
        </button>
      )}

      {/* Services (per-port, expandable with HTTP metadata) */}
      {allPorts.length > 0 && (
        <div className="space-y-0.5">
          <div className="flex items-center gap-1 text-[11px] text-muted-foreground font-medium">
            <Server className="w-3 h-3" />
            Services ({allPorts.length})
            {secLoading && <Loader2 className="w-3 h-3 animate-spin text-muted-foreground/20 ml-1" />}
          </div>
          <div className="pl-1 space-y-1">
            {allPorts.map((port) => {
              const info = getPortInfo(port);
              const portEndpoints = endpointsByPort.get(port) || [];
              const portJs = jsByPort.get(port) || [];
              const isExpanded = expandedPorts.has(port);
              const isHttp = info?.service === "http" || info?.http_status != null;
              const hasChildren = portEndpoints.length > 0 || portJs.length > 0;

              return (
                <div key={port} className="rounded-lg border border-border/10 overflow-hidden">
                  <button
                    type="button"
                    onClick={() => togglePort(port)}
                    className="flex items-start gap-2 w-full px-2 py-1.5 text-left hover:bg-muted/10 transition-colors"
                  >
                    <span className={cn(
                      "w-1.5 h-1.5 rounded-full mt-1.5 flex-shrink-0",
                      isHttp ? "bg-emerald-400" : "bg-zinc-500",
                    )} />

                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-[11px] font-mono text-emerald-400 font-medium">:{port}</span>
                        {info?.protocol && <span className="text-[9px] text-muted-foreground/40">/{info.protocol}</span>}
                        {info?.service && <span className="text-[10px] text-foreground/50">{info.service}</span>}
                        {info?.state && info.state !== "open" && (
                          <span className="text-[9px] text-muted-foreground/30">[{info.state}]</span>
                        )}
                        {info?.http_status != null && (
                          <span className={cn("text-[10px] font-mono font-medium",
                            info.http_status < 300 ? "text-green-400" :
                            info.http_status < 400 ? "text-blue-400" :
                            info.http_status < 500 ? "text-yellow-400" : "text-red-400",
                          )}>[{info.http_status}]</span>
                        )}
                        <div className="flex items-center gap-1.5 ml-auto flex-shrink-0">
                          {portEndpoints.length > 0 && (
                            <span className="text-[9px] px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400">
                              {portEndpoints.length} ep
                            </span>
                          )}
                          {portJs.length > 0 && (
                            <span className="text-[9px] px-1.5 py-0.5 rounded bg-yellow-500/10 text-yellow-400">
                              {portJs.length} JS
                            </span>
                          )}
                          {(hasChildren || isHttp) && (
                            isExpanded
                              ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/30" />
                              : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/30" />
                          )}
                        </div>
                      </div>

                      {isHttp && info?.http_title && (
                        <div className="text-[10px] text-foreground/60 truncate mt-0.5">
                          {info.http_title}
                        </div>
                      )}
                      {isHttp && info?.webserver && (
                        <div className="text-[9px] text-muted-foreground/40 mt-0.5">
                          {info.webserver}
                        </div>
                      )}
                      {isHttp && info?.technologies && info.technologies.length > 0 && (
                        <div className="flex flex-wrap gap-0.5 mt-1">
                          {info.technologies.map((tech) => (
                            <span key={tech} className="text-[8px] px-1 py-0.5 rounded bg-purple-500/10 text-purple-400/70">{tech}</span>
                          ))}
                        </div>
                      )}
                    </div>
                  </button>

                  {isExpanded && (hasChildren || isHttp) && (
                    <div className="border-t border-border/5 px-2 py-1 space-y-1.5 bg-[var(--bg-hover)]/10">
                      {portEndpoints.length > 0 && (
                        <div>
                          <div className="text-[9px] text-muted-foreground/40 font-medium mb-0.5">Endpoints</div>
                          {portEndpoints.map((ep) => (
                            <div key={ep.id} className="flex items-center gap-2 py-0.5 text-[10px]">
                              <span className={cn("font-mono font-medium w-10 text-right flex-shrink-0", methodCol[ep.method] ?? "text-muted-foreground")}>{ep.method}</span>
                              <span className="font-mono text-foreground/60 flex-1 truncate">{ep.path}</span>
                              {ep.authType && <span className="text-muted-foreground/30 text-[9px]">{ep.authType}</span>}
                              <span className={cn("text-[8px] px-1 py-0.5 rounded", riskCol[ep.riskLevel] ?? "bg-zinc-500/10 text-zinc-400")}>{ep.riskLevel}</span>
                              {ep.tested ? <Check className="w-2.5 h-2.5 text-green-400" /> : <span className="text-muted-foreground/15 text-[9px]">—</span>}
                            </div>
                          ))}
                        </div>
                      )}

                      {portJs.length > 0 && (
                        <div>
                          <div className="text-[9px] text-muted-foreground/40 font-medium mb-0.5">JS Files</div>
                          {portJs.map((js) => {
                            const secrets = Array.isArray(js.secretsFound) ? js.secretsFound.length : 0;
                            return (
                              <div key={js.id} className="flex items-center gap-2 py-0.5 text-[10px]">
                                <FileCode2 className="w-3 h-3 text-yellow-400 flex-shrink-0" />
                                <span className="font-mono text-foreground/60 flex-1 truncate">{js.filename || js.url}</span>
                                {secrets > 0 && <span className="text-[8px] px-1 py-0.5 rounded bg-red-500/10 text-red-400">{secrets} secrets</span>}
                                {js.sourceMaps && <span className="text-[8px] px-1 py-0.5 rounded bg-yellow-500/10 text-yellow-400">srcmaps</span>}
                              </div>
                            );
                          })}
                        </div>
                      )}

                      {!hasChildren && isHttp && info?.url && (
                        <div className="text-[9px] text-muted-foreground/30 font-mono truncate py-1">{info.url}</div>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* Operation Logs (collapsible) */}
      {secData.logs.length > 0 && (
        <div className="rounded-lg border border-border/10 overflow-hidden">
          <button
            type="button"
            onClick={() => setShowLogs(!showLogs)}
            className="flex items-center gap-2 w-full px-2 py-1.5 text-left hover:bg-muted/10 transition-colors"
          >
            {showLogs ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/30" /> : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/30" />}
            <Database className="w-3 h-3 text-accent/50" />
            <span className="text-[11px] text-muted-foreground font-medium">Operation Logs ({secData.logs.length})</span>
          </button>
          {showLogs && (
            <div className="border-t border-border/5 max-h-[200px] overflow-y-auto">
              <SecLogs data={secData.logs} />
            </div>
          )}
        </div>
      )}

      {/* Notes */}
      <textarea
        className="w-full text-[11px] bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent resize-none"
        placeholder={t("targets.notes")}
        rows={2}
        defaultValue={target.notes}
        onBlur={(e) => {
          if (e.target.value !== target.notes) {
            onUpdateNotes(target.id, e.target.value);
          }
        }}
      />
      <QuickNotes entityType="target" entityId={target.id} compact />
    </div>
  );
}

function SecLogs({ data }: { data: AuditRow[] }) {
  if (data.length === 0) return <EmptySecData label="No operation logs for this target" />;
  const statusDot: Record<string, string> = {
    completed: "bg-green-500", running: "bg-yellow-500", failed: "bg-red-500", pending: "bg-zinc-500",
  };
  return (
    <div className="divide-y divide-border/5">
      {data.map((e) => (
        <div key={e.id} className="flex items-center gap-2 px-3 py-1.5 text-[10px]">
          <span className={cn("w-1.5 h-1.5 rounded-full flex-shrink-0", statusDot[e.status] ?? "bg-zinc-500")} />
          <span className="text-muted-foreground/30 font-mono w-24 flex-shrink-0 text-[9px]">
            {new Date(e.createdAt).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" })}
          </span>
          <span className="text-foreground/70 flex-1 truncate">{e.action}</span>
          {e.toolName && <span className="font-mono text-accent/50 bg-accent/5 px-1 py-0.5 rounded text-[9px]">{e.toolName}</span>}
        </div>
      ))}
    </div>
  );
}

function EmptySecData({ label }: { label: string }) {
  return (
    <div className="text-center text-[10px] text-muted-foreground/20 py-6">
      {label}
    </div>
  );
}
