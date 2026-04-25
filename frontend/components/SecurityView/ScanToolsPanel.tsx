import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Activity, ChevronDown, ChevronRight, Crosshair, History,
  Loader2, Play, Shield,
  ShieldAlert, ShieldCheck, ShieldX, XCircle,
} from "lucide-react";
import { formatDurationShort } from "@/lib/time";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";
import {
  oplogListByTarget, oplogSearch,
  type AuditRow,
} from "@/lib/security-analysis";
import {
  matchPocsForTarget, scanNucleiTargeted,
  type ToolScanResult, type PocMatch, type NucleiScanOptions,
} from "@/lib/pentest/scan-runner";
import { PipelineLauncher } from "@/components/TargetPanel/PipelineLauncher";
import { StyledSelect } from "./shared";

import { SEV_BADGE, SEV_DOT as _SEV_DOT } from "@/lib/severity";
const SEV_COLORS: Record<string, string> = {
  ...SEV_BADGE,
  info: "bg-zinc-500/10 text-zinc-400 border-zinc-500/20",
};
const SEV_DOT: Record<string, string> = { ..._SEV_DOT, info: "bg-zinc-500" };

function NucleiSection({ targetId, targetUrl }: { targetId: string; targetUrl: string }) {
  const [expanded, setExpanded] = useState(false);
  const [pocMatches, setPocMatches] = useState<PocMatch[]>([]);
  const [matchDone, setMatchDone] = useState(false);
  const [matching, setMatching] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [scanResult, setScanResult] = useState<ToolScanResult | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [showConfig, setShowConfig] = useState(false);
  const [opts, setOpts] = useState<NucleiScanOptions>({});
  const projectPath = getProjectPath();

  useEffect(() => {
    setPocMatches([]);
    setMatchDone(false);
    setScanResult(null);
    setScanError(null);
  }, [targetId]);

  const templateIds = useMemo(
    () => pocMatches.filter((p) => p.template_id).map((p) => p.template_id!),
    [pocMatches],
  );

  const sevGroups = useMemo(() => {
    const groups: Record<string, PocMatch[]> = {};
    for (const p of pocMatches) {
      const sev = (p.severity || "info").toLowerCase();
      (groups[sev] ??= []).push(p);
    }
    return groups;
  }, [pocMatches]);

  const handleMatchPocs = useCallback(async () => {
    setMatching(true);
    setScanError(null);
    try {
      const matches = await matchPocsForTarget(targetId);
      setPocMatches(matches);
      setMatchDone(true);
    } catch (e) {
      setScanError(String(e));
    } finally {
      setMatching(false);
    }
  }, [targetId]);

  const handleRunNuclei = useCallback(async () => {
    if (templateIds.length === 0) return;
    setScanning(true);
    setScanResult(null);
    setScanError(null);
    try {
      const result = await scanNucleiTargeted(
        targetUrl, targetId, templateIds, projectPath, undefined,
        Object.keys(opts).length > 0 ? opts : undefined,
      );
      setScanResult(result);
    } catch (e) {
      setScanError(String(e));
    } finally {
      setScanning(false);
    }
  }, [targetUrl, targetId, templateIds, projectPath, opts]);

  const handleRunAll = useCallback(async () => {
    setMatching(true);
    setScanResult(null);
    setScanError(null);
    try {
      const matches = await matchPocsForTarget(targetId);
      setPocMatches(matches);
      setMatchDone(true);

      const tids = matches.filter((p) => p.template_id).map((p) => p.template_id!);
      if (tids.length === 0) {
        setScanResult({ tool: "nuclei", success: true, items_found: 0, items_stored: 0, errors: [], duration_ms: 0 });
        return;
      }

      setMatching(false);
      setScanning(true);
      const result = await scanNucleiTargeted(
        targetUrl, targetId, tids, projectPath, undefined,
        Object.keys(opts).length > 0 ? opts : undefined,
      );
      setScanResult(result);
    } catch (e) {
      setScanError(String(e));
    } finally {
      setMatching(false);
      setScanning(false);
    }
  }, [targetId, targetUrl, projectPath, opts]);

  const handleCancel = useCallback(async () => {
    try { await invoke("nuclei_cancel"); } catch { /* ignore */ }
    setMatching(false);
    setScanning(false);
  }, []);

  const anyRunning = matching || scanning;

  return (
    <div className="rounded-lg border border-border/10 overflow-hidden">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 w-full px-3 py-2 text-left hover:bg-muted/10 transition-colors"
      >
        {expanded
          ? <ChevronDown className="w-3 h-3 text-muted-foreground/30" />
          : <ChevronRight className="w-3 h-3 text-muted-foreground/30" />}
        <ShieldAlert className="w-3.5 h-3.5 text-red-400/70" />
        <span className="text-[11px] font-medium text-foreground/80">Nuclei Vulnerability Scan</span>
        {anyRunning && <Loader2 className="w-3 h-3 animate-spin text-accent ml-1" />}
        {scanResult && !anyRunning && (
          <span className={cn(
            "ml-auto text-[9px] px-1.5 py-0.5 rounded",
            scanResult.items_found > 0 ? "bg-red-500/10 text-red-400" : "bg-green-500/10 text-green-400",
          )}>
            {scanResult.items_found > 0
              ? `${scanResult.items_found} vuln${scanResult.items_found !== 1 ? "s" : ""}`
              : "Clean"}
          </span>
        )}
        {pocMatches.length > 0 && !scanResult && !anyRunning && (
          <span className="ml-auto text-[9px] text-muted-foreground/40">
            {templateIds.length} templates ready
          </span>
        )}
        {matchDone && pocMatches.length === 0 && !scanResult && !anyRunning && (
          <span className="ml-auto text-[9px] text-muted-foreground/30">No matches</span>
        )}
      </button>

      {expanded && (
        <div className="border-t border-border/5 px-3 py-2.5 space-y-2.5">
          {/* Action buttons */}
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={anyRunning}
              onClick={handleRunAll}
              className={cn(
                "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-colors",
                anyRunning
                  ? "bg-muted/10 text-muted-foreground cursor-not-allowed"
                  : "bg-accent/10 text-accent hover:bg-accent/20 border border-accent/20",
              )}
            >
              {anyRunning
                ? <><Loader2 className="w-3 h-3 animate-spin" /> {matching ? "Matching PoCs..." : "Scanning..."}</>
                : <><Play className="w-3 h-3" /> Match & Scan</>}
            </button>

            <button
              type="button"
              disabled={anyRunning}
              onClick={handleMatchPocs}
              className="flex items-center gap-1 px-2 py-1.5 rounded-md text-[10px] text-muted-foreground/50 hover:text-foreground hover:bg-muted/10 transition-colors disabled:opacity-30"
            >
              <Shield className="w-3 h-3" /> Match Only
            </button>

            {templateIds.length > 0 && !anyRunning && (
              <button
                type="button"
                onClick={handleRunNuclei}
                className="flex items-center gap-1 px-2 py-1.5 rounded-md text-[10px] text-muted-foreground/50 hover:text-foreground hover:bg-muted/10 transition-colors"
              >
                <Crosshair className="w-3 h-3" /> Scan Only ({templateIds.length})
              </button>
            )}
            {anyRunning && (
              <button
                type="button"
                onClick={handleCancel}
                className="flex items-center gap-1 px-2 py-1.5 rounded-md text-[10px] text-red-400 hover:bg-red-500/10 transition-colors border border-red-500/20"
              >
                <XCircle className="w-3 h-3" /> Cancel
              </button>
            )}

            <button
              type="button"
              onClick={() => setShowConfig(!showConfig)}
              className={cn(
                "ml-auto p-1 rounded transition-colors",
                showConfig ? "text-accent bg-accent/10" : "text-muted-foreground/20 hover:text-muted-foreground/50",
              )}
              title="Configuration"
            >
              <Activity className="w-3.5 h-3.5" />
            </button>
          </div>

          {/* Configuration */}
          {showConfig && (
            <div className="space-y-1.5 p-2 rounded-md bg-background/30 border border-border/10">
              <div className="text-[9px] font-medium text-accent/60">Nuclei Configuration</div>
              <div className="grid grid-cols-3 gap-1.5">
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Rate (req/s)</label>
                  <input
                    type="number"
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="150"
                    value={opts.rate_limit ?? ""}
                    onChange={(e) => setOpts({ ...opts, rate_limit: e.target.value ? Number(e.target.value) : undefined })}
                  />
                </div>
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Bulk</label>
                  <input
                    type="number"
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="25"
                    value={opts.bulk_size ?? ""}
                    onChange={(e) => setOpts({ ...opts, bulk_size: e.target.value ? Number(e.target.value) : undefined })}
                  />
                </div>
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Concurrency</label>
                  <input
                    type="number"
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="25"
                    value={opts.concurrency ?? ""}
                    onChange={(e) => setOpts({ ...opts, concurrency: e.target.value ? Number(e.target.value) : undefined })}
                  />
                </div>
              </div>
              <div className="space-y-0.5">
                <label className="text-[9px] text-muted-foreground/50">Tags (include)</label>
                <input
                  className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                  placeholder="cve,rce,sqli,xss"
                  value={opts.tags?.join(", ") ?? ""}
                  onChange={(e) => {
                    const v = e.target.value.trim();
                    setOpts({ ...opts, tags: v ? v.split(/\s*,\s*/).filter(Boolean) : undefined });
                  }}
                />
              </div>
              <div className="space-y-0.5">
                <label className="text-[9px] text-muted-foreground/50">Exclude Tags</label>
                <input
                  className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                  placeholder="dos,fuzz"
                  value={opts.exclude_tags?.join(", ") ?? ""}
                  onChange={(e) => {
                    const v = e.target.value.trim();
                    setOpts({ ...opts, exclude_tags: v ? v.split(/\s*,\s*/).filter(Boolean) : undefined });
                  }}
                />
              </div>
              <div className="grid grid-cols-2 gap-1.5">
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Proxy</label>
                  <input
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="http://127.0.0.1:8080"
                    value={opts.proxy ?? ""}
                    onChange={(e) => setOpts({ ...opts, proxy: e.target.value || undefined })}
                  />
                </div>
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Timeout (s)</label>
                  <input
                    type="number"
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="10"
                    value={opts.timeout ?? ""}
                    onChange={(e) => setOpts({ ...opts, timeout: e.target.value ? Number(e.target.value) : undefined })}
                  />
                </div>
              </div>
              <div className="space-y-0.5">
                <label className="text-[9px] text-muted-foreground/50">Template Path</label>
                <input
                  className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                  placeholder="/path/to/custom-templates"
                  value={opts.template_path ?? ""}
                  onChange={(e) => setOpts({ ...opts, template_path: e.target.value || undefined })}
                />
              </div>
            </div>
          )}

          {/* Error */}
          {scanError && (
            <div className="flex items-start gap-1.5 text-[10px] text-red-400/70 p-1.5 rounded bg-red-500/5 border border-red-500/10">
              <ShieldX className="w-3 h-3 flex-shrink-0 mt-0.5" />
              <span className="break-all">{scanError}</span>
            </div>
          )}

          {/* No matches state */}
          {matchDone && pocMatches.length === 0 && !anyRunning && !scanError && (
            <div className="flex items-center gap-2 text-[10px] text-muted-foreground/40 p-2 rounded-md bg-muted/5 border border-border/10">
              <Shield className="w-3.5 h-3.5" />
              <div>
                <span className="text-foreground/50">No matching PoC templates found.</span>
                <span className="ml-1">Run the pipeline first to populate fingerprints, then try again.</span>
              </div>
            </div>
          )}

          {/* PoC Matches */}
          {pocMatches.length > 0 && (
            <div className="space-y-1.5">
              <div className="text-[10px] font-medium text-foreground/60 flex items-center gap-1.5">
                <Shield className="w-3 h-3 text-accent/50" />
                {pocMatches.length} PoCs matched
                {templateIds.length > 0 && (
                  <span className="text-muted-foreground/30">({templateIds.length} Nuclei templates)</span>
                )}
              </div>
              <div className="space-y-1 max-h-[200px] overflow-y-auto">
                {(["critical", "high", "medium", "low", "info"] as const).map((sev) => {
                  const group = sevGroups[sev];
                  if (!group?.length) return null;
                  return (
                    <div key={sev}>
                      <div className="flex items-center gap-1.5 mb-0.5">
                        <span className={cn("w-1.5 h-1.5 rounded-full", SEV_DOT[sev])} />
                        <span className="text-[9px] font-medium text-muted-foreground/50 uppercase">{sev} ({group.length})</span>
                      </div>
                      {group.map((p) => (
                        <div
                          key={p.poc_id}
                          className={cn("flex items-center gap-2 text-[10px] px-1.5 py-0.5 rounded border ml-3 mb-0.5", SEV_COLORS[sev] ?? SEV_COLORS.info)}
                        >
                          <span className="font-mono text-[9px] truncate flex-1">{p.cve_id || p.poc_name}</span>
                          <span className="text-muted-foreground/30 text-[8px]">{p.source}</span>
                          {p.template_id && <span title={`Template: ${p.template_id}`}><Crosshair className="w-2.5 h-2.5 text-accent/50 flex-shrink-0" /></span>}
                        </div>
                      ))}
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {/* Scan Results */}
          {scanResult && (
            <div className={cn(
              "p-2 rounded-md border",
              scanResult.items_found > 0 ? "bg-red-500/5 border-red-500/15" : "bg-green-500/5 border-green-500/15",
            )}>
              <div className="flex items-center gap-2 text-[10px]">
                {scanResult.items_found > 0 ? (
                  <>
                    <ShieldAlert className="w-3.5 h-3.5 text-red-400" />
                    <span className="text-red-400 font-medium">{scanResult.items_found} vulnerabilities confirmed</span>
                  </>
                ) : (
                  <>
                    <ShieldCheck className="w-3.5 h-3.5 text-green-400" />
                    <span className="text-green-400 font-medium">No vulnerabilities found</span>
                  </>
                )}
                {scanResult.duration_ms > 0 && (
                  <span className="text-muted-foreground/30 text-[9px] ml-auto">
                    {scanResult.duration_ms > 1000 ? `${(scanResult.duration_ms / 1000).toFixed(1)}s` : `${scanResult.duration_ms}ms`}
                  </span>
                )}
              </div>
              {scanResult.items_stored > 0 && scanResult.items_stored !== scanResult.items_found && (
                <div className="text-[9px] text-muted-foreground/40 mt-0.5">{scanResult.items_stored} new (stored)</div>
              )}
              {scanResult.errors.length > 0 && (
                <div className="text-[9px] text-red-400/50 mt-1">{scanResult.errors.length} error(s)</div>
              )}
            </div>
          )}

          {pocMatches.length === 0 && !scanResult && !anyRunning && !scanError && (
            <div className="text-[10px] text-muted-foreground/25 text-center py-2">
              Click "Match & Scan" to find and test vulnerabilities for this target
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Scan Tools Panel
// ═══════════════════════════════════════════════════════════════════════

interface TargetOption {
  id: string;
  value: string;
  type: string;
}

export function ScanToolsPanel({ initialTarget }: { initialTarget?: { id: string; value: string } }) {
  const [targets, setTargets] = useState<TargetOption[]>([]);
  const [selectedTarget, setSelectedTarget] = useState<TargetOption | null>(null);
  const [loading, setLoading] = useState(true);
  const projectPath = useStore((s) => s.currentProjectPath);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setSelectedTarget(null);
    (async () => {
      try {
        const data = await invoke<{ targets: TargetOption[] }>("target_list", {
          projectPath: getProjectPath(),
        });
        if (cancelled) return;
        const scannable = (data?.targets ?? []).filter(
          (t) => t.type === "url" || t.type === "domain" || t.type === "ip",
        );
        setTargets(scannable);
        const initial = initialTarget
          ? scannable.find((t) => t.id === initialTarget.id)
          : null;
        if (initial) {
          setSelectedTarget(initial);
        } else if (scannable.length > 0) {
          setSelectedTarget(scannable[0]);
        }
      } catch {
        if (!cancelled) setTargets([]);
      }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, [initialTarget?.id, projectPath]);

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-6 h-6 animate-spin text-muted-foreground/20" />
      </div>
    );
  }

  if (targets.length === 0) {
    return (
      <div className="h-full flex flex-col items-center justify-center gap-3 text-muted-foreground/20">
        <Crosshair className="w-12 h-12" />
        <p className="text-[12px] font-medium">No scannable targets</p>
        <p className="text-[10px] text-muted-foreground/15 max-w-[280px] text-center">
          Add URL or domain targets in the Targets panel first.
          Scan Tools supports WhatWeb fingerprinting, targeted Nuclei scanning, and feroxbuster directory brute-forcing.
        </p>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      {/* Target selector */}
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <Crosshair className="w-3.5 h-3.5 text-accent flex-shrink-0" />
        <span className="text-[10px] text-muted-foreground/50 flex-shrink-0">Target:</span>
        <StyledSelect
          value={selectedTarget?.id ?? ""}
          onChange={(v) => { const t = targets.find((t) => t.id === v); if (t) setSelectedTarget(t); }}
          options={targets.map((t) => ({ value: t.id, label: `[${t.type}] ${t.value}` }))}
          className="flex-1"
        />
        <span className="text-[9px] text-muted-foreground/30">
          {targets.length} target{targets.length !== 1 ? "s" : ""}
        </span>
      </div>

      {/* Pipeline launcher + Nuclei + scan history */}
      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4">
        {selectedTarget && (
          <>
            <PipelineLauncher
              key={selectedTarget.id}
              targetId={selectedTarget.id}
              targetValue={selectedTarget.value}
            />
            <NucleiSection
              targetId={selectedTarget.id}
              targetUrl={selectedTarget.value}
            />
            <ScanTimeline targetId={selectedTarget.id} targetValue={selectedTarget.value} />
          </>
        )}
      </div>
    </div>
  );
}

interface StepSummary {
  tool: string;
  stored: number;
  new?: number;
  parsed: number;
  exit: number | null;
  ms: number;
}

function ScanTimeline({ targetId, targetValue }: { targetId: string; targetValue: string }) {
  const [logs, setLogs] = useState<AuditRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedRun, setExpandedRun] = useState<number | null>(null);

  const loadLogs = useCallback(async () => {
    try {
      const [byTarget, bySearch] = await Promise.all([
        oplogListByTarget(targetId, 100).catch(() => [] as AuditRow[]),
        oplogSearch(getProjectPath() ?? "", targetValue, 200).catch(() => [] as AuditRow[]),
      ]);
      const seen = new Set<number>();
      const merged: AuditRow[] = [];
      for (const entry of [...byTarget, ...bySearch]) {
        if (entry.action !== "pipeline_executed" || seen.has(entry.id)) continue;
        const detail = entry.detail as Record<string, unknown> | null;
        const detailTarget = detail?.target as string | undefined;
        if (entry.targetId === targetId || detailTarget === targetValue) {
          seen.add(entry.id);
          merged.push(entry);
        }
      }
      merged.sort((a, b) => b.createdAt - a.createdAt);
      setLogs(merged);
    } catch {
      setLogs([]);
    }
    setLoading(false);
  }, [targetId, targetValue]);

  useEffect(() => {
    setLoading(true);
    loadLogs();
  }, [loadLogs]);

  useEffect(() => {
    const unlistenTargets = listen("targets-changed", () => loadLogs());
    return () => { unlistenTargets.then((fn) => fn()); };
  }, [loadLogs]);

  if (loading) {
    return (
      <div className="flex items-center gap-2 py-4 text-[10px] text-muted-foreground/30">
        <Loader2 className="w-3 h-3 animate-spin" />
        Loading scan history...
      </div>
    );
  }

  if (logs.length === 0) {
    return (
      <div className="text-center py-6 text-[10px] text-muted-foreground/20">
        No scan history for this target yet
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-border/10 overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/10">
        <History className="w-3.5 h-3.5 text-blue-400" />
        <span className="text-[11px] font-semibold text-foreground/80">Scan History</span>
        <span className="text-[10px] text-muted-foreground/30 ml-auto">{logs.length} run{logs.length !== 1 ? "s" : ""}</span>
      </div>

      <div className="divide-y divide-border/5">
        {logs.map((run, i) => {
          const detail = run.detail ?? {};
          const steps = (detail.steps ?? []) as StepSummary[];
          const totalStored = (detail.total_stored ?? 0) as number;
          const totalNew = (detail.total_new ?? null) as number | null;
          const durationMs = (detail.duration_ms ?? 0) as number;
          const completedSteps = (detail.completed_steps ?? 0) as number;
          const totalSteps = (detail.total_steps ?? 0) as number;
          const status = run.status;
          const isExpanded = expandedRun === run.id;

          return (
            <div key={run.id}>
              <button
                type="button"
                onClick={() => setExpandedRun(isExpanded ? null : run.id)}
                className="flex items-center gap-2.5 w-full px-3 py-2 text-left hover:bg-white/[0.02] transition-colors"
              >
                {/* Timeline dot */}
                <div className="flex flex-col items-center gap-0.5 w-3 flex-shrink-0">
                  <span className={cn(
                    "w-2 h-2 rounded-full flex-shrink-0",
                    status === "completed" ? "bg-emerald-400" : status === "partial" ? "bg-yellow-400" : "bg-red-400",
                  )} />
                  {i < logs.length - 1 && <div className="w-px flex-1 min-h-[8px] bg-border/10" />}
                </div>

                {/* Run info */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-[10px] font-mono text-muted-foreground/40">
                      {new Date(run.createdAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                      {" "}
                      {new Date(run.createdAt).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
                    </span>
                    <span className="text-[10px] text-foreground/60 font-medium">
                      {run.toolName ?? "Pipeline"}
                    </span>
                    {totalNew !== null ? (
                      totalNew > 0 ? (
                        <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-emerald-500/15 text-emerald-300 font-medium">
                          +{totalNew} new
                        </span>
                      ) : totalStored > 0 ? (
                        <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-white/[0.05] text-muted-foreground/40 font-medium">
                          no changes
                        </span>
                      ) : null
                    ) : totalStored > 0 ? (
                      <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-300 font-medium">
                        +{totalStored} items
                      </span>
                    ) : null}
                  </div>
                  <div className="flex items-center gap-2 mt-0.5 text-[9px] text-muted-foreground/30">
                    <span>{completedSteps}/{totalSteps} steps</span>
                    {durationMs > 0 && <span>{formatDurationShort(durationMs)}</span>}
                    {steps.filter((s) => s.stored > 0).map((s) => (
                      <span key={s.tool} className="text-[8px] px-1 py-0.5 rounded bg-white/[0.03]">
                        {s.tool}: {s.new != null ? s.new : s.stored}
                      </span>
                    ))}
                  </div>
                </div>

                {steps.length > 0 && (
                  isExpanded
                    ? <ChevronDown className="w-3 h-3 text-muted-foreground/20 flex-shrink-0" />
                    : <ChevronRight className="w-3 h-3 text-muted-foreground/20 flex-shrink-0" />
                )}
              </button>

              {/* Expanded step details */}
              {isExpanded && steps.length > 0 && (
                <div className="border-t border-border/5 px-3 py-2 ml-5 space-y-1">
                  {steps.map((step, si) => (
                    <div key={si} className="flex items-center gap-2 text-[10px]">
                      <span className={cn(
                        "w-1.5 h-1.5 rounded-full flex-shrink-0",
                        step.exit === 0 ? "bg-emerald-400/60" :
                        step.exit === null ? "bg-zinc-500/40" : "bg-red-400/60",
                      )} />
                      <span className="font-mono text-foreground/50 w-16 flex-shrink-0">{step.tool}</span>
                      {step.new != null && step.new > 0 ? (
                        <span className="text-emerald-300/60">+{step.new} new</span>
                      ) : step.stored > 0 ? (
                        <span className="text-blue-300/60">+{step.stored} stored</span>
                      ) : null}
                      {step.new != null && step.stored > step.new && step.new > 0 && (
                        <span className="text-muted-foreground/25">({step.stored - step.new} existing)</span>
                      )}
                      {step.parsed > 0 && step.parsed !== step.stored && (
                        <span className="text-muted-foreground/20">({step.parsed} parsed)</span>
                      )}
                      {step.ms > 0 && (
                        <span className="text-muted-foreground/20 ml-auto">
                          {formatDurationShort(step.ms)}
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Sensitive File Scanner Panel ──


