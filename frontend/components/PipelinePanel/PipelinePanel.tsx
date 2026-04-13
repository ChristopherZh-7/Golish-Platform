import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowRight, Code2, Database, Download, GitBranch, Loader2, Plus, Save, Trash2, X,
  Shield, Globe, Server, Cpu, Wrench,
  AlertTriangle, CheckCircle2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import { scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";
import { checkReconTools, type ReconToolCheck } from "@/lib/ai";
import { useStore } from "@/store";

type ExecMode = "pipe" | "sequential" | "parallel" | "on_success" | "on_failure";
const EXEC_LABELS: Record<ExecMode, { label: string; op: string; color: string }> = {
  pipe: { label: "|", op: " | ", color: "text-blue-400" },
  sequential: { label: "&&", op: " && ", color: "text-green-400" },
  parallel: { label: "&", op: " & ", color: "text-yellow-400" },
  on_success: { label: "&&", op: " && ", color: "text-emerald-400" },
  on_failure: { label: "||", op: " || ", color: "text-red-400" },
};

const STEP_TYPE_META: Record<string, { icon: typeof Shield; color: string; label: string }> = {
  dns_lookup: { icon: Globe, color: "text-blue-400", label: "DNS Lookup" },
  subdomain_enum: { icon: Globe, color: "text-cyan-400", label: "Subdomain Enum" },
  http_probe: { icon: Globe, color: "text-green-400", label: "HTTP Probe" },
  port_scan: { icon: Server, color: "text-red-400", label: "Port Scan" },
  tech_fingerprint: { icon: Cpu, color: "text-purple-400", label: "Tech Fingerprint" },
  js_collect: { icon: Code2, color: "text-amber-400", label: "JS Collect" },
  js_harvest: { icon: Download, color: "text-amber-500", label: "JS Harvest (AI)" },
  shell_command: { icon: Wrench, color: "text-muted-foreground", label: "Shell" },
};

interface PipelineStep {
  id: string;
  step_type: string;
  tool_name: string;
  tool_id: string;
  command_template: string;
  args: string[];
  params: Record<string, unknown>;
  input_from: string | null;
  exec_mode: ExecMode;
  requires?: string | null;
  x: number;
  y: number;
}

interface PipelineConnection {
  from_step: string;
  to_step: string;
}

interface Pipeline {
  id: string;
  name: string;
  description: string;
  is_template: boolean;
  workflow_id?: string;
  steps: PipelineStep[];
  connections: PipelineConnection[];
  created_at: number;
  updated_at: number;
}

type ToolWithMeta = ToolConfig & { categoryName?: string; subcategoryName?: string };

function uuid() {
  return Math.random().toString(36).slice(2, 10);
}

export function PipelinePanel() {
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [pipelines, setPipelines] = useState<Pipeline[]>([]);
  const [active, setActive] = useState<Pipeline | null>(null);
  const [tools, setTools] = useState<ToolWithMeta[]>([]);
  const [loading, setLoading] = useState(true);
  const [dirty, setDirty] = useState(false);
  const [showToolPicker, setShowToolPicker] = useState(false);
  const [toolCheck, setToolCheck] = useState<ReconToolCheck | null>(null);
  // AI-driven pipeline execution progress (received via pipeline-event)
  const [aiRunning, setAiRunning] = useState(false);
  const [aiProgress, setAiProgress] = useState<{ step: number; total: number; tool: string } | null>(null);
  const [aiResult, setAiResult] = useState<{ total_stored: number; steps: { tool_name: string; stored: number; exit_code: number | null }[] } | null>(null);
  const canvasRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [pipelineList, toolList] = await Promise.all([
        invoke<Pipeline[]>("pipeline_list", { projectPath: getProjectPath() }),
        scanTools(),
      ]);
      setPipelines(Array.isArray(pipelineList) ? pipelineList : []);
      setTools((toolList?.tools || []).filter((t) =>
        t.ui === "cli" && t.installed
      ));
    } catch { /* ignore */ }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load, currentProjectPath]);

  useEffect(() => {
    if (active?.workflow_id === "recon_basic") {
      checkReconTools()
        .then(setToolCheck)
        .catch(() => setToolCheck(null));
    } else {
      setToolCheck(null);
    }
  }, [active?.id, active?.workflow_id]);

  const handleNew = useCallback(() => {
    setActive({
      id: "",
      name: "New Pipeline",
      description: "",
      is_template: false,
      steps: [],
      connections: [],
      created_at: 0,
      updated_at: 0,
    });
    setDirty(true);
  }, []);

  const handleSave = useCallback(async () => {
    if (!active) return;
    const id = await invoke<string>("pipeline_save", {
      pipeline: active,
      projectPath: getProjectPath(),
    });
    setActive((prev) => prev ? { ...prev, id } : null);
    setDirty(false);
    load();
  }, [active, load]);

  const handleDelete = useCallback(async (id: string) => {
    await invoke("pipeline_delete", { id, projectPath: getProjectPath() });
    if (active?.id === id) setActive(null);
    load();
  }, [active, load, pipelines]);

  const addStep = useCallback((tool: ToolWithMeta) => {
    if (!active) return;
    const stepCount = active.steps.length;
    const newStep: PipelineStep = {
      id: uuid(),
      step_type: "shell_command",
      tool_name: tool.name,
      tool_id: tool.id,
      command_template: tool.executable || tool.name,
      args: [],
      params: {},
      input_from: null,
      exec_mode: "pipe",
      requires: null,
      x: 40 + stepCount * 220,
      y: 80,
    };

    const newConnections = [...active.connections];
    if (active.steps.length > 0) {
      const lastStep = active.steps[active.steps.length - 1];
      newConnections.push({ from_step: lastStep.id, to_step: newStep.id });
    }

    setActive({
      ...active,
      steps: [...active.steps, newStep],
      connections: newConnections,
    });
    setDirty(true);
    setShowToolPicker(false);
  }, [active]);

  const cycleExecMode = useCallback((stepId: string) => {
    if (!active) return;
    const modes: ExecMode[] = ["pipe", "sequential", "parallel", "on_success", "on_failure"];
    setActive({
      ...active,
      steps: active.steps.map((s) => {
        if (s.id !== stepId) return s;
        const idx = modes.indexOf(s.exec_mode || "pipe");
        return { ...s, exec_mode: modes[(idx + 1) % modes.length] };
      }),
    });
    setDirty(true);
  }, [active]);

  const removeStep = useCallback((stepId: string) => {
    if (!active) return;
    setActive({
      ...active,
      steps: active.steps.filter((s) => s.id !== stepId),
      connections: active.connections.filter(
        (c) => c.from_step !== stepId && c.to_step !== stepId
      ),
    });
    setDirty(true);
  }, [active]);

  // Listen for AI-driven pipeline execution events
  useEffect(() => {
    const unlistenPromise = listen<{
      pipeline_id: string;
      step_index: number;
      total_steps: number;
      tool_name: string;
      status: string;
      store_stats?: { stored_count: number };
    }>("pipeline-event", (event) => {
      const p = event.payload;

      if (p.status === "running") {
        setAiRunning(true);
        setAiProgress({ step: p.step_index + 1, total: p.total_steps, tool: p.tool_name });
      } else if (p.status === "skipped") {
        // Step skipped due to target type mismatch — don't count as error
      } else if (p.status === "completed" || p.status === "error") {
        setAiResult((prev) => {
          const steps = prev?.steps ?? [];
          return {
            total_stored: (prev?.total_stored ?? 0) + (p.store_stats?.stored_count ?? 0),
            steps: [...steps, {
              tool_name: p.tool_name,
              stored: p.store_stats?.stored_count ?? 0,
              exit_code: p.status === "completed" ? 0 : -1,
            }],
          };
        });

        if (p.step_index + 1 >= p.total_steps) {
          setAiRunning(false);
          setAiProgress(null);
        }
      }
    });

    return () => { unlistenPromise.then((f) => f()); };
  }, []);

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
      </div>
    );
  }

  const isReconBasic = active?.workflow_id === "recon_basic";

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div className="flex-shrink-0 px-4 py-3 border-b border-border/30 flex items-center gap-3">
        <GitBranch className="w-4 h-4 text-accent" />
        <h2 className="text-sm font-semibold flex-1">Pipelines</h2>
        <button
          onClick={handleNew}
          className="flex items-center gap-1 px-2 py-1 text-[10px] rounded-md bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
        >
          <Plus className="w-3 h-3" />
          New
        </button>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* Sidebar - pipeline list */}
        <div className="w-[200px] flex-shrink-0 border-r border-border/20 overflow-y-auto">
          {pipelines.length === 0 && !active ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/30 px-4 text-center">
              <GitBranch className="w-6 h-6" />
              <p className="text-[10px]">No pipelines yet.</p>
            </div>
          ) : (
            <div className="py-1">
              {pipelines.map((p) => (
                <button
                  key={p.id}
                  onClick={() => { setActive(p); setDirty(false); }}
                  className={cn(
                    "w-full text-left px-3 py-2 text-[11px] transition-colors flex items-center gap-2 group",
                    active?.id === p.id
                      ? "bg-accent/10 text-accent"
                      : "text-muted-foreground/60 hover:bg-muted/10 hover:text-foreground"
                  )}
                >
                  {p.workflow_id ? (
                    <Shield className="w-3 h-3 flex-shrink-0 text-emerald-400" />
                  ) : (
                    <GitBranch className="w-3 h-3 flex-shrink-0" />
                  )}
                  <span className="flex-1 truncate">{p.name}</span>
                  <span className="text-[9px] text-muted-foreground/30">{p.steps.length}</span>
                  <span
                    role="button"
                    tabIndex={0}
                    onClick={(e) => { e.stopPropagation(); handleDelete(p.id); }}
                    onKeyDown={(e) => { if (e.key === "Enter") { e.stopPropagation(); handleDelete(p.id); } }}
                    className="p-0.5 opacity-0 group-hover:opacity-100 text-muted-foreground/30 hover:text-red-400 transition-all cursor-pointer"
                  >
                    <Trash2 className="w-3 h-3" />
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Main canvas */}
        <div className="flex-1 flex flex-col min-w-0">
          {active ? (
            <>
              {/* Pipeline header */}
              <div className="flex-shrink-0 px-4 py-2 border-b border-border/20 flex items-center gap-3">
                <input
                  value={active.name}
                  onChange={(e) => { setActive({ ...active, name: e.target.value }); setDirty(true); }}
                  className="text-sm font-medium bg-transparent outline-none flex-1 text-foreground"
                  placeholder="Pipeline name"
                />
                {isReconBasic && (
                  <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 flex-shrink-0">
                    Recon
                  </span>
                )}
                <button
                  onClick={() => setShowToolPicker(!showToolPicker)}
                  className="flex items-center gap-1 px-2 py-1 text-[10px] rounded-md border border-border/20 text-muted-foreground/50 hover:text-foreground hover:border-border/40 transition-colors"
                >
                  <Plus className="w-3 h-3" />
                  Add Step
                </button>
                {aiRunning && (
                  <span className="flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md bg-emerald-500/10 text-emerald-400">
                    <Loader2 className="w-3 h-3 animate-spin" />
                    {aiProgress ? `AI: ${aiProgress.step}/${aiProgress.total} ${aiProgress.tool}` : "AI running..."}
                  </span>
                )}
                <button
                  onClick={handleSave}
                  disabled={!dirty}
                  className={cn(
                    "flex items-center gap-1 px-2 py-1 text-[10px] rounded-md transition-colors",
                    dirty
                      ? "bg-accent/15 text-accent hover:bg-accent/25"
                      : "bg-muted/10 text-muted-foreground/30 cursor-not-allowed"
                  )}
                >
                  <Save className="w-3 h-3" />
                  Save
                </button>
              </div>

              {/* Description */}
              {active.description && (
                <div className="flex-shrink-0 px-4 py-1.5 border-b border-border/10 text-[10px] text-muted-foreground/50">
                  {active.description}
                </div>
              )}

              {/* Tool status for recon_basic */}
              {isReconBasic && toolCheck && (
                <div className={cn(
                  "flex-shrink-0 px-4 py-2.5 border-b flex flex-col gap-2",
                  toolCheck.all_ready
                    ? "border-emerald-500/10 bg-emerald-500/5"
                    : "border-amber-500/15 bg-amber-500/5"
                )}>
                  <div className="flex items-center gap-2">
                    {toolCheck.all_ready ? (
                      <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />
                    ) : (
                      <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />
                    )}
                    <span className={cn(
                      "text-[11px] font-medium",
                      toolCheck.all_ready ? "text-emerald-400" : "text-amber-400"
                    )}>
                      {toolCheck.all_ready
                        ? "All tools ready"
                        : `${toolCheck.missing.length} tool${toolCheck.missing.length > 1 ? "s" : ""} missing`}
                    </span>
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {toolCheck.tools.map((t) => (
                      <span
                        key={t.name}
                        className={cn(
                          "inline-flex items-center gap-1 px-1.5 py-0.5 text-[10px] rounded-md border",
                          t.installed
                            ? "border-emerald-500/15 bg-emerald-500/5 text-emerald-400"
                            : "border-amber-500/20 bg-amber-500/5 text-amber-400"
                        )}
                      >
                        {t.installed ? (
                          <CheckCircle2 className="w-2.5 h-2.5" />
                        ) : (
                          <AlertTriangle className="w-2.5 h-2.5" />
                        )}
                        {t.name}
                      </span>
                    ))}
                  </div>
                  {!toolCheck.all_ready && (
                    <p className="text-[10px] text-amber-400/70">
                      Install missing tools in the Tool Manager before running this pipeline.
                    </p>
                  )}
                </div>
              )}

              {/* Tool picker dropdown */}
              {showToolPicker && (
                <div className="flex-shrink-0 px-4 py-2 border-b border-border/20 bg-muted/5">
                  <div className="flex flex-wrap gap-1.5 max-h-[120px] overflow-y-auto">
                    {tools.map((tool) => (
                      <button
                        key={tool.id}
                        onClick={() => addStep(tool)}
                        className="flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border border-border/15 bg-card hover:bg-muted/20 hover:border-accent/30 transition-colors"
                      >
                        {tool.icon && <span className="text-[10px]">{tool.icon}</span>}
                        <span>{tool.name}</span>
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* Pipeline visualization */}
              <div ref={canvasRef} className="flex-1 overflow-auto p-4">
                {active.steps.length === 0 ? (
                  <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/30">
                    <GitBranch className="w-8 h-8" />
                    <p className="text-[11px]">Add tools to build your pipeline</p>
                  </div>
                ) : (
                  <div className="flex items-start gap-3 min-w-max">
                    {active.steps.map((step, idx) => {
                      const hasConnection = idx > 0;
                      const mode = step.exec_mode || "sequential";
                      const modeInfo = EXEC_LABELS[mode as ExecMode] || EXEC_LABELS.sequential;
                      const meta = STEP_TYPE_META[step.step_type] || STEP_TYPE_META.shell_command;
                      const StepIcon = meta.icon;

                      return (
                        <div key={step.id} className="flex items-center gap-1">
                          {hasConnection && (
                            <button
                              onClick={() => cycleExecMode(step.id)}
                              title={`Click to cycle: ${Object.keys(EXEC_LABELS).join(" → ")}`}
                              className={cn(
                                "flex flex-col items-center gap-0.5 px-2 py-1 rounded-md transition-colors hover:bg-muted/20 cursor-pointer",
                                modeInfo.color,
                              )}
                            >
                              <ArrowRight className="w-4 h-4" />
                            </button>
                          )}
                          <div className="w-[200px] rounded-lg border border-border/20 bg-card overflow-hidden group">
                            <div className="flex items-center gap-2 px-3 py-2 border-b border-border/15 bg-muted/10">
                              <StepIcon className={cn("w-3.5 h-3.5", meta.color)} />
                              <input
                                value={step.tool_name}
                                onChange={(e) => {
                                  setActive({
                                    ...active,
                                    steps: active.steps.map((s) =>
                                      s.id === step.id ? { ...s, tool_name: e.target.value } : s
                                    ),
                                  });
                                  setDirty(true);
                                }}
                                className="text-[11px] font-medium flex-1 truncate bg-transparent outline-none text-foreground"
                              />
                              {step.requires && (
                                <span className="px-1 py-0.5 text-[8px] rounded bg-blue-500/10 text-blue-400 border border-blue-500/15">
                                  {step.requires}
                                </span>
                              )}
                              <button
                                onClick={() => removeStep(step.id)}
                                className="p-0.5 text-muted-foreground/30 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-all"
                              >
                                <X className="w-3 h-3" />
                              </button>
                            </div>
                            <div className="px-3 py-2 space-y-1.5">
                              <div className="text-[9px] text-muted-foreground/40">Command</div>
                              <input
                                value={step.command_template}
                                onChange={(e) => {
                                  setActive({
                                    ...active,
                                    steps: active.steps.map((s) =>
                                      s.id === step.id ? { ...s, command_template: e.target.value } : s
                                    ),
                                  });
                                  setDirty(true);
                                }}
                                className="w-full px-1.5 py-1 text-[10px] font-mono rounded bg-muted/10 border border-border/10 text-foreground outline-none"
                              />
                              <div className="text-[9px] text-muted-foreground/40">Arguments</div>
                              <input
                                value={step.args.join(" ")}
                                onChange={(e) => {
                                  const args = e.target.value.split(/\s+/).filter(Boolean);
                                  setActive({
                                    ...active,
                                    steps: active.steps.map((s) =>
                                      s.id === step.id ? { ...s, args } : s
                                    ),
                                  });
                                  setDirty(true);
                                }}
                                placeholder="e.g. -d {target} -silent"
                                className="w-full px-1.5 py-1 text-[10px] font-mono rounded bg-muted/10 border border-border/10 text-foreground placeholder:text-muted-foreground/20 outline-none"
                              />
                              <div className="flex gap-2">
                                <div className="flex-1">
                                  <div className="text-[9px] text-muted-foreground/40">Requires</div>
                                  <select
                                    value={step.requires || ""}
                                    onChange={(e) => {
                                      setActive({
                                        ...active,
                                        steps: active.steps.map((s) =>
                                          s.id === step.id ? { ...s, requires: e.target.value || null } : s
                                        ),
                                      });
                                      setDirty(true);
                                    }}
                                    className="w-full px-1 py-0.5 text-[10px] rounded bg-muted/10 border border-border/10 text-foreground outline-none"
                                  >
                                    <option value="">any</option>
                                    <option value="domain">domain</option>
                                    <option value="ip">ip</option>
                                    <option value="url">url</option>
                                  </select>
                                </div>
                                <div className="flex-1">
                                  <div className="text-[9px] text-muted-foreground/40">Input from</div>
                                  <select
                                    value={step.input_from || ""}
                                    onChange={(e) => {
                                      setActive({
                                        ...active,
                                        steps: active.steps.map((s) =>
                                          s.id === step.id ? { ...s, input_from: e.target.value || null } : s
                                        ),
                                      });
                                      setDirty(true);
                                    }}
                                    className="w-full px-1 py-0.5 text-[10px] rounded bg-muted/10 border border-border/10 text-foreground outline-none"
                                  >
                                    <option value="">prev step</option>
                                    {active.steps
                                      .filter((s) => s.id !== step.id)
                                      .map((s) => (
                                        <option key={s.id} value={s.id}>{s.tool_name}</option>
                                      ))}
                                  </select>
                                </div>
                              </div>
                            </div>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>

              {/* AI execution results (shown when AI runs this pipeline) */}
              {aiResult && (
                <div className="flex-shrink-0 px-4 py-3 border-t border-border/20 bg-muted/5">
                  <div className="flex items-center gap-2 mb-2">
                    <Database className="w-3.5 h-3.5 text-blue-400" />
                    <span className="text-[11px] font-medium text-foreground">
                      AI stored {aiResult.total_stored} records to database
                    </span>
                    <button
                      onClick={() => setAiResult(null)}
                      className="ml-auto p-0.5 text-muted-foreground/30 hover:text-foreground transition-colors"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {aiResult.steps.map((s, i) => (
                      <span
                        key={i}
                        className={cn(
                          "inline-flex items-center gap-1 px-1.5 py-0.5 text-[10px] rounded-md border",
                          s.exit_code === 0
                            ? "border-emerald-500/15 bg-emerald-500/5 text-emerald-400"
                            : "border-red-500/15 bg-red-500/5 text-red-400"
                        )}
                      >
                        {s.exit_code === 0 ? (
                          <CheckCircle2 className="w-2.5 h-2.5" />
                        ) : (
                          <AlertTriangle className="w-2.5 h-2.5" />
                        )}
                        {s.tool_name}
                        {s.stored > 0 && (
                          <span className="text-blue-400 ml-0.5">+{s.stored}</span>
                        )}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </>
          ) : (
            <div className="flex-1 flex flex-col items-center justify-center gap-3 text-muted-foreground/30">
              <GitBranch className="w-10 h-10" />
              <p className="text-sm">Select or create a pipeline</p>
              <p className="text-[10px]">Chain tools together or run built-in reconnaissance workflows</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
