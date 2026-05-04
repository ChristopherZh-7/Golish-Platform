import {
  AlertTriangle,
  CheckCircle2,
  Database,
  Download,
  GitBranch,
  Loader2,
  Plus,
  Save,
  Shield,
  Trash2,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { checkReconTools, type ReconToolCheck } from "@/lib/ai";
import { invoke, targets } from "@/lib/api";
import { onEvent } from "@/lib/events";
import { scanTools } from "@/lib/pentest/api";
import type { Pipeline, PipelineStep } from "@/lib/pentest/pipeline-types";
import type { ToolConfig } from "@/lib/pentest/types";
import { getProjectPath } from "@/lib/projects";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import { cn } from "@/lib/utils";
import { DagCanvas } from "./DagComponents";

type ToolWithMeta = ToolConfig & { categoryName?: string; subcategoryName?: string };

function uuid() {
  return Math.random().toString(36).slice(2, 10);
}

export function PipelinePanel() {
  const [pipelines, setPipelines] = useState<Pipeline[]>([]);
  const [active, setActive] = useState<Pipeline | null>(null);
  const [tools, setTools] = useState<ToolWithMeta[]>([]);
  const [loading, setLoading] = useState(true);
  const [dirty, setDirty] = useState(false);
  const [showToolPicker, setShowToolPicker] = useState(false);
  const [toolCheck, setToolCheck] = useState<ReconToolCheck | null>(null);
  const [aiRunning, setAiRunning] = useState(false);
  const [aiProgress, setAiProgress] = useState<{
    step: number;
    total: number;
    tool: string;
  } | null>(null);
  const [aiResult, setAiResult] = useState<{
    total_stored: number;
    steps: { tool_name: string; stored: number; exit_code: number | null }[];
  } | null>(null);
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null);
  const [previewTargetType, setPreviewTargetType] = useState<string>("");

  const knownTypes = useMemo(() => {
    const built = new Set(["domain", "ip", "url"]);
    if (active)
      for (const s of active.steps) {
        if (s.requires) built.add(s.requires);
      }
    return Array.from(built).sort();
  }, [active]);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [pl, tl] = await Promise.all([
        invoke<Pipeline[]>("pipeline_list", { projectPath: getProjectPath() }),
        scanTools(),
      ]);
      setPipelines(Array.isArray(pl) ? pl : []);
      setTools((tl?.tools || []).filter((t) => t.launchMode === "cli" && t.installed));
    } catch {
      /* */
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);
  useEffect(() => {
    if (active?.workflow_id === "recon_basic")
      checkReconTools()
        .then(setToolCheck)
        .catch(() => setToolCheck(null));
    else setToolCheck(null);
  }, [active?.workflow_id]);

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
    setActive((p) => (p ? { ...p, id } : null));
    setDirty(false);
    load();
  }, [active, load]);

  const handleSaveAsTemplate = useCallback(async () => {
    if (!active) return;
    await invoke<string>("pipeline_save_template", { pipeline: active });
    load();
  }, [active, load]);

  const handleLoadTemplate = useCallback(async () => {
    try {
      const templates = await invoke<Pipeline[]>("pipeline_list_templates");
      if (templates.length === 0) return;
      const t = templates[0];
      setActive({ ...t, id: "", is_template: false, created_at: 0, updated_at: 0 });
      setDirty(true);
    } catch {
      /* */
    }
  }, []);

  const handleDelete = useCallback(
    async (id: string) => {
      await targets.deletePipeline(id, getProjectPath());
      if (active?.id === id) setActive(null);
      load();
    },
    [active, load]
  );

  const addStep = useCallback(
    (tool: ToolWithMeta) => {
      if (!active) return;
      const s: PipelineStep = {
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
        db_action: null,
        x: 0,
        y: 0,
      };
      const conns = [...active.connections];
      if (active.steps.length > 0)
        conns.push({ from_step: active.steps[active.steps.length - 1].id, to_step: s.id });
      setActive({ ...active, steps: [...active.steps, s], connections: conns });
      setSelectedStepId(s.id);
      setDirty(true);
      setShowToolPicker(false);
    },
    [active]
  );

  const updateStep = useCallback(
    (id: string, patch: Partial<PipelineStep>) => {
      if (!active) return;
      setActive({
        ...active,
        steps: active.steps.map((s) => (s.id === id ? { ...s, ...patch } : s)),
      });
      setDirty(true);
    },
    [active]
  );

  const removeStep = useCallback(
    (id: string) => {
      if (!active) return;
      setActive({
        ...active,
        steps: active.steps.filter((s) => s.id !== id),
        connections: active.connections.filter((c) => c.from_step !== id && c.to_step !== id),
      });
      setDirty(true);
    },
    [active]
  );

  useEffect(() => {
    const ul = onEvent("pipeline-event", (p) => {
      if (p.status === "running") {
        setAiRunning(true);
        setAiProgress({ step: p.step_index + 1, total: p.total_steps, tool: p.tool_name });
      } else if (p.status === "completed" || p.status === "error") {
        setAiResult((prev) => ({
          total_stored: (prev?.total_stored ?? 0) + (p.store_stats?.stored_count ?? 0),
          steps: [
            ...(prev?.steps ?? []),
            {
              tool_name: p.tool_name,
              stored: p.store_stats?.stored_count ?? 0,
              exit_code: p.status === "completed" ? 0 : -1,
            },
          ],
        }));
        if (p.step_index + 1 >= p.total_steps) {
          setAiRunning(false);
          setAiProgress(null);
        }
      }
    });
    return () => {
      runTauriUnlistenFromPromise(ul);
    };
  }, []);

  if (loading)
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
      </div>
    );

  const isReconBasic = active?.workflow_id === "recon_basic";

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div className="flex-shrink-0 h-10 px-4 border-b border-white/[0.06] flex items-center gap-3">
        <GitBranch className="w-3.5 h-3.5 text-accent" />
        <h2 className="text-[13px] font-semibold text-foreground/90 flex-1">Pipelines</h2>
        <button
          type="button"
          onClick={handleNew}
          className="flex items-center gap-1 px-2.5 py-1 text-[10px] font-medium rounded-md bg-accent/10 text-accent hover:bg-accent/20 transition-colors border border-accent/15"
        >
          <Plus className="w-3 h-3" /> New
        </button>
        <button
          type="button"
          onClick={handleLoadTemplate}
          className="flex items-center gap-1 px-2.5 py-1 text-[10px] font-medium rounded-md text-muted-foreground/50 hover:text-foreground/70 border border-white/[0.08] hover:border-white/[0.15] transition-colors"
        >
          <Download className="w-3 h-3" /> From Template
        </button>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* Sidebar */}
        <div className="w-[180px] flex-shrink-0 border-r border-white/[0.04] overflow-y-auto bg-white/[0.01]">
          {pipelines.length === 0 && !active ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/20 px-4 text-center">
              <GitBranch className="w-5 h-5" />
              <p className="text-[10px]">No pipelines yet</p>
            </div>
          ) : (
            <div className="py-0.5">
              {pipelines.map((p) => (
                <button
                  type="button"
                  key={p.id}
                  onClick={() => {
                    setActive(p);
                    setDirty(false);
                    setSelectedStepId(null);
                  }}
                  className={cn(
                    "w-full text-left px-3 py-2 text-[11px] transition-all flex items-center gap-2 group",
                    active?.id === p.id
                      ? "bg-accent/10 text-accent border-l-2 border-accent"
                      : "text-muted-foreground/50 hover:bg-white/[0.03] hover:text-foreground/70 border-l-2 border-transparent"
                  )}
                >
                  {p.workflow_id ? (
                    <Shield className="w-3 h-3 flex-shrink-0 text-emerald-400" />
                  ) : (
                    <GitBranch className="w-3 h-3 flex-shrink-0" />
                  )}
                  <span className="flex-1 truncate">{p.name}</span>
                  <span className="text-[9px] text-muted-foreground/25">{p.steps.length}</span>
                  <span
                    role="button"
                    tabIndex={0}
                    onClick={(e) => {
                      e.stopPropagation();
                      handleDelete(p.id);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.stopPropagation();
                        handleDelete(p.id);
                      }
                    }}
                    className="p-0.5 opacity-0 group-hover:opacity-100 text-muted-foreground/20 hover:text-red-400 transition-all cursor-pointer"
                  >
                    <Trash2 className="w-3 h-3" />
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Main */}
        <div className="flex-1 flex flex-col min-w-0">
          {active ? (
            <>
              {/* Name bar */}
              <div className="flex-shrink-0 h-10 px-4 border-b border-white/[0.04] flex items-center gap-3">
                <input
                  value={active.name}
                  onChange={(e) => {
                    setActive({ ...active, name: e.target.value });
                    setDirty(true);
                  }}
                  className="text-[13px] font-semibold bg-transparent outline-none flex-1 text-foreground/90"
                  placeholder="Pipeline name"
                />
                {isReconBasic && (
                  <span className="text-[9px] px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/15">
                    Recon
                  </span>
                )}
                {aiRunning && (
                  <span className="flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md bg-emerald-500/10 text-emerald-400 border border-emerald-500/15">
                    <Loader2 className="w-3 h-3 animate-spin" />{" "}
                    {aiProgress
                      ? `${aiProgress.step}/${aiProgress.total} ${aiProgress.tool}`
                      : "Running..."}
                  </span>
                )}
                <button
                  type="button"
                  onClick={() => setShowToolPicker(!showToolPicker)}
                  className="flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md border border-white/[0.08] text-muted-foreground/50 hover:text-foreground/70 hover:border-white/[0.15] transition-colors"
                >
                  <Plus className="w-3 h-3" /> Add Step
                </button>
                <button
                  type="button"
                  onClick={handleSave}
                  disabled={!dirty}
                  className={cn(
                    "flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md transition-colors",
                    dirty
                      ? "bg-accent/15 text-accent hover:bg-accent/25 border border-accent/20"
                      : "bg-white/[0.03] text-muted-foreground/25 cursor-not-allowed border border-white/[0.04]"
                  )}
                >
                  <Save className="w-3 h-3" /> Save
                </button>
                <button
                  type="button"
                  onClick={handleSaveAsTemplate}
                  className="flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md border border-white/[0.08] text-muted-foreground/50 hover:text-foreground/70 hover:border-white/[0.15] transition-colors"
                >
                  <Download className="w-3 h-3" /> Save Template
                </button>
              </div>

              {/* Description */}
              <div className="flex-shrink-0 px-4 py-1.5 border-b border-white/[0.03]">
                <input
                  value={active.description}
                  onChange={(e) => {
                    setActive({ ...active, description: e.target.value });
                    setDirty(true);
                  }}
                  className="w-full text-[10px] text-muted-foreground/40 bg-transparent outline-none placeholder:text-muted-foreground/15"
                  placeholder="Pipeline description..."
                />
              </div>

              {/* Tool check */}
              {isReconBasic && toolCheck && (
                <div
                  className={cn(
                    "flex-shrink-0 px-4 py-2 border-b flex flex-col gap-1.5",
                    toolCheck.all_ready
                      ? "border-emerald-500/10 bg-emerald-500/[0.03]"
                      : "border-amber-500/10 bg-amber-500/[0.03]"
                  )}
                >
                  <div className="flex items-center gap-2">
                    {toolCheck.all_ready ? (
                      <CheckCircle2 className="w-3 h-3 text-emerald-400" />
                    ) : (
                      <AlertTriangle className="w-3 h-3 text-amber-400" />
                    )}
                    <span
                      className={cn(
                        "text-[10px] font-medium",
                        toolCheck.all_ready ? "text-emerald-400" : "text-amber-400"
                      )}
                    >
                      {toolCheck.all_ready
                        ? "All tools ready"
                        : `${toolCheck.missing.length} tools missing`}
                    </span>
                  </div>
                  <div className="flex flex-wrap gap-1">
                    {toolCheck.tools.map((t) => (
                      <span
                        key={t.name}
                        className={cn(
                          "inline-flex items-center gap-0.5 px-1.5 py-[2px] text-[9px] rounded-md border",
                          t.installed
                            ? "border-emerald-500/10 bg-emerald-500/5 text-emerald-400/80"
                            : "border-amber-500/15 bg-amber-500/5 text-amber-400/80"
                        )}
                      >
                        {t.installed ? (
                          <CheckCircle2 className="w-2 h-2" />
                        ) : (
                          <AlertTriangle className="w-2 h-2" />
                        )}{" "}
                        {t.name}
                      </span>
                    ))}
                  </div>
                  {!toolCheck.all_ready && (
                    <p className="text-[9px] text-amber-400/50">
                      Install missing tools in the Tool Manager before running.
                    </p>
                  )}
                </div>
              )}

              {/* Tool picker */}
              {showToolPicker && (
                <div className="flex-shrink-0 px-4 py-2.5 border-b border-white/[0.04] bg-white/[0.02]">
                  <div className="flex flex-wrap gap-1.5 max-h-[100px] overflow-y-auto">
                    {tools.map((tool) => (
                      <button
                        type="button"
                        key={tool.id}
                        onClick={() => addStep(tool)}
                        className="flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border border-white/[0.06] bg-white/[0.02] hover:bg-accent/10 hover:border-accent/20 hover:text-accent transition-all"
                      >
                        {tool.icon && <span className="text-[10px]">{tool.icon}</span>}{" "}
                        <span>{tool.name}</span>
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* Preview toolbar */}
              {active.steps.length > 0 && (
                <div className="flex-shrink-0 h-8 px-4 border-b border-white/[0.03] flex items-center gap-2 bg-white/[0.01]">
                  <span className="text-[9px] text-muted-foreground/25 uppercase tracking-wider mr-1">
                    Preview
                  </span>
                  {["", ...knownTypes].map((t) => (
                    <button
                      type="button"
                      key={t || "all"}
                      onClick={() => setPreviewTargetType(t)}
                      className={cn(
                        "px-2 py-[2px] text-[9px] rounded-md transition-all border",
                        previewTargetType === t
                          ? "bg-accent/15 text-accent border-accent/20"
                          : "text-muted-foreground/30 border-transparent hover:text-foreground/50 hover:bg-white/[0.03]"
                      )}
                    >
                      {t ? t.charAt(0).toUpperCase() + t.slice(1) : "All"}
                    </button>
                  ))}
                  {previewTargetType && (
                    <span className="ml-2 text-[9px] text-muted-foreground/20">
                      {
                        active.steps.filter((s) => !s.requires || s.requires === previewTargetType)
                          .length
                      }
                      /{active.steps.length} steps will run
                    </span>
                  )}
                </div>
              )}

              {/* DAG Canvas */}
              <DagCanvas
                pipeline={active}
                selectedStepId={selectedStepId}
                onSelectStep={setSelectedStepId}
                previewTargetType={previewTargetType}
                updateStep={updateStep}
                removeStep={removeStep}
                knownTypes={knownTypes}
              />

              {/* AI Results */}
              {aiResult && (
                <div className="flex-shrink-0 px-4 py-3 border-t border-white/[0.06] bg-white/[0.02]">
                  <div className="flex items-center gap-2 mb-1.5">
                    <Database className="w-3 h-3 text-blue-400" />
                    <span className="text-[11px] font-medium text-foreground/70">
                      {aiResult.total_stored} items stored
                    </span>
                    <button
                      type="button"
                      onClick={() => setAiResult(null)}
                      className="ml-auto p-0.5 text-muted-foreground/20 hover:text-foreground transition-colors"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                  <div className="flex flex-wrap gap-1">
                    {aiResult.steps.map((s, i) => (
                      <span
                        key={i}
                        className={cn(
                          "inline-flex items-center gap-0.5 px-1.5 py-[2px] text-[9px] rounded-md border",
                          s.exit_code === 0
                            ? "border-emerald-500/10 bg-emerald-500/5 text-emerald-400/80"
                            : "border-red-500/10 bg-red-500/5 text-red-400/80"
                        )}
                      >
                        {s.exit_code === 0 ? (
                          <CheckCircle2 className="w-2 h-2" />
                        ) : (
                          <AlertTriangle className="w-2 h-2" />
                        )}{" "}
                        {s.tool_name}
                        {s.stored > 0 && (
                          <span className="text-blue-400/70 ml-0.5">+{s.stored}</span>
                        )}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </>
          ) : (
            <div className="flex-1 flex flex-col items-center justify-center gap-3 text-muted-foreground/15">
              <GitBranch className="w-10 h-10" />
              <p className="text-[13px] font-medium text-muted-foreground/30">
                Select or create a pipeline
              </p>
              <p className="text-[10px] text-muted-foreground/20">
                Chain tools together for automated reconnaissance
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
