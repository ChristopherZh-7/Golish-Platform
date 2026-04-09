import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  AlertTriangle,
  Bot,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  Loader2,
  Play,
  Radar,
  RefreshCw,
  Target,
  Terminal,
  Wrench,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { triggerAutoRecon } from "@/lib/ai";
import { logger } from "@/lib/logger";
import { useStore } from "@/store";
import { getProjectPath } from "@/lib/projects";

declare global {
  interface Window {
    __PENDING_RECON__?: {
      sessionId: string;
      targets: string[];
      projectName: string;
      projectPath: string;
    };
  }
}

type TargetStatus = "new" | "recon" | "recon_done" | "scanning" | "tested";

interface TargetEntry {
  id: string;
  name: string;
  type: string;
  value: string;
  status: TargetStatus;
  scope: string;
  source: string;
  ports: unknown[];
  technologies: string[];
  created_at: number;
  updated_at: number;
}

interface TargetStore {
  targets: TargetEntry[];
  groups: string[];
}

/* ── Activity types ────────────────────────────────────────────────── */

type ItemKind =
  | "tool_start" | "tool_done" | "tool_error"
  | "agent_thinking" | "agent_done"
  | "sub_agent_start" | "sub_agent_done"
  | "pipeline_start" | "pipeline_done" | "pipeline_error"
  | "info";

interface ActivityItem {
  id: string;
  kind: ItemKind;
  label: string;
  detail?: string;
  ts: number;
  durationMs?: number;
}

interface StepGroup {
  id: string;
  stepName: string;
  status: "pending" | "running" | "completed" | "failed";
  startTs: number;
  durationMs?: number;
  output?: string;
  children: ActivityItem[];
}

type FeedEntry =
  | { type: "item"; data: ActivityItem }
  | { type: "step"; data: StepGroup };

interface PipelineProgress {
  status: "running" | "completed" | "failed";
  totalSteps: number;
  completedSteps: number;
  currentStepIndex: number;
  currentStepName: string;
  stepNames: string[];
}

const TOOL_DISPLAY: Record<string, string> = {
  run_shell_command: "Shell",
  read_file: "Read File",
  write_file: "Write File",
  list_directory: "List Dir",
  web_search: "Web Search",
  web_fetch: "Web Fetch",
  nmap_scan: "Nmap",
  dns_lookup: "DNS Lookup",
  http_probe: "HTTP Probe",
  port_scan: "Port Scan",
  tech_fingerprint: "Fingerprint",
  whatweb: "WhatWeb",
  subfinder: "Subfinder",
  httpx: "HTTPX",
  initialize: "Initialize",
  tool_check: "Tool Check",
  tool_install: "Tool Install",
  brew_install_nmap: "Install nmap",
  brew_install_whatweb: "Install whatweb",
  brew_install_subfinder: "Install subfinder",
  brew_install_httpx: "Install httpx",
  summarize: "Summarize",
};

const STEP_DESCRIPTIONS: Record<string, string> = {
  initialize: "Setting up targets and preparing workspace",
  tool_check: "Checking which recon tools are installed",
  tool_install: "Installing missing tools via Homebrew",
  dns_lookup: "Resolving DNS records for targets",
  http_probe: "Probing HTTP/HTTPS services",
  port_scan: "Scanning for open ports (nmap)",
  tech_fingerprint: "Identifying web technologies",
  summarize: "Generating results summary",
};

function friendly(raw: string): string {
  return TOOL_DISPLAY[raw] ?? raw.replace(/_/g, " ");
}

function fmtDur(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
}

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function itemIcon(kind: ItemKind) {
  switch (kind) {
    case "tool_start": return <Wrench className="w-3 h-3 text-blue-400 animate-pulse" />;
    case "tool_done": return <CheckCircle2 className="w-3 h-3 text-green-400/70" />;
    case "tool_error": return <AlertTriangle className="w-3 h-3 text-red-400" />;
    case "agent_thinking": return <Bot className="w-3 h-3 text-purple-400 animate-pulse" />;
    case "agent_done": return <Bot className="w-3 h-3 text-green-400/70" />;
    case "sub_agent_start": return <Bot className="w-3 h-3 text-cyan-400 animate-pulse" />;
    case "sub_agent_done": return <Bot className="w-3 h-3 text-cyan-300/70" />;
    case "pipeline_start": return <Play className="w-3 h-3 text-blue-400" />;
    case "pipeline_done": return <CheckCircle2 className="w-3 h-3 text-green-400" />;
    case "pipeline_error": return <AlertTriangle className="w-3 h-3 text-red-400" />;
    case "info": return <Activity className="w-3 h-3 text-muted-foreground/40" />;
  }
}

function itemColor(kind: ItemKind): string {
  if (kind.endsWith("_start") || kind === "agent_thinking") return "text-blue-300";
  if (kind.endsWith("_done")) return "text-foreground/50";
  if (kind.endsWith("_error")) return "text-red-300";
  return "text-muted-foreground/40";
}

/* ── Pipeline progress indicator ───────────────────────────────────── */

function PipelineProgressBar({ progress }: { progress: PipelineProgress }) {
  return (
    <div className="flex-shrink-0 px-4 py-2.5 border-b border-border/10 bg-muted/5">
      <div className="flex items-center justify-between mb-2">
        <span className="text-[10px] font-medium text-muted-foreground/50 uppercase tracking-wider">
          Pipeline
        </span>
        <span className="text-[10px] text-muted-foreground/30">
          {progress.status === "running"
            ? `Step ${progress.currentStepIndex + 1} of ${progress.totalSteps}`
            : progress.status === "completed"
              ? `${progress.totalSteps} steps complete`
              : "Failed"}
        </span>
      </div>

      {/* Step bars */}
      <div className="flex items-center gap-1">
        {progress.stepNames.map((name, i) => {
          const isDone = i < progress.completedSteps;
          const isCurrent = i === progress.currentStepIndex && progress.status === "running";

          return (
            <div key={name} className="flex flex-col items-center gap-1 flex-1 min-w-0">
              <div className={cn(
                "w-full h-1.5 rounded-full transition-all duration-500",
                isDone && "bg-green-400/50",
                isCurrent && "bg-blue-400/60 animate-pulse",
                !isDone && !isCurrent && "bg-muted-foreground/10",
              )} />
              <span className={cn(
                "text-[9px] font-medium truncate max-w-full",
                isDone && "text-green-400/60",
                isCurrent && "text-blue-300",
                !isDone && !isCurrent && "text-muted-foreground/30",
              )}>
                {friendly(name)}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/* ── Render components ─────────────────────────────────────────────── */

function ItemRow({ item, indent = false }: { item: ActivityItem; indent?: boolean }) {
  const active = item.kind === "tool_start" || item.kind === "agent_thinking" || item.kind === "sub_agent_start";
  return (
    <div className={cn(
      "flex items-start gap-2 py-1 px-3 transition-colors",
      indent && "pl-8",
      active && "bg-blue-500/[0.03]",
    )}>
      <div className="mt-0.5 flex-shrink-0">{itemIcon(item.kind)}</div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-1.5">
          <span className={cn("text-[11px] font-medium truncate", itemColor(item.kind))}>
            {item.label}
          </span>
          {item.durationMs != null && (
            <span className="text-[9px] text-muted-foreground/20 flex items-center gap-0.5 flex-shrink-0">
              <Clock className="w-2 h-2" />
              {fmtDur(item.durationMs)}
            </span>
          )}
          <span className="text-[9px] text-muted-foreground/12 ml-auto flex-shrink-0">
            {fmtTime(item.ts)}
          </span>
        </div>
        {item.detail && (
          <p className="mt-0.5 text-[10px] text-muted-foreground/25 font-mono leading-relaxed truncate">
            {item.detail}
          </p>
        )}
      </div>
    </div>
  );
}

function StepRow({ step, defaultOpen, dynamicDesc }: { step: StepGroup; defaultOpen: boolean; dynamicDesc?: string }) {
  const [open, setOpen] = useState(defaultOpen);
  const { status, stepName, children, output, durationMs, startTs } = step;
  const desc = dynamicDesc || STEP_DESCRIPTIONS[stepName];
  const hasChildren = children.length > 0;
  const hasExpandable = hasChildren || (!!output && status === "completed");
  const isPending = status === "pending";

  return (
    <div className={cn(isPending && "opacity-40")}>
      <button
        type="button"
        onClick={() => hasExpandable && setOpen((v) => !v)}
        className={cn(
          "w-full flex items-start gap-2 py-2 px-3 text-left transition-colors",
          hasExpandable && "hover:bg-muted/10",
          status === "running" && "bg-blue-500/[0.04]",
        )}
      >
        <div className="mt-0.5 flex-shrink-0">
          {status === "pending" && <div className="w-3.5 h-3.5 rounded-full border border-muted-foreground/15" />}
          {status === "running" && <Loader2 className="w-3.5 h-3.5 text-blue-400 animate-spin" />}
          {status === "completed" && <CheckCircle2 className="w-3.5 h-3.5 text-green-400" />}
          {status === "failed" && <AlertTriangle className="w-3.5 h-3.5 text-red-400" />}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className={cn(
              "text-xs font-medium truncate",
              status === "pending" && "text-muted-foreground/30",
              status === "running" && "text-blue-300",
              status === "completed" && "text-foreground/60",
              status === "failed" && "text-red-300",
            )}>
              {friendly(stepName)}
            </span>
            {hasChildren && (
              <span className="text-[9px] text-muted-foreground/25">
                {children.length} op{children.length !== 1 ? "s" : ""}
              </span>
            )}
            {durationMs != null && (
              <span className="text-[9px] text-muted-foreground/25 flex items-center gap-0.5">
                <Clock className="w-2 h-2" />
                {fmtDur(durationMs)}
              </span>
            )}
          </div>
          {status === "pending" && desc && (
            <p className="mt-0.5 text-[10px] text-muted-foreground/20">{desc}</p>
          )}
          {status === "running" && desc && (
            <p className="mt-0.5 text-[10px] text-muted-foreground/40">{desc}</p>
          )}
          {!open && status === "completed" && output && (
            <p className="mt-0.5 text-[10px] text-muted-foreground/30 font-mono truncate">
              {output.slice(0, 120)}
            </p>
          )}
        </div>
        <div className="flex items-center gap-1.5 mt-0.5 flex-shrink-0">
          {!isPending && <span className="text-[9px] text-muted-foreground/15">{fmtTime(startTs)}</span>}
          {hasExpandable && (
            open ? (
              <ChevronDown className="w-3 h-3 text-muted-foreground/25" />
            ) : (
              <ChevronRight className="w-3 h-3 text-muted-foreground/25" />
            )
          )}
        </div>
      </button>
      {open && hasChildren && (
        <div className="border-l border-border/10 ml-5">
          {children.map((child) => (
            <ItemRow key={child.id} item={child} indent />
          ))}
        </div>
      )}
      {open && output && status === "completed" && (
        <pre className="ml-8 mr-3 mb-1 text-[10px] text-muted-foreground/30 font-mono leading-relaxed whitespace-pre-wrap break-all max-h-40 overflow-auto">
          {output.length > 800 ? `${output.slice(0, 800)}...` : output}
        </pre>
      )}
    </div>
  );
}

/* ── Main component ────────────────────────────────────────────────── */

const MAX_ENTRIES = 200;

const RECON_STEPS = ["initialize", "tool_check", "tool_install", "dns_lookup", "http_probe", "port_scan", "tech_fingerprint", "summarize"];

export function ProjectOverview({ sessionId }: { sessionId: string }) {
  const projectName = useStore((s) => s.currentProjectName);
  const projectPath = useStore((s) => s.currentProjectPath);
  const [targets, setTargets] = useState<TargetEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [reconRunning, setReconRunning] = useState(false);
  const [pipelineActive, setPipelineActive] = useState(false);
  const [feed, setFeed] = useState<FeedEntry[]>([]);
  const [progress, setProgress] = useState<PipelineProgress | null>(null);
  const feedEndRef = useRef<HTMLDivElement>(null);
  const seqRef = useRef(0);
  const activeStepRef = useRef<string | null>(null);

  const nextId = useCallback(() => `e-${++seqRef.current}`, []);

  const pushItem = useCallback((item: Omit<ActivityItem, "id">) => {
    const id = nextId();
    const entry: ActivityItem = { ...item, id };

    setFeed((prev) => {
      const activeStepId = activeStepRef.current;
      if (activeStepId) {
        return prev.map((e) => {
          if (e.type === "step" && e.data.id === activeStepId) {
            return { ...e, data: { ...e.data, children: [...e.data.children, entry] } };
          }
          return e;
        });
      }
      const next = [...prev, { type: "item" as const, data: entry }];
      return next.length > MAX_ENTRIES ? next.slice(-MAX_ENTRIES) : next;
    });
  }, [nextId]);

  const pushStep = useCallback((stepName: string, stepIndex: number, totalSteps: number) => {
    setFeed((prev) => {
      // Find existing pending step and activate it
      const idx = prev.findIndex(
        (e) => e.type === "step" && e.data.stepName === stepName && e.data.status === "pending",
      );
      if (idx >= 0) {
        const entry = prev[idx] as { type: "step"; data: StepGroup };
        activeStepRef.current = entry.data.id;
        const updated = [...prev];
        updated[idx] = { type: "step", data: { ...entry.data, status: "running", startTs: Date.now() } };
        return updated;
      }
      // Fallback: create new step if not pre-populated
      const id = nextId();
      activeStepRef.current = id;
      const step: StepGroup = { id, stepName, status: "running", startTs: Date.now(), children: [] };
      const next = [...prev, { type: "step" as const, data: step }];
      return next.length > MAX_ENTRIES ? next.slice(-MAX_ENTRIES) : next;
    });

    setProgress((prev) => {
      const stepNames = prev?.stepNames ?? RECON_STEPS.slice(0, totalSteps);
      return {
        status: "running",
        totalSteps,
        completedSteps: stepIndex,
        currentStepIndex: stepIndex,
        currentStepName: stepName,
        stepNames,
      };
    });
  }, [nextId]);

  const completeStep = useCallback((stepName: string, output?: string, durationMs?: number) => {
    activeStepRef.current = null;
    setFeed((prev) => prev.map((e) => {
      if (e.type === "step" && e.data.stepName === stepName && e.data.status === "running") {
        return { ...e, data: { ...e.data, status: "completed", output, durationMs } };
      }
      return e;
    }));
    setProgress((prev) => prev ? { ...prev, completedSteps: prev.completedSteps + 1 } : prev);
  }, []);

  const failStep = useCallback((stepName: string) => {
    activeStepRef.current = null;
    setFeed((prev) => prev.map((e) => {
      if (e.type === "step" && e.data.stepName === stepName && e.data.status === "running") {
        return { ...e, data: { ...e.data, status: "failed" } };
      }
      return e;
    }));
  }, []);

  useEffect(() => {
    feedEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [feed]);

  const fetchTargets = useCallback(async () => {
    try {
      const pp = getProjectPath();
      const data = await invoke<TargetStore>("target_list", { projectPath: pp });
      setTargets(data.targets);
    } catch (e) {
      logger.error("[ProjectOverview] fetchTargets failed:", e);
      setTargets([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const handleStartRecon = useCallback(async () => {
    if (!projectName || reconRunning) return;
    const targetValues = targets.map((t) => t.value);
    if (targetValues.length === 0) return;
    setReconRunning(true);
    try {
      await triggerAutoRecon(sessionId, targetValues, projectName, projectPath ?? "");
    } catch (e) {
      logger.error("Failed to run recon:", e);
    } finally {
      setReconRunning(false);
    }
  }, [sessionId, targets, projectName, projectPath, reconRunning]);

  useEffect(() => {
    let cancelled = false;
    const cleanups: (() => void)[] = [];

    fetchTargets();

    (async () => {
      const u1 = await listen("targets-changed", () => {
        if (!cancelled) fetchTargets();
      });
      if (cancelled) { u1(); return; }
      cleanups.push(u1);

      const u2 = await listen<Record<string, unknown>>("ai-event", (event) => {
        if (cancelled) return;
        const p = event.payload as Record<string, unknown>;
        const now = Date.now();

        switch (p.type) {
          case "started":
            pushItem({ kind: "agent_thinking", label: "AI is thinking...", ts: now });
            break;

          case "tool_request":
          case "tool_auto_approved": {
            const name = friendly(p.tool_name as string);
            let detail: string | undefined;
            if (p.args && typeof p.args === "object") {
              const args = p.args as Record<string, unknown>;
              const val = args.command ?? args.query ?? args.path ?? args.url ?? args.target;
              if (val) detail = String(val).slice(0, 120);
            }
            pushItem({ kind: "tool_start", label: name, detail, ts: now });
            break;
          }

          case "tool_result": {
            const name = friendly(p.tool_name as string);
            const ok = p.success as boolean;
            let detail: string | undefined;
            if (typeof p.result === "string") detail = (p.result as string).slice(0, 150);
            pushItem({ kind: ok ? "tool_done" : "tool_error", label: `${name} ${ok ? "done" : "failed"}`, detail, ts: now });
            break;
          }

          case "completed":
            pushItem({ kind: "agent_done", label: "AI turn done", ts: now, durationMs: p.duration_ms as number | undefined });
            break;

          case "error":
            pushItem({ kind: "tool_error", label: `Error: ${(p.message as string)?.slice(0, 80)}`, ts: now });
            break;

          case "sub_agent_started":
            pushItem({ kind: "sub_agent_start", label: `Sub-agent: ${p.agent_name}`, detail: p.task as string, ts: now });
            break;

          case "sub_agent_completed":
            pushItem({ kind: "sub_agent_done", label: "Sub-agent done", ts: now, durationMs: p.duration_ms as number | undefined });
            break;

          case "workflow_started": {
            setPipelineActive(true);
            setProgress({
              status: "running",
              totalSteps: RECON_STEPS.length,
              completedSteps: 0,
              currentStepIndex: 0,
              currentStepName: RECON_STEPS[0],
              stepNames: [...RECON_STEPS],
            });
            // Pre-populate all steps as "pending" so user can see the full plan
            const pendingSteps: FeedEntry[] = RECON_STEPS.map((name) => ({
              type: "step" as const,
              data: {
                id: `step-${name}-${now}`,
                stepName: name,
                status: "pending" as const,
                startTs: now,
                children: [],
              },
            }));
            setFeed((prev) => [...prev, { type: "item", data: { id: nextId(), kind: "pipeline_start", label: `Pipeline: ${p.workflow_name ?? "Recon"}`, ts: now } }, ...pendingSteps]);
            break;
          }

          case "workflow_step_started":
            pushStep(
              p.step_name as string,
              (p.step_index as number) ?? 0,
              (p.total_steps as number) ?? RECON_STEPS.length,
            );
            break;

          case "workflow_step_completed":
            completeStep(
              p.step_name as string,
              (p.output as string | null) ?? undefined,
              p.duration_ms as number | undefined,
            );
            break;

          case "workflow_completed":
            setPipelineActive(false);
            setProgress((prev) => prev ? { ...prev, status: "completed", completedSteps: prev.totalSteps } : prev);
            pushItem({ kind: "pipeline_done", label: "Pipeline complete", ts: now, durationMs: p.total_duration_ms as number | undefined });
            fetchTargets();
            break;

          case "workflow_error":
          case "workflow_failed":
            setPipelineActive(false);
            setProgress((prev) => prev ? { ...prev, status: "failed" } : prev);
            if (activeStepRef.current) failStep((p.step_name as string) ?? "");
            pushItem({ kind: "pipeline_error", label: `Pipeline error: ${(p.error as string)?.slice(0, 80) ?? "unknown"}`, ts: now });
            break;

          case "server_tool_started":
            pushItem({ kind: "tool_start", label: friendly(p.tool_name as string), ts: now });
            break;

          case "web_search_result":
            pushItem({ kind: "tool_done", label: "Web search done", ts: now });
            break;

          case "web_fetch_result":
            pushItem({ kind: "tool_done", label: `Fetched: ${(p.url as string)?.slice(0, 80)}`, ts: now });
            break;
        }
      });
      if (cancelled) { u2(); return; }
      cleanups.push(u2);

      // Pick up pending recon from project creation (set by HomeView)
      const pending = window.__PENDING_RECON__;
      if (pending && !cancelled) {
        delete window.__PENDING_RECON__;
        triggerAutoRecon(pending.sessionId, pending.targets, pending.projectName, pending.projectPath)
          .catch((e) => logger.error("Failed to run pending recon:", e));
      }
    })();

    return () => {
      cancelled = true;
      cleanups.forEach((fn) => fn());
    };
  }, [fetchTargets, pushItem, pushStep, completeStep, failStep]);

  if (!projectName) return null;

  const hasTargets = targets.length > 0;
  const canScan = hasTargets && !pipelineActive && !reconRunning;
  const hasInterruptedScan = targets.some((t) => t.status === "recon") && !pipelineActive;
  const hasFeed = feed.length > 0;

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex-shrink-0 flex items-center justify-between px-4 py-2.5 border-b border-border/10">
        <div className="flex items-center gap-3 min-w-0">
          <Target className="w-4 h-4 text-accent/60 flex-shrink-0" />
          <h2 className="text-sm font-semibold text-foreground/90 truncate">{projectName}</h2>
          {hasTargets && (
            <span className="text-[10px] text-muted-foreground/30">
              {targets.length} target{targets.length !== 1 ? "s" : ""}
            </span>
          )}
        </div>
        {canScan && (
          <button
            type="button"
            onClick={handleStartRecon}
            className={cn(
              "flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] font-medium transition-colors",
              hasInterruptedScan
                ? "bg-yellow-500/10 text-yellow-300 hover:bg-yellow-500/20"
                : "bg-accent/10 text-accent hover:bg-accent/20",
            )}
          >
            {hasInterruptedScan ? (
              <><RefreshCw className="w-3 h-3" /> Restart</>
            ) : (
              <><Radar className="w-3 h-3" /> Start Recon</>
            )}
          </button>
        )}
      </div>

      {/* Pipeline progress */}
      {progress && <PipelineProgressBar progress={progress} />}

      {/* Feed */}
      <div className="flex-1 min-h-0 overflow-auto">
        {hasFeed ? (
          <div className="divide-y divide-border/5">
            {feed.map((entry) => {
              if (entry.type === "step") {
                let dynDesc: string | undefined;
                if (entry.data.stepName === "tool_install" && entry.data.status === "running") {
                  const tcStep = feed.find(
                    (e) => e.type === "step" && e.data.stepName === "tool_check" && e.data.output,
                  );
                  if (tcStep && tcStep.type === "step") {
                    const out = tcStep.data.output || "";
                    const allTools = ["nmap", "whatweb", "subfinder", "httpx"];
                    const match = out.match(/Tools available:\s*(.*)/i);
                    if (match) {
                      const available = match[1].split(",").map((s: string) => s.trim().toLowerCase());
                      const missing = allTools.filter((t) => !available.includes(t));
                      if (missing.length > 0) {
                        dynDesc = `Installing: ${missing.join(", ")}`;
                      }
                    }
                  }
                }
                return (
                  <StepRow key={entry.data.id} step={entry.data} defaultOpen={entry.data.status === "running"} dynamicDesc={dynDesc} />
                );
              }
              return <ItemRow key={entry.data.id} item={entry.data} />;
            })}
            <div ref={feedEndRef} />
          </div>
        ) : (
          <div className="h-full flex flex-col items-center justify-center gap-3 px-4">
            {loading ? (
              <Loader2 className="w-6 h-6 text-muted-foreground/20 animate-spin" />
            ) : hasTargets ? (
              <>
                <Radar className="w-8 h-8 text-muted-foreground/15" />
                <p className="text-xs text-muted-foreground/30 text-center">
                  Ready to scan. Click <span className="text-accent/60">Start Recon</span> or send a message to the AI.
                </p>
              </>
            ) : (
              <>
                <Terminal className="w-8 h-8 text-muted-foreground/15" />
                <p className="text-xs text-muted-foreground/30 text-center">Waiting for activity...</p>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
