import { useCallback, useState } from "react";
import {
  Check, ChevronDown, ChevronRight, FileCode2, Globe, Loader2, Search,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  targetAssetsList, apiEndpointsList, fingerprintsList, jsAnalysisList,
  type TargetAsset, type ApiEndpoint,
  type Fingerprint, type JsAnalysisResult,
} from "@/lib/security-analysis";
import { methodColor, formatSize } from "./shared";

export function ReconDataPanel() {
  const [subTab, setSubTab] = useState<"assets" | "endpoints" | "fingerprints">("assets");
  const [targetId, setTargetId] = useState("");
  const [assets, setAssets] = useState<TargetAsset[]>([]);
  const [endpoints, setEndpoints] = useState<ApiEndpoint[]>([]);
  const [fingerprints, setFingerprints] = useState<Fingerprint[]>([]);
  const [loading, setLoading] = useState(false);

  const loadData = useCallback(async () => {
    if (!targetId.trim()) return;
    setLoading(true);
    try {
      const [a, e, f] = await Promise.all([
        targetAssetsList(targetId).catch(() => []),
        apiEndpointsList(targetId).catch(() => []),
        fingerprintsList(targetId).catch(() => []),
      ]);
      setAssets(Array.isArray(a) ? a : []);
      setEndpoints(Array.isArray(e) ? e : []);
      setFingerprints(Array.isArray(f) ? f : []);
    } catch {
      /* ignore */
    }
    setLoading(false);
  }, [targetId]);

  const subTabs: { id: "assets" | "endpoints" | "fingerprints"; label: string; count: number }[] = [
    { id: "assets", label: "Assets", count: assets.length },
    { id: "endpoints", label: "Endpoints", count: endpoints.length },
    { id: "fingerprints", label: "Fingerprints", count: fingerprints.length },
  ];

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-3 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <Globe className="w-3.5 h-3.5 text-accent" />
        <span className="text-[12px] font-medium text-foreground/80">Recon Data</span>
        <div className="flex-1" />
        <div className="flex items-center gap-1.5 bg-muted/10 rounded-lg px-2 py-1">
          <Search className="w-3 h-3 text-muted-foreground/30" />
          <input
            type="text"
            value={targetId}
            onChange={(e) => setTargetId(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && loadData()}
            placeholder="Target UUID..."
            className="bg-transparent text-[10px] text-foreground outline-none w-48 placeholder:text-muted-foreground/20 font-mono"
          />
        </div>
        <button onClick={loadData} className="px-2.5 py-1 rounded-md text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors">
          Load
        </button>
      </div>

      {/* Sub-tabs */}
      <div className="flex items-center gap-1 px-4 py-1.5 border-b border-border/5 flex-shrink-0">
        {subTabs.map((st) => (
          <button
            key={st.id}
            onClick={() => setSubTab(st.id)}
            className={cn(
              "px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors",
              subTab === st.id ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground"
            )}
          >
            {st.label}
            {st.count > 0 && <span className="ml-1 text-[8px] text-muted-foreground/30">({st.count})</span>}
          </button>
        ))}
      </div>

      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="h-full flex items-center justify-center">
            <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
          </div>
        ) : !targetId.trim() ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <Globe className="w-10 h-10" />
            <p className="text-[12px]">Enter a target UUID to view recon data</p>
            <p className="text-[10px] text-muted-foreground/15">Data is populated by the AI agent during security analysis</p>
          </div>
        ) : subTab === "assets" ? (
          <ReconAssetsTable data={assets} />
        ) : subTab === "endpoints" ? (
          <ReconEndpointsTable data={endpoints} />
        ) : (
          <ReconFingerprintsTable data={fingerprints} />
        )}
      </div>
    </div>
  );
}

function ReconAssetsTable({ data }: { data: TargetAsset[] }) {
  if (data.length === 0) {
    return <div className="text-center text-[11px] text-muted-foreground/20 py-12">No assets discovered</div>;
  }
  return (
    <table className="w-full text-[10px]">
      <thead className="sticky top-0 bg-card z-10">
        <tr className="text-muted-foreground/30 text-left border-b border-border/10">
          <th className="px-4 py-2 font-medium">Type</th>
          <th className="px-2 py-2 font-medium">Value</th>
          <th className="px-2 py-2 font-medium w-[60px]">Port</th>
          <th className="px-2 py-2 font-medium w-[80px]">Service</th>
          <th className="px-2 py-2 font-medium w-[80px]">Version</th>
          <th className="px-2 py-2 font-medium w-[70px]">Status</th>
        </tr>
      </thead>
      <tbody>
        {data.map((a) => (
          <tr key={a.id} className="border-b border-border/5 hover:bg-[var(--bg-hover)]/20">
            <td className="px-4 py-1.5">
              <span className="text-[9px] font-medium px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400">{a.assetType}</span>
            </td>
            <td className="px-2 py-1.5 font-mono text-foreground/70">{a.value}</td>
            <td className="px-2 py-1.5 text-muted-foreground/40 font-mono">{a.port ?? "-"}</td>
            <td className="px-2 py-1.5 text-muted-foreground/40">{a.service ?? "-"}</td>
            <td className="px-2 py-1.5 text-muted-foreground/40">{a.version ?? "-"}</td>
            <td className="px-2 py-1.5">
              <span className={cn(
                "text-[9px] px-1.5 py-0.5 rounded",
                a.status === "active" ? "bg-green-500/10 text-green-400" : "bg-zinc-500/10 text-zinc-400"
              )}>{a.status}</span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ReconEndpointsTable({ data }: { data: ApiEndpoint[] }) {
  if (data.length === 0) {
    return <div className="text-center text-[11px] text-muted-foreground/20 py-12">No endpoints discovered</div>;
  }
  const riskBadge: Record<string, string> = {
    critical: "bg-red-500/10 text-red-400",
    high: "bg-orange-500/10 text-orange-400",
    medium: "bg-yellow-500/10 text-yellow-400",
    low: "bg-blue-500/10 text-blue-400",
    info: "bg-zinc-500/10 text-zinc-400",
  };
  return (
    <table className="w-full text-[10px]">
      <thead className="sticky top-0 bg-card z-10">
        <tr className="text-muted-foreground/30 text-left border-b border-border/10">
          <th className="px-4 py-2 font-medium w-[60px]">Method</th>
          <th className="px-2 py-2 font-medium">Path</th>
          <th className="px-2 py-2 font-medium w-[60px]">Auth</th>
          <th className="px-2 py-2 font-medium w-[60px]">Risk</th>
          <th className="px-2 py-2 font-medium w-[60px]">Tested</th>
          <th className="px-2 py-2 font-medium w-[70px]">Source</th>
        </tr>
      </thead>
      <tbody>
        {data.map((ep) => (
          <tr key={ep.id} className="border-b border-border/5 hover:bg-[var(--bg-hover)]/20">
            <td className={cn("px-4 py-1.5 font-mono font-medium", methodColor(ep.method))}>{ep.method}</td>
            <td className="px-2 py-1.5 font-mono text-foreground/60">{ep.path}</td>
            <td className="px-2 py-1.5 text-muted-foreground/40">{ep.authType ?? "-"}</td>
            <td className="px-2 py-1.5">
              <span className={cn("text-[9px] px-1.5 py-0.5 rounded", riskBadge[ep.riskLevel] ?? "bg-zinc-500/10 text-zinc-400")}>{ep.riskLevel}</span>
            </td>
            <td className="px-2 py-1.5">
              {ep.tested
                ? <Check className="w-3 h-3 text-green-400" />
                : <span className="text-muted-foreground/20">—</span>}
            </td>
            <td className="px-2 py-1.5 text-muted-foreground/30">{ep.source}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ReconFingerprintsTable({ data }: { data: Fingerprint[] }) {
  if (data.length === 0) {
    return <div className="text-center text-[11px] text-muted-foreground/20 py-12">No fingerprints detected</div>;
  }
  return (
    <table className="w-full text-[10px]">
      <thead className="sticky top-0 bg-card z-10">
        <tr className="text-muted-foreground/30 text-left border-b border-border/10">
          <th className="px-4 py-2 font-medium">Category</th>
          <th className="px-2 py-2 font-medium">Name</th>
          <th className="px-2 py-2 font-medium w-[80px]">Version</th>
          <th className="px-2 py-2 font-medium w-[80px]">Confidence</th>
          <th className="px-2 py-2 font-medium w-[120px]">CPE</th>
          <th className="px-2 py-2 font-medium w-[70px]">Source</th>
        </tr>
      </thead>
      <tbody>
        {data.map((fp) => (
          <tr key={fp.id} className="border-b border-border/5 hover:bg-[var(--bg-hover)]/20">
            <td className="px-4 py-1.5">
              <span className="text-[9px] font-medium px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400">{fp.category}</span>
            </td>
            <td className="px-2 py-1.5 text-foreground/70 font-medium">{fp.name}</td>
            <td className="px-2 py-1.5 font-mono text-muted-foreground/40">{fp.version ?? "-"}</td>
            <td className="px-2 py-1.5">
              <div className="flex items-center gap-1.5">
                <div className="w-12 h-1 rounded-full bg-muted/20 overflow-hidden">
                  <div
                    className={cn("h-full rounded-full", fp.confidence >= 80 ? "bg-green-500" : fp.confidence >= 50 ? "bg-yellow-500" : "bg-red-500")}
                    style={{ width: `${Math.min(100, fp.confidence)}%` }}
                  />
                </div>
                <span className="text-muted-foreground/30">{fp.confidence}%</span>
              </div>
            </td>
            <td className="px-2 py-1.5 font-mono text-[9px] text-muted-foreground/30 truncate max-w-[120px]">{fp.cpe ?? "-"}</td>
            <td className="px-2 py-1.5 text-muted-foreground/30">{fp.source}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── JS Analysis Panel ──

export function JsAnalysisPanel() {
  const [targetId, setTargetId] = useState("");
  const [results, setResults] = useState<JsAnalysisResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const loadData = useCallback(async () => {
    if (!targetId.trim()) return;
    setLoading(true);
    try {
      const data = await jsAnalysisList(targetId);
      setResults(Array.isArray(data) ? data : []);
    } catch {
      setResults([]);
    }
    setLoading(false);
  }, [targetId]);

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-3 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <FileCode2 className="w-3.5 h-3.5 text-accent" />
        <span className="text-[12px] font-medium text-foreground/80">JS Analysis Results</span>
        <span className="text-[10px] text-muted-foreground/30">{results.length} files</span>
        <div className="flex-1" />
        <div className="flex items-center gap-1.5 bg-muted/10 rounded-lg px-2 py-1">
          <Search className="w-3 h-3 text-muted-foreground/30" />
          <input
            type="text"
            value={targetId}
            onChange={(e) => setTargetId(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && loadData()}
            placeholder="Target UUID..."
            className="bg-transparent text-[10px] text-foreground outline-none w-48 placeholder:text-muted-foreground/20 font-mono"
          />
        </div>
        <button onClick={loadData} className="px-2.5 py-1 rounded-md text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors">
          Load
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="h-full flex items-center justify-center">
            <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
          </div>
        ) : !targetId.trim() ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <FileCode2 className="w-10 h-10" />
            <p className="text-[12px]">Enter a target UUID to view JS analysis</p>
            <p className="text-[10px] text-muted-foreground/15">Results from JS security analysis appear here</p>
          </div>
        ) : results.length === 0 ? (
          <div className="text-center text-[11px] text-muted-foreground/20 py-12">No JS analysis results</div>
        ) : (
          <div className="space-y-2 p-4">
            {results.map((r) => {
              const isExpanded = expandedId === r.id;
              const secretCount = Array.isArray(r.secretsFound) ? r.secretsFound.length : 0;
              const endpointCount = Array.isArray(r.endpointsFound) ? r.endpointsFound.length : 0;
              return (
                <div key={r.id} className="rounded-xl border border-border/10 bg-[var(--bg-hover)]/15 overflow-hidden">
                  <button
                    type="button"
                    onClick={() => setExpandedId(isExpanded ? null : r.id)}
                    className="w-full flex items-center gap-3 p-3 text-left hover:bg-[var(--bg-hover)]/30 transition-colors"
                  >
                    {isExpanded ? <ChevronDown className="w-3 h-3 text-muted-foreground/40" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/40" />}
                    <FileCode2 className="w-3.5 h-3.5 text-yellow-400 flex-shrink-0" />
                    <span className="text-[11px] font-mono text-foreground/80 flex-1 truncate">{r.filename || r.url}</span>
                    {secretCount > 0 && (
                      <span className="text-[9px] px-1.5 py-0.5 rounded bg-red-500/10 text-red-400 flex-shrink-0">
                        {secretCount} secrets
                      </span>
                    )}
                    {endpointCount > 0 && (
                      <span className="text-[9px] px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 flex-shrink-0">
                        {endpointCount} endpoints
                      </span>
                    )}
                    {r.sourceMaps && (
                      <span className="text-[9px] px-1.5 py-0.5 rounded bg-yellow-500/10 text-yellow-400 flex-shrink-0">
                        source maps
                      </span>
                    )}
                  </button>
                  {isExpanded && (
                    <div className="border-t border-border/5 p-3 space-y-2">
                      <div className="grid grid-cols-3 gap-3 text-[10px]">
                        <div>
                          <span className="text-muted-foreground/40 block mb-1">URL</span>
                          <span className="font-mono text-foreground/60 text-[9px] break-all">{r.url}</span>
                        </div>
                        <div>
                          <span className="text-muted-foreground/40 block mb-1">Size</span>
                          <span className="text-foreground/60">{r.sizeBytes ? formatSize(r.sizeBytes) : "-"}</span>
                        </div>
                        <div>
                          <span className="text-muted-foreground/40 block mb-1">Risk Summary</span>
                          <span className="text-foreground/60">{r.riskSummary || "-"}</span>
                        </div>
                      </div>
                      {Array.isArray(r.frameworks) && r.frameworks.length > 0 && (
                        <div>
                          <span className="text-[9px] text-muted-foreground/40 block mb-1">Frameworks</span>
                          <div className="flex flex-wrap gap-1">
                            {r.frameworks.map((fw, i) => (
                              <span key={i} className="text-[9px] px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400">
                                {String(fw)}
                              </span>
                            ))}
                          </div>
                        </div>
                      )}
                      {Array.isArray(r.libraries) && r.libraries.length > 0 && (
                        <div>
                          <span className="text-[9px] text-muted-foreground/40 block mb-1">Libraries</span>
                          <div className="flex flex-wrap gap-1">
                            {r.libraries.map((lib, i) => (
                              <span key={i} className="text-[9px] px-1.5 py-0.5 rounded bg-cyan-500/10 text-cyan-400">
                                {String(lib)}
                              </span>
                            ))}
                          </div>
                        </div>
                      )}
                      {secretCount > 0 && (
                        <div>
                          <span className="text-[9px] text-red-400/70 block mb-1">Secrets Found</span>
                          <div className="space-y-0.5">
                            {r.secretsFound.map((s, i) => (
                              <div key={i} className="text-[9px] font-mono text-red-400/50 bg-red-500/5 px-2 py-1 rounded">{String(s)}</div>
                            ))}
                          </div>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

