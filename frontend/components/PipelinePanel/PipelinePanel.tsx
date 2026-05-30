import {
  AlertTriangle,
  Bot,
  CheckCircle2,
  Database,
  Download,
  GitBranch,
  Loader2,
  Plus,
  Save,
  Shield,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { checkReconTools, type ReconToolCheck } from "@/lib/ai";
import { ApiError } from "@/lib/api";
import { translateErrorCode } from "@/lib/api/error-codes";
import {
  deletePipeline,
  listPipelines,
  listPipelineTemplates,
  savePipeline,
  savePipelineTemplate,
} from "@/lib/api/pipeline";
import { onEvent } from "@/lib/events";
import { listAiTools, scanTools } from "@/lib/pentest/api";
import type { Pipeline, PipelineStep } from "@/lib/pentest/pipeline-types";
import type { AiToolMeta, ToolConfig } from "@/lib/pentest/types";
import { getProjectPath } from "@/lib/projects";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import { cn } from "@/lib/utils";
import { DagCanvas } from "./DagComponents";

type ToolWithMeta = ToolConfig & { categoryName?: string; subcategoryName?: string };

type PickerTab = "cli" | "ai";

function uuid() {
  return Math.random().toString(36).slice(2, 10);
}

/** AI-tool category → tailwind colour token used in the picker chips and
 *  DAG node headers. Keeps the visual language consistent with the existing
 *  STEP_ICONS palette in DagComponents.tsx. */
const AI_CATEGORY_STYLES: Record<string, { text: string; bg: string; border: string }> = {
  recon: {
    text: "text-amber-400",
    bg: "bg-amber-500/10",
    border: "border-amber-500/15",
  },
  scan: {
    text: "text-violet-400",
    bg: "bg-violet-500/10",
    border: "border-violet-500/15",
  },
  data: {
    text: "text-cyan-400",
    bg: "bg-cyan-500/10",
    border: "border-cyan-500/15",
  },
  control: {
    text: "text-indigo-400",
    bg: "bg-indigo-500/10",
    border: "border-indigo-500/15",
  },
  other: {
    text: "text-muted-foreground/60",
    bg: "bg-white/[0.04]",
    border: "border-white/[0.08]",
  },
};

function aiCategoryStyle(cat: string) {
  return AI_CATEGORY_STYLES[cat] ?? AI_CATEGORY_STYLES.other;
}

export function PipelinePanel() {
  const [pipelines, setPipelines] = useState<Pipeline[]>([]);
  const [active, setActive] = useState<Pipeline | null>(null);
  const [tools, setTools] = useState<ToolWithMeta[]>([]);
  const [aiTools, setAiTools] = useState<AiToolMeta[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [showToolPicker, setShowToolPicker] = useState(false);
  const [pickerTab, setPickerTab] = useState<PickerTab>("cli");
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
    setError(null);
    try {
      const [pl, tl, ai] = await Promise.all([
        listPipelines(getProjectPath()),
        scanTools(),
        listAiTools().catch(() => [] as AiToolMeta[]),
      ]);
      setPipelines(Array.isArray(pl) ? pl : []);
      setTools((tl?.tools || []).filter((t) => t.launchMode === "cli" && t.installed));
      setAiTools(Array.isArray(ai) ? ai : []);
    } catch (e) {
      setError(
        translateErrorCode(
          e instanceof ApiError ? e.code : "UNKNOWN",
          e instanceof Error ? e.message : undefined
        )
      );
      setPipelines([]);
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
    const id = await savePipeline(active, getProjectPath());
    setActive((p) => (p ? { ...p, id } : null));
    setDirty(false);
    load();
  }, [active, load]);

  const handleSaveAsTemplate = useCallback(async () => {
    if (!active) return;
    await savePipelineTemplate(active);
    load();
  }, [active, load]);

  const handleLoadTemplate = useCallback(async () => {
    try {
      const templates = await listPipelineTemplates();
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
      await deletePipeline(id, getProjectPath());
      if (active?.id === id) setActive(null);
      load();
    },
    [active, load]
  );

  const addCliStep = useCallback(
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

  const addAiStep = useCallback(
    (tool: AiToolMeta) => {
      if (!active) return;
      const s: PipelineStep = {
        id: uuid(),
        step_type: "ai_tool",
        tool_name: tool.name,
        tool_id: `ai:${tool.name}`,
        command_template: `ai_tool:${tool.name}`,
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

  const aiToolsByCategory = useMemo(() => {
    const groups = new Map<string, AiToolMeta[]>();
    for (const t of aiTools) {
      const key = t.category || "other";
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)?.push(t);
    }
    return Array.from(groups.entries()).sort((a, b) => {
      const order = ["recon", "scan", "data", "control", "other"];
      return order.indexOf(a[0]) - order.indexOf(b[0]);
    });
  }, [aiTools]);

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

  if (error)
    return (
      <div className="flex-1 flex flex-col items-center justify-center gap-2 text-red-400/70">
        <AlertTriangle className="w-5 h-5" />
        <p className="text-[11px]">{error}</p>
      </div>
    );

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

              {/* Tool picker — split between external CLI tools and
                  in-process AI tools so users can compose mixed pipelines. */}
              {showToolPicker && (
                <div className="flex-shrink-0 px-4 py-2.5 border-b border-white/[0.04] bg-white/[0.02]">
                  <div className="flex items-center gap-1 mb-2">
                    <button
                      type="button"
                      onClick={() => setPickerTab("cli")}
                      className={cn(
                        "flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md border transition-all",
                        pickerTab === "cli"
                          ? "bg-emerald-500/10 text-emerald-300 border-emerald-500/20"
                          : "text-muted-foreground/40 border-transparent hover:text-foreground/60 hover:bg-white/[0.03]"
                      )}
                    >
                      <Terminal className="w-2.5 h-2.5" /> CLI
                      <span className="text-[9px] text-muted-foreground/30 ml-0.5">
                        {tools.length}
                      </span>
                    </button>
                    <button
                      type="button"
                      onClick={() => setPickerTab("ai")}
                      className={cn(
                        "flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md border transition-all",
                        pickerTab === "ai"
                          ? "bg-violet-500/10 text-violet-300 border-violet-500/20"
                          : "text-muted-foreground/40 border-transparent hover:text-foreground/60 hover:bg-white/[0.03]"
                      )}
                    >
                      <Bot className="w-2.5 h-2.5" /> AI Built-in
                      <span className="text-[9px] text-muted-foreground/30 ml-0.5">
                        {aiTools.length}
                      </span>
                    </button>
                    <span className="text-[9px] text-muted-foreground/25 ml-2 italic">
                      {pickerTab === "cli"
                        ? "External binaries (subfinder, httpx, …)"
                        : "In-process tools (js_collect, auth_probe, …)"}
                    </span>
                  </div>
                  {pickerTab === "cli" ? (
                    <div className="flex flex-wrap gap-1.5 max-h-[120px] overflow-y-auto">
                      {tools.length === 0 ? (
                        <span className="text-[10px] text-muted-foreground/30 italic">
                          No installed CLI tools detected. Install via Tool Manager.
                        </span>
                      ) : (
                        tools.map((tool) => (
                          <button
                            type="button"
                            key={tool.id}
                            onClick={() => addCliStep(tool)}
                            className="flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border border-white/[0.06] bg-white/[0.02] hover:bg-accent/10 hover:border-accent/20 hover:text-accent transition-all"
                          >
                            {tool.icon && <span className="text-[10px]">{tool.icon}</span>}{" "}
                            <span>{tool.name}</span>
                          </button>
                        ))
                      )}
                    </div>
                  ) : (
                    <div className="flex flex-col gap-2 max-h-[180px] overflow-y-auto">
                      {aiTools.length === 0 ? (
                        <span className="text-[10px] text-muted-foreground/30 italic">
                          No AI tools registered. Restart the app to refresh the catalog.
                        </span>
                      ) : (
                        aiToolsByCategory.map(([cat, list]) => {
                          const style = aiCategoryStyle(cat);
                          return (
                            <div key={cat} className="flex flex-col gap-1">
                              <span
                                className={cn(
                                  "self-start px-1.5 py-[1px] text-[9px] rounded uppercase tracking-wider border",
                                  style.bg,
                                  style.text,
                                  style.border
                                )}
                              >
                                {cat}
                              </span>
                              <div className="flex flex-wrap gap-1.5">
                                {list.map((tool) => (
                                  <button
                                    type="button"
                                    key={tool.name}
                                    onClick={() => addAiStep(tool)}
                                    title={tool.description}
                                    className="flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border border-violet-500/15 bg-violet-500/5 hover:bg-violet-500/15 hover:border-violet-500/30 hover:text-violet-200 transition-all"
                                  >
                                    {tool.icon && <span className="text-[10px]">{tool.icon}</span>}{" "}
                                    <span className="font-mono">{tool.name}</span>
                                  </button>
                                ))}
                              </div>
                            </div>
                          );
                        })
                      )}
                    </div>
                  )}
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
