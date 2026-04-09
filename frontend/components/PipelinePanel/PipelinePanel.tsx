import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowRight, GitBranch, Loader2, Play, Plus, Save, Trash2, X,
  Shield, Globe, Server, Cpu, Wrench,
  AlertTriangle, CheckCircle2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import { getProjectPath } from "@/lib/projects";
import { scanTools, type ToolConfig } from "@/lib/pentest/api";
import { useCreateTerminalTab } from "@/hooks/useCreateTerminalTab";
import { checkReconTools, type ReconToolCheck } from "@/lib/ai";

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
  const [running, setRunning] = useState(false);
  const [toolCheck, setToolCheck] = useState<ReconToolCheck | null>(null);
  const [checkingTools, setCheckingTools] = useState(false);
  const canvasRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [pipelineList, toolList] = await Promise.all([
        invoke<Pipeline[]>("pipeline_list", { projectPath: getProjectPath() }),
        scanTools(),
      ]);
      setPipelines(pipelineList);
      setTools((toolList.tools || []).filter((t) =>
        t.ui === "cli" && t.installed
      ));
    } catch { /* ignore */ }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load, currentProjectPath]);

  useEffect(() => {
    if (active?.workflow_id === "recon_basic") {
      setCheckingTools(true);
      checkReconTools()
        .then(setToolCheck)
        .catch(() => setToolCheck(null))
        .finally(() => setCheckingTools(false));
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

  const { createTerminalTab } = useCreateTerminalTab();

  const handleRun = useCallback(async () => {
    if (!active || active.steps.length === 0 || running) return;
    if (toolCheck && !toolCheck.all_ready) return;

    const store = useStore.getState();
    const targets = store.targets?.map((t: { value: string }) => t.value) || [];
    const targetStr = targets[0] || "";

    setRunning(true);
    try {
      let chainedCommand = "";
      active.steps.forEach((step, idx) => {
        const args = step.args.length > 0 ? ` ${step.args.join(" ")}` : "";
        let cmd = `${step.command_template}${args}`;
        cmd = cmd.replaceAll("{target}", targetStr);

        if (idx === 0 || !chainedCommand) {
          chainedCommand = cmd;
        } else {
          const mode = step.exec_mode || "sequential";
          chainedCommand += EXEC_LABELS[mode as ExecMode]?.op ?? " && ";
          chainedCommand += cmd;
        }
      });

      if (!chainedCommand) return;

      const sessionId = await createTerminalTab();
      if (sessionId) {
        store.setActiveSession(sessionId);
        setTimeout(async () => {
          await invoke("terminal_write", { sessionId, data: chainedCommand + "\n" }).catch(() => {});
        }, 300);
      }
    } finally {
      setRunning(false);
    }
  }, [active, createTerminalTab, running, toolCheck]);

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
      </div>
    );
  }

  const isReconBasic = active?.workflow_id === "recon_basic";
  const hasMissingTools = isReconBasic && toolCheck !== null && !toolCheck.all_ready;
  const runDisabled = !active || active.steps.length === 0 || running || hasMissingTools || checkingTools;

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
                  <button
                    onClick={(e) => { e.stopPropagation(); handleDelete(p.id); }}
                    className="p-0.5 opacity-0 group-hover:opacity-100 text-muted-foreground/30 hover:text-red-400 transition-all"
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
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
                <button
                  onClick={handleRun}
                  disabled={runDisabled}
                  title={hasMissingTools ? `Missing tools: ${toolCheck!.missing.join(", ")}` : undefined}
                  className={cn(
                    "flex items-center gap-1 px-2 py-1 text-[10px] rounded-md transition-colors",
                    !runDisabled
                      ? "bg-emerald-500/15 text-emerald-400 hover:bg-emerald-500/25"
                      : "bg-muted/10 text-muted-foreground/30 cursor-not-allowed"
                  )}
                >
                  {running ? (
                    <Loader2 className="w-3 h-3 animate-spin" />
                  ) : checkingTools ? (
                    <Loader2 className="w-3 h-3 animate-spin" />
                  ) : hasMissingTools ? (
                    <AlertTriangle className="w-3 h-3 text-amber-400" />
                  ) : (
                    <Play className="w-3 h-3" />
                  )}
                  {checkingTools ? "Checking..." : hasMissingTools ? "Tools Missing" : "Run"}
                </button>
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
                            </div>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
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
