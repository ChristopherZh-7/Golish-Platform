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
  type WhatWebOptions,
  type NucleiScanOptions,
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
  Settings2,
  Shield,
  ShieldAlert,
  Target,
  X,
} from "lucide-react";

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

  // Tool configurations
  const [wwOpts, setWwOpts] = useState<WhatWebOptions>({});
  const [nuOpts, setNuOpts] = useState<NucleiScanOptions>({});
  const [fxOpts, setFxOpts] = useState<FeroxScanOptions>({
    depth: 3,
    threads: 50,
    timeout: 10,
  });

  // UI toggles
  const [showFingerprints, setShowFingerprints] = useState(false);
  const [showPocs, setShowPocs] = useState(false);
  const [showDirResults, setShowDirResults] = useState(false);
  const [showWwConfig, setShowWwConfig] = useState(false);
  const [showNuConfig, setShowNuConfig] = useState(false);
  const [showFxConfig, setShowFxConfig] = useState(false);

  const mountedRef = useRef(true);
  useEffect(() => () => { mountedRef.current = false; }, []);

  useEffect(() => {
    const unlisten = listen<ToolScanProgress>("scan-progress", (e) => {
      if (mountedRef.current) setProgress(e.payload);
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  const projectPath = getProjectPath();

  // ── Step 1: WhatWeb ─────────────────────────────────────────────────

  const runWhatweb = useCallback(async () => {
    setWhatweb({ status: "running" });
    try {
      const opts = Object.keys(wwOpts).length > 0 ? wwOpts : undefined;
      const result = await scanWhatWeb(targetUrl, targetId, projectPath, opts);
      if (!mountedRef.current) return;
      setWhatweb({ status: "done", result });
      const fps = await fingerprintsList(targetId).catch(() => []);
      if (mountedRef.current) setFingerprints(Array.isArray(fps) ? fps : []);
    } catch (e) {
      if (mountedRef.current) setWhatweb({ status: "error", error: String(e) });
    }
  }, [targetUrl, targetId, projectPath, wwOpts]);

  // ── Step 2: PoC Matching ────────────────────────────────────────────

  const runPocMatch = useCallback(async () => {
    setPocs({ status: "running" });
    try {
      const matches = await matchPocsForTarget(targetId);
      if (!mountedRef.current) return;
      setPocMatches(matches);
      setPocs({
        status: "done",
        result: {
          tool: "poc-match", success: true,
          items_found: matches.length, items_stored: matches.length,
          errors: [], duration_ms: 0,
        },
      });
    } catch (e) {
      if (mountedRef.current) setPocs({ status: "error", error: String(e) });
    }
  }, [targetId]);

  // ── Step 3: Nuclei Targeted ─────────────────────────────────────────

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
      const opts = Object.keys(nuOpts).length > 0 ? nuOpts : undefined;
      const result = await scanNucleiTargeted(
        targetUrl, targetId, nucleiTemplateIds, projectPath, undefined, opts,
      );
      if (mountedRef.current) setNuclei({ status: "done", result });
    } catch (e) {
      if (mountedRef.current) setNuclei({ status: "error", error: String(e) });
    }
  }, [targetUrl, targetId, nucleiTemplateIds, projectPath, nuOpts]);

  // ── Step 4: feroxbuster ─────────────────────────────────────────────

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
      const result = await scanFeroxbuster(
        targetUrl, targetId, zapPaths, projectPath, fxOpts,
      );
      if (!mountedRef.current) return;
      setFerox({ status: "done", result });
      const entries = await listDirectoryEntries({ targetId }).catch(() => []);
      if (mountedRef.current) setDirEntries(Array.isArray(entries) ? entries : []);
    } catch (e) {
      if (mountedRef.current) setFerox({ status: "error", error: String(e) });
    }
  }, [targetUrl, targetId, zapPaths, projectPath, fxOpts]);

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

  // ── Full Workflow ───────────────────────────────────────────────────

  const runFullWorkflow = useCallback(async () => {
    // Step 1: WhatWeb
    setWhatweb({ status: "running" });
    try {
      const wOpts = Object.keys(wwOpts).length > 0 ? wwOpts : undefined;
      const wResult = await scanWhatWeb(targetUrl, targetId, projectPath, wOpts);
      if (!mountedRef.current) return;
      setWhatweb({ status: "done", result: wResult });
      const fps = await fingerprintsList(targetId).catch(() => []);
      if (!mountedRef.current) return;
      setFingerprints(Array.isArray(fps) ? fps : []);
    } catch (e) {
      if (!mountedRef.current) return;
      setWhatweb({ status: "error", error: String(e) });
      return;
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
          tool: "poc-match", success: true,
          items_found: matches.length, items_stored: matches.length,
          errors: [], duration_ms: 0,
        },
      });

      // Step 3: Nuclei
      const templateIds = matches.filter((p) => p.template_id).map((p) => p.template_id!);
      if (templateIds.length > 0) {
        setNuclei({ status: "running" });
        try {
          const nOpts = Object.keys(nuOpts).length > 0 ? nuOpts : undefined;
          const nResult = await scanNucleiTargeted(
            targetUrl, targetId, templateIds, projectPath, undefined, nOpts,
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
      if (!mountedRef.current) return;
      setPocs({ status: "error", error: String(e) });
      return;
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
        targetUrl, targetId, paths, projectPath, fxOpts,
      );
      if (!mountedRef.current) return;
      setFerox({ status: "done", result: fResult });
      const entries = await listDirectoryEntries({ targetId }).catch(() => []);
      if (mountedRef.current) setDirEntries(Array.isArray(entries) ? entries : []);
    } catch (e) {
      if (mountedRef.current) setFerox({ status: "error", error: String(e) });
    }
  }, [targetUrl, targetId, projectPath, wwOpts, nuOpts, fxOpts]);

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
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 w-full px-2.5 py-2 text-left hover:bg-muted/10 transition-colors"
      >
        {expanded ? <ChevronDown className="w-3 h-3 text-muted-foreground/30" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/30" />}
        <Crosshair className="w-3.5 h-3.5 text-accent" />
        <span className="text-[11px] font-medium text-foreground/80">Scan Workflow</span>
        {anyRunning && <Loader2 className="w-3 h-3 animate-spin text-accent ml-1" />}
        <div className="flex items-center gap-1 ml-auto">
          <StepDot status={whatweb.status} label="WW" />
          <StepDot status={pocs.status} label="PoC" />
          <StepDot status={nuclei.status} label="Nu" />
          <StepDot status={ferox.status} label="Fx" />
        </div>
      </button>

      {expanded && (
        <div className="border-t border-border/5 px-2.5 py-2 space-y-2">
          {/* Progress */}
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

          {/* Run All */}
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
              <><Loader2 className="w-3.5 h-3.5 animate-spin" /> Running...</>
            ) : (
              <><Play className="w-3.5 h-3.5" /> Run Full Workflow</>
            )}
          </button>

          {/* ── Step 1: WhatWeb ─────────────────────────────── */}
          <StepCard
            icon={<Scan className="w-3.5 h-3.5" />}
            title="1. WhatWeb Fingerprinting"
            step={whatweb}
            onRun={runWhatweb}
            disabled={anyRunning}
            showConfig={showWwConfig}
            onToggleConfig={() => setShowWwConfig(!showWwConfig)}
          >
            {showWwConfig && (
              <WhatWebConfig value={wwOpts} onChange={setWwOpts} />
            )}
            {fingerprints.length > 0 && (
              <Collapsible
                label={`${fingerprints.length} technologies detected`}
                open={showFingerprints}
                onToggle={() => setShowFingerprints(!showFingerprints)}
              >
                <div className="space-y-0.5">
                  {fingerprints.map((fp) => (
                    <div key={fp.id} className="flex items-center gap-2 text-[10px]">
                      <span className="px-1 py-0.5 rounded bg-purple-500/10 text-purple-400 text-[9px]">{fp.category}</span>
                      <span className="text-foreground/70">{fp.name}</span>
                      {fp.version && <span className="font-mono text-muted-foreground/40">{fp.version}</span>}
                      <div className="flex items-center gap-0.5 ml-auto">
                        <div className="w-6 h-1 rounded-full bg-muted/20 overflow-hidden">
                          <div
                            className={cn("h-full rounded-full", fp.confidence >= 80 ? "bg-green-500" : fp.confidence >= 50 ? "bg-yellow-500" : "bg-red-500")}
                            style={{ width: `${Math.min(100, fp.confidence)}%` }}
                          />
                        </div>
                        <span className="text-[8px] text-muted-foreground/30">{fp.confidence}%</span>
                      </div>
                    </div>
                  ))}
                </div>
              </Collapsible>
            )}
          </StepCard>

          {/* ── Step 2: PoC Matching ────────────────────────── */}
          <StepCard
            icon={<Shield className="w-3.5 h-3.5" />}
            title="2. PoC Matching"
            step={pocs}
            onRun={runPocMatch}
            disabled={anyRunning}
          >
            {pocMatches.length > 0 && (
              <Collapsible
                label={`${pocMatches.length} PoCs matched${nucleiTemplateIds.length > 0 ? ` (${nucleiTemplateIds.length} Nuclei templates)` : ""}`}
                open={showPocs}
                onToggle={() => setShowPocs(!showPocs)}
              >
                <div className="space-y-1">
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
                          <div key={p.poc_id} className={cn("flex items-center gap-2 text-[10px] px-1.5 py-0.5 rounded border ml-3", SEV_COLORS[sev] ?? SEV_COLORS.info)}>
                            <span className="font-mono text-[9px] truncate flex-1">{p.cve_id || p.poc_name}</span>
                            <span className="text-muted-foreground/30 text-[8px]">{p.source}</span>
                            {p.template_id && <Bug className="w-2.5 h-2.5 text-accent/50 flex-shrink-0" />}
                          </div>
                        ))}
                      </div>
                    );
                  })}
                </div>
              </Collapsible>
            )}
          </StepCard>

          {/* ── Step 3: Nuclei Targeted ─────────────────────── */}
          <StepCard
            icon={<Target className="w-3.5 h-3.5" />}
            title={`3. Nuclei Targeted${nucleiTemplateIds.length > 0 ? ` (${nucleiTemplateIds.length})` : ""}`}
            step={nuclei}
            onRun={runNuclei}
            disabled={anyRunning || nucleiTemplateIds.length === 0}
            showConfig={showNuConfig}
            onToggleConfig={() => setShowNuConfig(!showNuConfig)}
          >
            {showNuConfig && (
              <NucleiConfig value={nuOpts} onChange={setNuOpts} />
            )}
            {nuclei.result && nuclei.result.items_found > 0 && (
              <div className="flex items-center gap-2 text-[10px]">
                <ShieldAlert className="w-3 h-3 text-red-400" />
                <span className="text-red-400 font-medium">{nuclei.result.items_found} vulnerabilities confirmed</span>
              </div>
            )}
            {nuclei.result && nuclei.result.items_found === 0 && nuclei.status === "done" && (
              <div className="text-[10px] text-muted-foreground/40">No vulnerabilities found</div>
            )}
          </StepCard>

          {/* ── Step 4: feroxbuster ─────────────────────────── */}
          <StepCard
            icon={<FolderSearch className="w-3.5 h-3.5" />}
            title="4. feroxbuster"
            step={ferox}
            onRun={runFeroxbuster}
            disabled={anyRunning}
            showConfig={showFxConfig}
            onToggleConfig={() => setShowFxConfig(!showFxConfig)}
          >
            {showFxConfig && (
              <FeroxConfig value={fxOpts} onChange={setFxOpts} />
            )}
            {zapPaths.length > 0 && (
              <div className="text-[10px] text-muted-foreground/40 flex items-center gap-1">
                <ArrowRight className="w-2.5 h-2.5" />
                {zapPaths.length} ZAP-discovered base paths
              </div>
            )}
            {dirEntries.length > 0 && (
              <Collapsible
                label={`${dirEntries.length} paths discovered`}
                open={showDirResults}
                onToggle={() => setShowDirResults(!showDirResults)}
              >
                <div className="max-h-[200px] overflow-y-auto space-y-0.5">
                  {dirEntries.map((de) => <DirEntryRow key={de.id} entry={de} />)}
                </div>
              </Collapsible>
            )}
          </StepCard>
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Configuration panels
// ═══════════════════════════════════════════════════════════════════════

function ConfigField({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-0.5">
      <div className="flex items-center gap-2">
        <label className="text-[9px] font-medium text-muted-foreground/60 uppercase tracking-wide">{label}</label>
        {hint && <span className="text-[8px] text-muted-foreground/30">{hint}</span>}
      </div>
      {children}
    </div>
  );
}

const inputCls = "w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40 transition-colors text-foreground";
const selectCls = cn(inputCls, "appearance-none cursor-pointer");

function WhatWebConfig({
  value,
  onChange,
}: {
  value: WhatWebOptions;
  onChange: (v: WhatWebOptions) => void;
}) {
  return (
    <div className="space-y-1.5 p-2 rounded-md bg-background/30 border border-border/10">
      <div className="text-[9px] font-medium text-accent/60 flex items-center gap-1">
        <Settings2 className="w-2.5 h-2.5" /> WhatWeb Configuration
      </div>

      <ConfigField label="Aggression" hint="1=stealthy, 3=aggressive, 4=heavy">
        <select
          className={selectCls}
          value={value.aggression ?? ""}
          onChange={(e) => onChange({ ...value, aggression: e.target.value ? Number(e.target.value) : undefined })}
        >
          <option value="">Default (1)</option>
          <option value="1">1 — Stealthy</option>
          <option value="2">2 — Passive</option>
          <option value="3">3 — Aggressive</option>
          <option value="4">4 — Heavy</option>
        </select>
      </ConfigField>

      <ConfigField label="Plugins" hint="Comma-separated names">
        <input
          className={inputCls}
          placeholder="e.g. WordPress,Apache,PHP"
          value={value.plugins?.join(", ") ?? ""}
          onChange={(e) => {
            const v = e.target.value.trim();
            onChange({ ...value, plugins: v ? v.split(/\s*,\s*/).filter(Boolean) : undefined });
          }}
        />
      </ConfigField>

      <ConfigField label="User Agent">
        <input
          className={inputCls}
          placeholder="Custom user-agent string"
          value={value.user_agent ?? ""}
          onChange={(e) => onChange({ ...value, user_agent: e.target.value || undefined })}
        />
      </ConfigField>

      <ConfigField label="Proxy">
        <input
          className={inputCls}
          placeholder="http://127.0.0.1:8080"
          value={value.proxy ?? ""}
          onChange={(e) => onChange({ ...value, proxy: e.target.value || undefined })}
        />
      </ConfigField>

      <ConfigField label="Extra Args" hint="Space-separated CLI flags">
        <input
          className={inputCls}
          placeholder="e.g. --max-threads=25 --follow-redirect=always"
          value={value.extra_args?.join(" ") ?? ""}
          onChange={(e) => {
            const v = e.target.value.trim();
            onChange({ ...value, extra_args: v ? v.split(/\s+/) : undefined });
          }}
        />
      </ConfigField>
    </div>
  );
}

function NucleiConfig({
  value,
  onChange,
}: {
  value: NucleiScanOptions;
  onChange: (v: NucleiScanOptions) => void;
}) {
  return (
    <div className="space-y-1.5 p-2 rounded-md bg-background/30 border border-border/10">
      <div className="text-[9px] font-medium text-accent/60 flex items-center gap-1">
        <Settings2 className="w-2.5 h-2.5" /> Nuclei Configuration
      </div>

      <div className="grid grid-cols-3 gap-1.5">
        <ConfigField label="Rate Limit" hint="req/s">
          <input
            type="number"
            className={inputCls}
            placeholder="150"
            value={value.rate_limit ?? ""}
            onChange={(e) => onChange({ ...value, rate_limit: e.target.value ? Number(e.target.value) : undefined })}
          />
        </ConfigField>

        <ConfigField label="Bulk Size">
          <input
            type="number"
            className={inputCls}
            placeholder="25"
            value={value.bulk_size ?? ""}
            onChange={(e) => onChange({ ...value, bulk_size: e.target.value ? Number(e.target.value) : undefined })}
          />
        </ConfigField>

        <ConfigField label="Concurrency">
          <input
            type="number"
            className={inputCls}
            placeholder="25"
            value={value.concurrency ?? ""}
            onChange={(e) => onChange({ ...value, concurrency: e.target.value ? Number(e.target.value) : undefined })}
          />
        </ConfigField>
      </div>

      <ConfigField label="Tags" hint="Include templates with these tags">
        <input
          className={inputCls}
          placeholder="e.g. cve,rce,sqli,xss"
          value={value.tags?.join(", ") ?? ""}
          onChange={(e) => {
            const v = e.target.value.trim();
            onChange({ ...value, tags: v ? v.split(/\s*,\s*/).filter(Boolean) : undefined });
          }}
        />
      </ConfigField>

      <ConfigField label="Exclude Tags">
        <input
          className={inputCls}
          placeholder="e.g. dos,fuzz"
          value={value.exclude_tags?.join(", ") ?? ""}
          onChange={(e) => {
            const v = e.target.value.trim();
            onChange({ ...value, exclude_tags: v ? v.split(/\s*,\s*/).filter(Boolean) : undefined });
          }}
        />
      </ConfigField>

      <ConfigField label="Template Path" hint="Custom template directory">
        <input
          className={inputCls}
          placeholder="/path/to/custom-templates"
          value={value.template_path ?? ""}
          onChange={(e) => onChange({ ...value, template_path: e.target.value || undefined })}
        />
      </ConfigField>

      <div className="grid grid-cols-2 gap-1.5">
        <ConfigField label="Proxy">
          <input
            className={inputCls}
            placeholder="http://127.0.0.1:8080"
            value={value.proxy ?? ""}
            onChange={(e) => onChange({ ...value, proxy: e.target.value || undefined })}
          />
        </ConfigField>

        <ConfigField label="Timeout" hint="seconds">
          <input
            type="number"
            className={inputCls}
            placeholder="10"
            value={value.timeout ?? ""}
            onChange={(e) => onChange({ ...value, timeout: e.target.value ? Number(e.target.value) : undefined })}
          />
        </ConfigField>
      </div>

      <ConfigField label="Extra Args" hint="Space-separated CLI flags">
        <input
          className={inputCls}
          placeholder="e.g. -headless -system-resolvers"
          value={value.extra_args?.join(" ") ?? ""}
          onChange={(e) => {
            const v = e.target.value.trim();
            onChange({ ...value, extra_args: v ? v.split(/\s+/) : undefined });
          }}
        />
      </ConfigField>
    </div>
  );
}

function FeroxConfig({
  value,
  onChange,
}: {
  value: FeroxScanOptions;
  onChange: (v: FeroxScanOptions) => void;
}) {
  return (
    <div className="space-y-1.5 p-2 rounded-md bg-background/30 border border-border/10">
      <div className="text-[9px] font-medium text-accent/60 flex items-center gap-1">
        <Settings2 className="w-2.5 h-2.5" /> feroxbuster Configuration
      </div>

      <div className="grid grid-cols-3 gap-1.5">
        <ConfigField label="Depth">
          <input
            type="number"
            className={inputCls}
            placeholder="3"
            value={value.depth ?? ""}
            onChange={(e) => onChange({ ...value, depth: e.target.value ? Number(e.target.value) : undefined })}
          />
        </ConfigField>

        <ConfigField label="Threads">
          <input
            type="number"
            className={inputCls}
            placeholder="50"
            value={value.threads ?? ""}
            onChange={(e) => onChange({ ...value, threads: e.target.value ? Number(e.target.value) : undefined })}
          />
        </ConfigField>

        <ConfigField label="Timeout" hint="sec">
          <input
            type="number"
            className={inputCls}
            placeholder="10"
            value={value.timeout ?? ""}
            onChange={(e) => onChange({ ...value, timeout: e.target.value ? Number(e.target.value) : undefined })}
          />
        </ConfigField>
      </div>

      <ConfigField label="Wordlist" hint="Path to custom wordlist file">
        <input
          className={inputCls}
          placeholder="/usr/share/seclists/Discovery/Web-Content/common.txt"
          value={value.wordlist ?? ""}
          onChange={(e) => onChange({ ...value, wordlist: e.target.value || undefined })}
        />
      </ConfigField>

      <ConfigField label="Extensions" hint="Comma-separated file extensions">
        <input
          className={inputCls}
          placeholder="e.g. php,asp,aspx,jsp,html,js,json,bak,zip"
          value={value.extensions?.join(", ") ?? ""}
          onChange={(e) => {
            const v = e.target.value.trim();
            onChange({ ...value, extensions: v ? v.split(/\s*,\s*/).filter(Boolean) : undefined });
          }}
        />
      </ConfigField>

      <ConfigField label="Status Codes" hint="Comma-separated codes to match">
        <input
          className={inputCls}
          placeholder="e.g. 200,204,301,302,307,401,403"
          value={value.status_codes?.join(", ") ?? ""}
          onChange={(e) => {
            const v = e.target.value.trim();
            onChange({
              ...value,
              status_codes: v
                ? v.split(/\s*,\s*/).map(Number).filter((n) => !Number.isNaN(n))
                : undefined,
            });
          }}
        />
      </ConfigField>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Shared sub-components
// ═══════════════════════════════════════════════════════════════════════

function StepCard({
  icon,
  title,
  step,
  onRun,
  disabled,
  showConfig,
  onToggleConfig,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  step: StepState;
  onRun: () => void;
  disabled: boolean;
  showConfig?: boolean;
  onToggleConfig?: () => void;
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
        {onToggleConfig && (
          <button
            type="button"
            onClick={onToggleConfig}
            className={cn(
              "p-0.5 rounded transition-colors",
              showConfig
                ? "text-accent bg-accent/10"
                : "text-muted-foreground/20 hover:text-muted-foreground/50 hover:bg-muted/10",
            )}
            title="Configure"
          >
            <Settings2 className="w-3 h-3" />
          </button>
        )}
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
            {step.status === "done" || step.status === "error"
              ? <RefreshCw className="w-3 h-3" />
              : <Play className="w-3 h-3" />}
          </button>
        )}
        {step.status === "running" && <Loader2 className="w-3 h-3 animate-spin text-accent" />}
      </div>
      {step.error && (
        <div className="px-2 pb-1.5 text-[9px] text-red-400/70 flex items-start gap-1">
          <AlertTriangle className="w-2.5 h-2.5 flex-shrink-0 mt-0.5" />
          <span className="break-all">{step.error}</span>
        </div>
      )}
      {children && <div className="px-2 pb-1.5">{children}</div>}
    </div>
  );
}

function StepStatusBadge({ status, result }: { status: StepStatus; result?: ToolScanResult }) {
  if (status === "idle" || status === "running") return null;
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
          {result.duration_ms > 1000 ? `${(result.duration_ms / 1000).toFixed(1)}s` : `${result.duration_ms}ms`}
        </span>
      )}
    </span>
  );
}

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

function Collapsible({
  label,
  open,
  onToggle,
  children,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <div>
      <button
        type="button"
        onClick={onToggle}
        className="flex items-center gap-1 text-[10px] text-muted-foreground/50 hover:text-muted-foreground/70"
      >
        {open ? <ChevronDown className="w-2.5 h-2.5" /> : <ChevronRight className="w-2.5 h-2.5" />}
        {label}
      </button>
      {open && <div className="mt-1 pl-3">{children}</div>}
    </div>
  );
}

function DirEntryRow({ entry }: { entry: DirectoryEntry }) {
  const statusColor = entry.status_code != null
    ? entry.status_code < 300 ? "text-green-400"
      : entry.status_code < 400 ? "text-yellow-400"
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
          {entry.content_length > 1024 ? `${(entry.content_length / 1024).toFixed(1)}k` : `${entry.content_length}B`}
        </span>
      )}
      {entry.content_type && entry.content_type !== "unknown" && (
        <span className="text-muted-foreground/20 text-[8px] truncate max-w-[60px]">{entry.content_type}</span>
      )}
    </div>
  );
}
