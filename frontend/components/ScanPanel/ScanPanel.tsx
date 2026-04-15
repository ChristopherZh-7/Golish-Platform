import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import {
  scanWhatWeb,
  matchPocsForTarget,
  scanNucleiTargeted,
  scanFeroxbuster,
  getZapDiscoveredPaths,
  type ToolScanResult,
  type ToolScanProgress,
  type PocMatch,
  type FeroxScanOptions,
} from "@/lib/pentest/scan-runner";
import { fingerprintsList, type Fingerprint } from "@/lib/security-analysis";
import { listDirectoryEntries, type DirectoryEntry } from "@/lib/pentest/api";
import {
  AlertTriangle,
  ArrowRight,
  Bug,
  Check,
  ChevronDown,
  ChevronRight,
  Crosshair,
  FolderSearch,
  Loader2,
  Play,
  RefreshCw,
  Scan,
  Shield,
  ShieldAlert,
  Target,
  X,
} from "lucide-react";

// ── Severity colors ───────────────────────────────────────────────────

const SEV_COLORS: Record<string, string> = {
  critical: "bg-red-500/15 text-red-400 border-red-500/20",
  high: "bg-orange-500/15 text-orange-400 border-orange-500/20",
  medium: "bg-yellow-500/15 text-yellow-400 border-yellow-500/20",
  low: "bg-blue-500/15 text-blue-400 border-blue-500/20",
  info: "bg-zinc-500/10 text-zinc-400 border-zinc-500/20",
};

const SEV_DOT: Record<string, string> = {
  critical: "bg-red-500",
  high: "bg-orange-500",
  medium: "bg-yellow-500",
  low: "bg-blue-500",
  info: "bg-zinc-500",
};

type StepStatus = "idle" | "running" | "done" | "error";

interface StepState {
  status: StepStatus;
  error?: string;
  result?: ToolScanResult;
}

interface ScanPanelProps {
  targetId: string;
  targetUrl: string;
}

export function ScanPanel({ targetId, targetUrl }: ScanPanelProps) {
  const [expanded, setExpanded] = useState(false);

  // Steps
  const [whatweb, setWhatweb] = useState<StepState>({ status: "idle" });
  const [pocs, setPocs] = useState<StepState>({ status: "idle" });
  const [nuclei, setNuclei] = useState<StepState>({ status: "idle" });
  const [ferox, setFerox] = useState<StepState>({ status: "idle" });

  // Data
  const [fingerprints, setFingerprints] = useState<Fingerprint[]>([]);
  const [pocMatches, setPocMatches] = useState<PocMatch[]>([]);
  const [zapPaths, setZapPaths] = useState<string[]>([]);
  const [dirEntries, setDirEntries] = useState<DirectoryEntry[]>([]);

  // Progress
  const [progress, setProgress] = useState<ToolScanProgress | null>(null);

  // Expanded sections
  const [showFingerprints, setShowFingerprints] = useState(false);
  const [showPocs, setShowPocs] = useState(false);
  const [showDirResults, setShowDirResults] = useState(false);

  const mountedRef = useRef(true);
  useEffect(() => () => { mountedRef.current = false; }, []);

  // Listen to scan-progress events
  useEffect(() => {
    const unlisten = listen<ToolScanProgress>("scan-progress", (e) => {
      if (mountedRef.current) setProgress(e.payload);
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  const projectPath = getProjectPath();

  // ── Step 1: WhatWeb Fingerprinting ────────────────────────────────

  const runWhatweb = useCallback(async () => {
    setWhatweb({ status: "running" });
    try {
      const result = await scanWhatWeb(targetUrl, targetId, projectPath);
      if (!mountedRef.current) return;
      setWhatweb({ status: "done", result });
      // Refresh fingerprints
      const fps = await fingerprintsList(targetId).catch(() => []);
      if (mountedRef.current) setFingerprints(Array.isArray(fps) ? fps : []);
    } catch (e) {
      if (mountedRef.current) setWhatweb({ status: "error", error: String(e) });
    }
  }, [targetUrl, targetId, projectPath]);

  // ── Step 2: PoC Matching ──────────────────────────────────────────

  const runPocMatch = useCallback(async () => {
    setPocs({ status: "running" });
    try {
      const matches = await matchPocsForTarget(targetId);
      if (!mountedRef.current) return;
      setPocMatches(matches);
      setPocs({
        status: "done",
        result: {
          tool: "poc-match",
          success: true,
          items_found: matches.length,
          items_stored: matches.length,
          errors: [],
          duration_ms: 0,
        },
      });
    } catch (e) {
      if (mountedRef.current) setPocs({ status: "error", error: String(e) });
    }
  }, [targetId]);

  // ── Step 3: Nuclei Targeted Scan ──────────────────────────────────

  const nucleiTemplateIds = useMemo(
    () => pocMatches.filter((p) => p.template_id).map((p) => p.template_id!),
    [pocMatches],
  );

  const runNuclei = useCallback(async () => {
    if (nucleiTemplateIds.length === 0) {
      setNuclei({ status: "error", error: "No Nuclei template IDs matched from PoCs" });
      return;
    }
    setNuclei({ status: "running" });
    try {
      const result = await scanNucleiTargeted(
        targetUrl,
        targetId,
        nucleiTemplateIds,
        projectPath,
      );
      if (mountedRef.current) setNuclei({ status: "done", result });
    } catch (e) {
      if (mountedRef.current) setNuclei({ status: "error", error: String(e) });
    }
  }, [targetUrl, targetId, nucleiTemplateIds, projectPath]);

  // ── Step 4: feroxbuster ───────────────────────────────────────────

  const loadZapPaths = useCallback(async () => {
    try {
      const host = new URL(targetUrl).host;
      const paths = await getZapDiscoveredPaths(host);
      if (mountedRef.current) setZapPaths(paths);
    } catch {
      if (mountedRef.current) setZapPaths([]);
    }
  }, [targetUrl]);

  const runFeroxbuster = useCallback(async () => {
    setFerox({ status: "running" });
    try {
      const opts: FeroxScanOptions = { depth: 3, threads: 50, timeout: 10 };
      const result = await scanFeroxbuster(
        targetUrl,
        targetId,
        zapPaths,
        projectPath,
        opts,
      );
      if (!mountedRef.current) return;
      setFerox({ status: "done", result });
      const entries = await listDirectoryEntries({ targetId }).catch(() => []);
      if (mountedRef.current) setDirEntries(Array.isArray(entries) ? entries : []);
    } catch (e) {
      if (mountedRef.current) setFerox({ status: "error", error: String(e) });
    }
  }, [targetUrl, targetId, zapPaths, projectPath]);

  // Load existing data on expand
  useEffect(() => {
    if (!expanded) return;
    let cancelled = false;
    (async () => {
      const [fps, entries] = await Promise.all([
        fingerprintsList(targetId).catch(() => []),
        listDirectoryEntries({ targetId }).catch(() => []),
      ]);
      if (cancelled) return;
      setFingerprints(Array.isArray(fps) ? fps : []);
      setDirEntries(Array.isArray(entries) ? entries : []);
      loadZapPaths();
    })();
    return () => { cancelled = true; };
  }, [expanded, targetId, loadZapPaths]);

  // ── Full Workflow ─────────────────────────────────────────────────

  const runFullWorkflow = useCallback(async () => {
    // Step 1: WhatWeb
    setWhatweb({ status: "running" });
    try {
      const wResult = await scanWhatWeb(targetUrl, targetId, projectPath);
      if (!mountedRef.current) return;
      setWhatweb({ status: "done", result: wResult });
      const fps = await fingerprintsList(targetId).catch(() => []);
      if (!mountedRef.current) return;
      setFingerprints(Array.isArray(fps) ? fps : []);
    } catch (e) {
      if (mountedRef.current) {
        setWhatweb({ status: "error", error: String(e) });
        return;
      }
    }

    // Step 2: PoC Match
    setPocs({ status: "running" });
    try {
      const matches = await matchPocsForTarget(targetId);
      if (!mountedRef.current) return;
      setPocMatches(matches);
      setPocs({
        status: "done",
        result: {
          tool: "poc-match",
          success: true,
          items_found: matches.length,
          items_stored: matches.length,
          errors: [],
          duration_ms: 0,
        },
      });

      // Step 3: Nuclei (only if we have template IDs)
      const templateIds = matches.filter((p) => p.template_id).map((p) => p.template_id!);
      if (templateIds.length > 0) {
        setNuclei({ status: "running" });
        try {
          const nResult = await scanNucleiTargeted(
            targetUrl,
            targetId,
            templateIds,
            projectPath,
          );
          if (!mountedRef.current) return;
          setNuclei({ status: "done", result: nResult });
        } catch (e) {
          if (mountedRef.current) setNuclei({ status: "error", error: String(e) });
        }
      } else {
        setNuclei({ status: "done", result: { tool: "nuclei", success: true, items_found: 0, items_stored: 0, errors: [], duration_ms: 0 } });
      }
    } catch (e) {
      if (mountedRef.current) {
        setPocs({ status: "error", error: String(e) });
        return;
      }
    }

    // Step 4: feroxbuster
    if (!mountedRef.current) return;
    setFerox({ status: "running" });
    try {
      let paths: string[] = [];
      try {
        const host = new URL(targetUrl).host;
        paths = await getZapDiscoveredPaths(host);
        if (mountedRef.current) setZapPaths(paths);
      } catch { /* use empty paths */ }

      const fResult = await scanFeroxbuster(
        targetUrl,
        targetId,
        paths,
        projectPath,
        { depth: 3, threads: 50, timeout: 10 },
      );
      if (!mountedRef.current) return;
      setFerox({ status: "done", result: fResult });
      const entries = await listDirectoryEntries({ targetId }).catch(() => []);
      if (mountedRef.current) setDirEntries(Array.isArray(entries) ? entries : []);
    } catch (e) {
      if (mountedRef.current) setFerox({ status: "error", error: String(e) });
    }
  }, [targetUrl, targetId, projectPath]);

  const anyRunning = whatweb.status === "running" || pocs.status === "running" ||
    nuclei.status === "running" || ferox.status === "running";

  const sevGroups = useMemo(() => {
    const groups: Record<string, PocMatch[]> = {};
    for (const p of pocMatches) {
      const sev = (p.severity || "info").toLowerCase();
      (groups[sev] ??= []).push(p);
    }
    return groups;
  }, [pocMatches]);

  return (
    <div className="rounded-lg border border-border/10 overflow-hidden">
      {/* Header */}
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 w-full px-2.5 py-2 text-left hover:bg-muted/10 transition-colors"
      >
        {expanded ? (
          <ChevronDown className="w-3 h-3 text-muted-foreground/30" />
        ) : (
          <ChevronRight className="w-3 h-3 text-muted-foreground/30" />
        )}
        <Crosshair className="w-3.5 h-3.5 text-accent" />
        <span className="text-[11px] font-medium text-foreground/80">
          Scan Workflow
        </span>
        {anyRunning && (
          <Loader2 className="w-3 h-3 animate-spin text-accent ml-1" />
        )}
        <div className="flex items-center gap-1 ml-auto">
          <StepDot status={whatweb.status} label="WW" />
          <StepDot status={pocs.status} label="PoC" />
          <StepDot status={nuclei.status} label="Nu" />
          <StepDot status={ferox.status} label="Fx" />
        </div>
      </button>

      {expanded && (
        <div className="border-t border-border/5 px-2.5 py-2 space-y-2">
          {/* Progress bar */}
          {progress && anyRunning && (
            <div className="space-y-0.5">
              <div className="flex items-center justify-between text-[9px] text-muted-foreground/50">
                <span className="font-mono">{progress.tool}: {progress.phase}</span>
                <span>{progress.current}/{progress.total}</span>
              </div>
              <div className="h-1 rounded-full bg-muted/10 overflow-hidden">
                <div
                  className="h-full rounded-full bg-accent/60 transition-all"
                  style={{ width: `${progress.total > 0 ? (progress.current / progress.total) * 100 : 0}%` }}
                />
              </div>
              <div className="text-[9px] text-muted-foreground/30 truncate">{progress.message}</div>
            </div>
          )}

          {/* Run All button */}
          <button
            type="button"
            disabled={anyRunning}
            onClick={runFullWorkflow}
            className={cn(
              "w-full flex items-center justify-center gap-2 px-3 py-1.5 rounded-md text-xs font-medium transition-colors",
              anyRunning
                ? "bg-muted/10 text-muted-foreground cursor-not-allowed"
                : "bg-accent/10 text-accent hover:bg-accent/20 border border-accent/20",
            )}
          >
            {anyRunning ? (
              <>
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                Running...
              </>
            ) : (
              <>
                <Play className="w-3.5 h-3.5" />
                Run Full Workflow
              </>
            )}
          </button>

          {/* ── Step 1: WhatWeb ───────────────────────────────── */}
          <StepCard
            icon={<Scan className="w-3.5 h-3.5" />}
            title="1. WhatWeb Fingerprinting"
            step={whatweb}
            onRun={runWhatweb}
            disabled={anyRunning}
          >
            {fingerprints.length > 0 && (
              <div>
                <button
                  type="button"
                  onClick={() => setShowFingerprints(!showFingerprints)}
                  className="flex items-center gap-1 text-[10px] text-muted-foreground/50 hover:text-muted-foreground/70"
                >
                  {showFingerprints ? <ChevronDown className="w-2.5 h-2.5" /> : <ChevronRight className="w-2.5 h-2.5" />}
                  {fingerprints.length} technologies detected
                </button>
                {showFingerprints && (
                  <div className="mt-1 space-y-0.5 pl-3">
                    {fingerprints.map((fp) => (
                      <div key={fp.id} className="flex items-center gap-2 text-[10px]">
                        <span className="px-1 py-0.5 rounded bg-purple-500/10 text-purple-400 text-[9px]">
                          {fp.category}
                        </span>
                        <span className="text-foreground/70">{fp.name}</span>
                        {fp.version && (
                          <span className="font-mono text-muted-foreground/40">{fp.version}</span>
                        )}
                        <div className="flex items-center gap-0.5 ml-auto">
                          <div className="w-6 h-1 rounded-full bg-muted/20 overflow-hidden">
                            <div
                              className={cn(
                                "h-full rounded-full",
                                fp.confidence >= 80 ? "bg-green-500" : fp.confidence >= 50 ? "bg-yellow-500" : "bg-red-500",
                              )}
                              style={{ width: `${Math.min(100, fp.confidence)}%` }}
                            />
                          </div>
                          <span className="text-[8px] text-muted-foreground/30">{fp.confidence}%</span>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </StepCard>

          {/* ── Step 2: PoC Matching ─────────────────────────── */}
          <StepCard
            icon={<Shield className="w-3.5 h-3.5" />}
            title="2. PoC Matching"
            step={pocs}
            onRun={runPocMatch}
            disabled={anyRunning}
          >
            {pocMatches.length > 0 && (
              <div>
                <button
                  type="button"
                  onClick={() => setShowPocs(!showPocs)}
                  className="flex items-center gap-1 text-[10px] text-muted-foreground/50 hover:text-muted-foreground/70"
                >
                  {showPocs ? <ChevronDown className="w-2.5 h-2.5" /> : <ChevronRight className="w-2.5 h-2.5" />}
                  {pocMatches.length} PoCs matched
                  {nucleiTemplateIds.length > 0 && (
                    <span className="text-accent/50 ml-1">
                      ({nucleiTemplateIds.length} Nuclei templates)
                    </span>
                  )}
                </button>
                {showPocs && (
                  <div className="mt-1 space-y-1 pl-3">
                    {(["critical", "high", "medium", "low", "info"] as const).map((sev) => {
                      const group = sevGroups[sev];
                      if (!group?.length) return null;
                      return (
                        <div key={sev}>
                          <div className="flex items-center gap-1.5 mb-0.5">
                            <span className={cn("w-1.5 h-1.5 rounded-full", SEV_DOT[sev])} />
                            <span className="text-[9px] font-medium text-muted-foreground/50 uppercase">
                              {sev} ({group.length})
                            </span>
                          </div>
                          {group.map((p) => (
                            <div
                              key={p.poc_id}
                              className={cn(
                                "flex items-center gap-2 text-[10px] px-1.5 py-0.5 rounded border ml-3",
                                SEV_COLORS[sev] ?? SEV_COLORS.info,
                              )}
                            >
                              <span className="font-mono text-[9px] truncate flex-1">{p.cve_id || p.poc_name}</span>
                              <span className="text-muted-foreground/30 text-[8px]">{p.source}</span>
                              {p.template_id && (
                                <Bug className="w-2.5 h-2.5 text-accent/50 flex-shrink-0" />
                              )}
                            </div>
                          ))}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            )}
          </StepCard>

          {/* ── Step 3: Nuclei Targeted ──────────────────────── */}
          <StepCard
            icon={<Target className="w-3.5 h-3.5" />}
            title={`3. Nuclei Targeted Scan${nucleiTemplateIds.length > 0 ? ` (${nucleiTemplateIds.length} templates)` : ""}`}
            step={nuclei}
            onRun={runNuclei}
            disabled={anyRunning || nucleiTemplateIds.length === 0}
          >
            {nuclei.result && nuclei.result.items_found > 0 && (
              <div className="flex items-center gap-2 text-[10px]">
                <ShieldAlert className="w-3 h-3 text-red-400" />
                <span className="text-red-400 font-medium">
                  {nuclei.result.items_found} vulnerabilities confirmed
                </span>
              </div>
            )}
            {nuclei.result && nuclei.result.items_found === 0 && nuclei.status === "done" && (
              <div className="text-[10px] text-muted-foreground/40">
                No vulnerabilities found with matched templates
              </div>
            )}
          </StepCard>

          {/* ── Step 4: feroxbuster ──────────────────────────── */}
          <StepCard
            icon={<FolderSearch className="w-3.5 h-3.5" />}
            title="4. Directory Brute-Force (feroxbuster)"
            step={ferox}
            onRun={runFeroxbuster}
            disabled={anyRunning}
          >
            {zapPaths.length > 0 && (
              <div className="text-[10px] text-muted-foreground/40 flex items-center gap-1">
                <ArrowRight className="w-2.5 h-2.5" />
                {zapPaths.length} ZAP-discovered base paths
              </div>
            )}
            {dirEntries.length > 0 && (
              <div>
                <button
                  type="button"
                  onClick={() => setShowDirResults(!showDirResults)}
                  className="flex items-center gap-1 text-[10px] text-muted-foreground/50 hover:text-muted-foreground/70"
                >
                  {showDirResults ? <ChevronDown className="w-2.5 h-2.5" /> : <ChevronRight className="w-2.5 h-2.5" />}
                  {dirEntries.length} paths discovered
                </button>
                {showDirResults && (
                  <div className="mt-1 max-h-[200px] overflow-y-auto space-y-0.5 pl-3">
                    {dirEntries.map((de) => (
                      <DirEntryRow key={de.id} entry={de} />
                    ))}
                  </div>
                )}
              </div>
            )}
          </StepCard>
        </div>
      )}
    </div>
  );
}

// ── Step card wrapper ─────────────────────────────────────────────────

function StepCard({
  icon,
  title,
  step,
  onRun,
  disabled,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  step: StepState;
  onRun: () => void;
  disabled: boolean;
  children?: React.ReactNode;
}) {
  return (
    <div className="rounded-md border border-border/10 bg-card/30 overflow-hidden">
      <div className="flex items-center gap-2 px-2 py-1.5">
        <span className={cn(
          "text-muted-foreground/50",
          step.status === "running" && "text-accent animate-pulse",
          step.status === "done" && "text-green-400",
          step.status === "error" && "text-red-400",
        )}>
          {icon}
        </span>
        <span className="text-[10px] font-medium text-foreground/70 flex-1">{title}</span>
        <StepStatusBadge status={step.status} result={step.result} />
        {step.status !== "running" && (
          <button
            type="button"
            onClick={onRun}
            disabled={disabled}
            className={cn(
              "p-0.5 rounded transition-colors",
              disabled
                ? "text-muted-foreground/10 cursor-not-allowed"
                : "text-muted-foreground/30 hover:text-accent hover:bg-accent/10",
            )}
          >
            {step.status === "done" || step.status === "error" ? (
              <RefreshCw className="w-3 h-3" />
            ) : (
              <Play className="w-3 h-3" />
            )}
          </button>
        )}
        {step.status === "running" && (
          <Loader2 className="w-3 h-3 animate-spin text-accent" />
        )}
      </div>
      {step.error && (
        <div className="px-2 pb-1.5 text-[9px] text-red-400/70 flex items-start gap-1">
          <AlertTriangle className="w-2.5 h-2.5 flex-shrink-0 mt-0.5" />
          <span className="break-all">{step.error}</span>
        </div>
      )}
      {children && (
        <div className="px-2 pb-1.5">
          {children}
        </div>
      )}
    </div>
  );
}

// ── Step status badge ─────────────────────────────────────────────────

function StepStatusBadge({ status, result }: { status: StepStatus; result?: ToolScanResult }) {
  if (status === "idle") return null;
  if (status === "running") return null;

  if (status === "error") {
    return (
      <span className="flex items-center gap-0.5 text-[8px] px-1 py-0.5 rounded bg-red-500/10 text-red-400">
        <X className="w-2 h-2" /> Error
      </span>
    );
  }

  if (!result) return null;

  return (
    <span className="flex items-center gap-0.5 text-[8px] px-1 py-0.5 rounded bg-green-500/10 text-green-400">
      <Check className="w-2 h-2" />
      {result.items_stored} found
      {result.duration_ms > 0 && (
        <span className="text-muted-foreground/30 ml-0.5">
          {result.duration_ms > 1000
            ? `${(result.duration_ms / 1000).toFixed(1)}s`
            : `${result.duration_ms}ms`}
        </span>
      )}
    </span>
  );
}

// ── Step dot (mini indicator) ─────────────────────────────────────────

function StepDot({ status, label }: { status: StepStatus; label: string }) {
  const color = {
    idle: "bg-zinc-600",
    running: "bg-accent animate-pulse",
    done: "bg-green-500",
    error: "bg-red-500",
  }[status];

  return (
    <div className="flex items-center gap-0.5" title={label}>
      <span className={cn("w-1.5 h-1.5 rounded-full", color)} />
      <span className="text-[7px] text-muted-foreground/25 font-mono">{label}</span>
    </div>
  );
}

// ── Directory entry row ───────────────────────────────────────────────

function DirEntryRow({ entry }: { entry: DirectoryEntry }) {
  const statusColor = entry.status_code != null
    ? entry.status_code < 300
      ? "text-green-400"
      : entry.status_code < 400
        ? "text-yellow-400"
        : "text-red-400"
    : "text-muted-foreground/30";

  return (
    <div className="flex items-center gap-2 text-[10px] py-0.5">
      <span className={cn("font-mono w-7 text-right flex-shrink-0", statusColor)}>
        {entry.status_code ?? "?"}
      </span>
      <span className="font-mono text-foreground/60 flex-1 truncate">{entry.url}</span>
      {entry.content_length != null && (
        <span className="text-muted-foreground/20 text-[8px] font-mono">
          {entry.content_length > 1024
            ? `${(entry.content_length / 1024).toFixed(1)}k`
            : `${entry.content_length}B`}
        </span>
      )}
      {entry.content_type && entry.content_type !== "unknown" && (
        <span className="text-muted-foreground/20 text-[8px] truncate max-w-[60px]">
          {entry.content_type}
        </span>
      )}
    </div>
  );
}
