import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowRight, ChevronDown, GitBranch, Loader2, Play, Plus, Save, Trash2, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import { getProjectPath } from "@/lib/projects";
import { scanTools, type ToolConfig } from "@/lib/pentest/api";

interface PipelineStep {
  id: string;
  tool_name: string;
  tool_id: string;
  command_template: string;
  args: string[];
  input_from: string | null;
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
  const canvasRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [pipelineList, toolList] = await Promise.all([
        invoke<Pipeline[]>("pipeline_list", { projectPath: getProjectPath() }),
        scanTools(),
      ]);
      setPipelines(pipelineList);
      setTools(toolList.filter((t) => t.ui === "cli"));
    } catch { /* ignore */ }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load, currentProjectPath]);

  const handleNew = useCallback(() => {
    setActive({
      id: "",
      name: "New Pipeline",
      description: "",
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
  }, [active, load]);

  const addStep = useCallback((tool: ToolWithMeta) => {
    if (!active) return;
    const stepCount = active.steps.length;
    const newStep: PipelineStep = {
      id: uuid(),
      tool_name: tool.name,
      tool_id: tool.id,
      command_template: tool.executable || tool.name,
      args: [],
      input_from: null,
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

  const handleRun = useCallback(async () => {
    if (!active || active.steps.length === 0) return;
    const commands = active.steps.map((step) => {
      const args = step.args.length > 0 ? ` ${step.args.join(" ")}` : "";
      return `${step.command_template}${args}`;
    });
    const chainedCommand = commands.join(" | ");

    const store = useStore.getState();
    const createTerminal = store.createSession;
    const sessionId = await createTerminal("pipeline");
    store.setActiveSession(sessionId);
    await invoke("terminal_write", {
      sessionId,
      data: chainedCommand + "\n",
    }).catch(() => {});
  }, [active]);

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div className="flex-shrink-0 px-4 py-3 border-b border-border/30 flex items-center gap-3">
        <GitBranch className="w-4 h-4 text-accent" />
        <h2 className="text-sm font-semibold flex-1">Tool Pipelines</h2>
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
              <p className="text-[10px]">No pipelines yet. Create one to chain tools together.</p>
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
                  <GitBranch className="w-3 h-3 flex-shrink-0" />
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
                <button
                  onClick={() => setShowToolPicker(!showToolPicker)}
                  className="flex items-center gap-1 px-2 py-1 text-[10px] rounded-md border border-border/20 text-muted-foreground/50 hover:text-foreground hover:border-border/40 transition-colors"
                >
                  <Plus className="w-3 h-3" />
                  Add Step
                </button>
                <button
                  onClick={handleRun}
                  disabled={active.steps.length === 0}
                  className={cn(
                    "flex items-center gap-1 px-2 py-1 text-[10px] rounded-md transition-colors",
                    active.steps.length > 0
                      ? "bg-emerald-500/15 text-emerald-400 hover:bg-emerald-500/25"
                      : "bg-muted/10 text-muted-foreground/30 cursor-not-allowed"
                  )}
                >
                  <Play className="w-3 h-3" />
                  Run
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
                      return (
                        <div key={step.id} className="flex items-center gap-3">
                          {hasConnection && (
                            <div className="flex items-center text-muted-foreground/30">
                              <ArrowRight className="w-5 h-5" />
                            </div>
                          )}
                          <div className="w-[200px] rounded-lg border border-border/20 bg-card overflow-hidden group">
                            <div className="flex items-center gap-2 px-3 py-2 border-b border-border/15 bg-muted/10">
                              <span className="text-[11px] font-medium flex-1 truncate">{step.tool_name}</span>
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
                                  const steps = active.steps.map((s) =>
                                    s.id === step.id ? { ...s, command_template: e.target.value } : s
                                  );
                                  setActive({ ...active, steps });
                                  setDirty(true);
                                }}
                                className="w-full px-1.5 py-1 text-[10px] font-mono rounded bg-muted/10 border border-border/10 text-foreground outline-none"
                              />
                              <div className="text-[9px] text-muted-foreground/40">Arguments</div>
                              <input
                                value={step.args.join(" ")}
                                onChange={(e) => {
                                  const args = e.target.value.split(/\s+/).filter(Boolean);
                                  const steps = active.steps.map((s) =>
                                    s.id === step.id ? { ...s, args } : s
                                  );
                                  setActive({ ...active, steps });
                                  setDirty(true);
                                }}
                                placeholder="e.g. -d example.com -silent"
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
              <p className="text-[10px]">Chain tools together to automate reconnaissance workflows</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
