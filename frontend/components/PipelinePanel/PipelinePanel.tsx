import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Code2, Database, Download, GitBranch, Loader2, Plus, Save, Trash2, X,
  Shield, Globe, Server, Cpu, Wrench,
  AlertTriangle, CheckCircle2, Repeat, Layers,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import { scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";
import { checkReconTools, type ReconToolCheck } from "@/lib/ai";
import { useStore } from "@/store";
import { MiniDropdown } from "@/components/ui/MiniDropdown";
import {
  type PipelineStep,
  type PipelineConnection,
  type Pipeline,
} from "@/lib/pentest/pipeline-types";

const STEP_ICONS: Record<string, { icon: typeof Shield; color: string }> = {
  dns_lookup:       { icon: Globe,    color: "text-blue-400" },
  subdomain_enum:   { icon: Globe,    color: "text-cyan-400" },
  http_probe:       { icon: Globe,    color: "text-green-400" },
  port_scan:        { icon: Server,   color: "text-red-400" },
  tech_fingerprint: { icon: Cpu,      color: "text-purple-400" },
  js_collect:       { icon: Code2,    color: "text-amber-400" },
  js_harvest:       { icon: Download, color: "text-amber-500" },
  shell_command:    { icon: Wrench,   color: "text-muted-foreground/60" },
  sub_pipeline:     { icon: Layers,   color: "text-indigo-400" },
  foreach:          { icon: Repeat,   color: "text-orange-400" },
};

const DB_ACTIONS = [
  { value: "",                     label: "No storage" },
  { value: "target_add",          label: "Add Target" },
  { value: "target_update_recon", label: "Update Recon" },
  { value: "directory_entry_add", label: "Add Dir Entry" },
  { value: "finding_add",        label: "Add Finding" },
];

const ITERATE_OPTS = [
  { value: "", label: "None" },
  { value: "ports", label: "Per Port" },
];

const TYPE_COLORS: Record<string, { bg: string; text: string; border: string; dot: string }> = {
  domain: { bg: "bg-cyan-500/10",   text: "text-cyan-400",   border: "border-cyan-500/20",   dot: "bg-cyan-400" },
  ip:     { bg: "bg-red-500/10",    text: "text-red-400",    border: "border-red-500/20",    dot: "bg-red-400" },
  url:    { bg: "bg-amber-500/10",  text: "text-amber-400",  border: "border-amber-500/20",  dot: "bg-amber-400" },
};
function typeStyle(t: string) {
  return TYPE_COLORS[t] ?? { bg: "bg-violet-500/10", text: "text-violet-400", border: "border-violet-500/20", dot: "bg-violet-400" };
}

type ToolWithMeta = ToolConfig & { categoryName?: string; subcategoryName?: string };

function uuid() { return Math.random().toString(36).slice(2, 10); }

/* ── Requires Input ── */

function RequiresInput({ value, onChange, knownTypes }: {
  value: string; onChange: (v: string) => void; knownTypes: string[];
}) {
  const [editing, setEditing] = useState(false);
  if (editing) {
    return (
      <input autoFocus defaultValue={value}
        onBlur={(e) => { onChange(e.target.value.toLowerCase().trim()); setEditing(false); }}
        onKeyDown={(e) => { if (e.key === "Enter") (e.target as HTMLInputElement).blur(); if (e.key === "Escape") setEditing(false); }}
        placeholder="e.g. webapp"
        className="w-full px-1.5 py-[3px] text-[10px] rounded-md bg-white/[0.03] border border-accent/30 text-foreground/80 outline-none"
      />
    );
  }
  return (
    <MiniDropdown value={value}
      onChange={(v) => { if (v === "__custom__") { setEditing(true); return; } onChange(v); }}
      options={[{ value: "", label: "Any" }, ...knownTypes.map((t) => ({ value: t, label: t.charAt(0).toUpperCase() + t.slice(1) })), { value: "__custom__", label: "+ Custom..." }]}
    />
  );
}

/* ── DAG Layout ── */

const NODE_W = 200;
const NODE_H = 72;
const LAYER_GAP_X = 260;
const NODE_GAP_Y = 92;
const PAD_X = 40;
const PAD_Y = 32;

interface NodeLayout {
  id: string;
  layer: number;
  posInLayer: number;
  x: number;
  y: number;
}

function topoLayers(steps: PipelineStep[], connections: PipelineConnection[]): Map<string, number> {
  const ids = new Set(steps.map((s) => s.id));
  const inDeg = new Map<string, number>();
  const children = new Map<string, string[]>();
  for (const id of ids) { inDeg.set(id, 0); children.set(id, []); }
  for (const c of connections) {
    if (!ids.has(c.from_step) || !ids.has(c.to_step)) continue;
    children.get(c.from_step)!.push(c.to_step);
    inDeg.set(c.to_step, (inDeg.get(c.to_step) ?? 0) + 1);
  }

  const layers = new Map<string, number>();
  let queue = [...ids].filter((id) => (inDeg.get(id) ?? 0) === 0);
  let layer = 0;
  while (queue.length > 0) {
    const next: string[] = [];
    for (const id of queue) {
      layers.set(id, layer);
      for (const child of children.get(id) ?? []) {
        const d = (inDeg.get(child) ?? 1) - 1;
        inDeg.set(child, d);
        if (d === 0) next.push(child);
      }
    }
    queue = next;
    layer++;
  }

  for (const id of ids) {
    if (!layers.has(id)) layers.set(id, layer);
  }
  return layers;
}

function layoutDag(steps: PipelineStep[], connections: PipelineConnection[]): { nodes: Map<string, NodeLayout>; width: number; height: number } {
  const layers = topoLayers(steps, connections);
  const byLayer = new Map<number, string[]>();
  for (const [id, l] of layers) {
    if (!byLayer.has(l)) byLayer.set(l, []);
    byLayer.get(l)!.push(id);
  }

  const nodes = new Map<string, NodeLayout>();
  let maxLayer = 0;
  let maxInLayer = 0;
  for (const [l, ids] of byLayer) {
    maxLayer = Math.max(maxLayer, l);
    maxInLayer = Math.max(maxInLayer, ids.length);
    for (let i = 0; i < ids.length; i++) {
      const x = PAD_X + l * LAYER_GAP_X;
      const y = PAD_Y + i * NODE_GAP_Y;
      nodes.set(ids[i], { id: ids[i], layer: l, posInLayer: i, x, y });
    }
  }

  const width = PAD_X * 2 + (maxLayer + 1) * LAYER_GAP_X;
  const height = PAD_Y * 2 + maxInLayer * NODE_GAP_Y;
  return { nodes, width: Math.max(width, 400), height: Math.max(height, 200) };
}

/* ── DAG Edge SVG ── */

function DagEdges({ connections, nodeMap }: {
  connections: PipelineConnection[];
  nodeMap: Map<string, NodeLayout>;
}) {
  return (
    <>
      {connections.map((c, i) => {
        const from = nodeMap.get(c.from_step);
        const to = nodeMap.get(c.to_step);
        if (!from || !to) return null;

        const x1 = from.x + NODE_W;
        const y1 = from.y + NODE_H / 2;
        const x2 = to.x;
        const y2 = to.y + NODE_H / 2;
        const dx = (x2 - x1) * 0.5;
        const path = `M${x1},${y1} C${x1 + dx},${y1} ${x2 - dx},${y2} ${x2},${y2}`;

        const hasCondition = !!c.condition;
        const midX = (x1 + x2) / 2;
        const midY = (y1 + y2) / 2;

        return (
          <g key={`edge-${i}`}>
            <path d={path} fill="none" stroke={hasCondition ? "rgba(251,191,36,0.35)" : "rgba(255,255,255,0.1)"} strokeWidth={hasCondition ? 2 : 1.5} strokeDasharray={hasCondition ? "4 3" : undefined} />
            <polygon
              points={`${x2},${y2} ${x2 - 6},${y2 - 3} ${x2 - 6},${y2 + 3}`}
              fill={hasCondition ? "rgba(251,191,36,0.5)" : "rgba(255,255,255,0.15)"}
            />
            {hasCondition && (
              <g transform={`translate(${midX}, ${midY})`}>
                <rect x={-40} y={-9} width={80} height={18} rx={4} fill="rgba(251,191,36,0.08)" stroke="rgba(251,191,36,0.2)" strokeWidth={0.5} />
                <text textAnchor="middle" dy="0.35em" fill="rgba(251,191,36,0.7)" fontSize={9} fontFamily="monospace">{c.condition}</text>
              </g>
            )}
          </g>
        );
      })}
    </>
  );
}

/* ── DAG Step Node ── */

function StepNode({ step, layout, isSelected, onClick, isSkipped }: {
  step: PipelineStep;
  layout: NodeLayout;
  isSelected: boolean;
  onClick: () => void;
  isSkipped: boolean;
}) {
  const meta = STEP_ICONS[step.step_type] || STEP_ICONS.shell_command;
  const StepIcon = meta.icon;
  const hasStorage = !!(step.db_action);
  const ts = step.requires ? typeStyle(step.requires) : null;
  const isSpecial = step.step_type === "sub_pipeline" || step.step_type === "foreach";

  return (
    <div
      className={cn(
        "absolute rounded-xl border transition-all cursor-pointer select-none",
        "bg-gradient-to-b from-white/[0.04] to-white/[0.01]",
        isSkipped && "opacity-30",
        isSelected
          ? "border-accent/40 shadow-[0_0_12px_rgba(var(--accent-rgb,99,102,241),0.15)] ring-1 ring-accent/20"
          : "border-white/[0.08] hover:border-white/[0.15] shadow-[0_1px_6px_rgba(0,0,0,0.12)]",
      )}
      style={{ left: layout.x, top: layout.y, width: NODE_W, height: NODE_H }}
      onClick={onClick}
    >
      <div className="flex items-center gap-2 px-3 py-2 h-full">
        <div className={cn(
          "w-8 h-8 rounded-lg flex items-center justify-center flex-shrink-0",
          `${meta.color.replace("text-", "bg-")}/10`,
        )}>
          <StepIcon className={cn("w-4 h-4", meta.color)} />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] font-semibold text-foreground/85 truncate">{step.tool_name}</span>
            {isSpecial && (
              <span className={cn(
                "px-1 py-[1px] text-[7px] font-bold uppercase rounded",
                step.step_type === "sub_pipeline"
                  ? "bg-indigo-500/10 text-indigo-400 border border-indigo-500/15"
                  : "bg-orange-500/10 text-orange-400 border border-orange-500/15",
              )}>
                {step.step_type === "sub_pipeline" ? "nested" : "loop"}
              </span>
            )}
          </div>
          <div className="text-[8px] text-muted-foreground/30 font-mono truncate mt-0.5">
            {step.command_template} {step.args.join(" ")}
          </div>
          <div className="flex items-center gap-1 mt-1">
            {ts ? (
              <span className={cn("px-1 py-[1px] text-[7px] font-semibold rounded uppercase", ts.bg, ts.text, "border", ts.border)}>
                {step.requires}
              </span>
            ) : (
              <span className="px-1 py-[1px] text-[7px] rounded bg-white/[0.04] text-muted-foreground/25 border border-white/[0.04]">any</span>
            )}
            {hasStorage && (
              <span className="px-1 py-[1px] text-[7px] rounded bg-emerald-500/10 text-emerald-400/70 border border-emerald-500/10 flex items-center gap-0.5">
                <Database className="w-2 h-2" />
              </span>
            )}
            {step.iterate_over && (
              <span className="px-1 py-[1px] text-[7px] rounded bg-purple-500/10 text-purple-400/70 border border-purple-500/10">iter</span>
            )}
            {step.foreach_source && (
              <span className="px-1 py-[1px] text-[7px] rounded bg-orange-500/10 text-orange-400/70 border border-orange-500/10">foreach</span>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/* ── Step Detail Side Panel ── */

function StepDetailPanel({ step, onUpdate, onRemove, onClose, allSteps, knownTypes }: {
  step: PipelineStep;
  onUpdate: (id: string, patch: Partial<PipelineStep>) => void;
  onRemove: (id: string) => void;
  onClose: () => void;
  allSteps: PipelineStep[];
  knownTypes: string[];
}) {
  const meta = STEP_ICONS[step.step_type] || STEP_ICONS.shell_command;
  const StepIcon = meta.icon;

  return (
    <div className="w-[280px] flex-shrink-0 border-l border-white/[0.06] bg-white/[0.02] overflow-y-auto">
      <div className="flex items-center gap-2 px-3 py-2.5 border-b border-white/[0.06]">
        <div className={cn("w-6 h-6 rounded-md flex items-center justify-center", `${meta.color.replace("text-", "bg-")}/10`)}>
          <StepIcon className={cn("w-3.5 h-3.5", meta.color)} />
        </div>
        <span className="text-[12px] font-semibold text-foreground/85 flex-1 truncate">{step.tool_name}</span>
        <button type="button" onClick={onClose} className="p-1 text-muted-foreground/30 hover:text-foreground/60 transition-colors">
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      <div className="px-3 py-3 space-y-3">
        <div>
          <label className="text-[9px] text-muted-foreground/30 font-medium uppercase tracking-wider">Command</label>
          <input
            value={step.command_template}
            onChange={(e) => onUpdate(step.id, { command_template: e.target.value })}
            className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 outline-none focus:border-accent/30 transition-colors"
          />
        </div>
        <div>
          <label className="text-[9px] text-muted-foreground/30 font-medium uppercase tracking-wider">Arguments</label>
          <input
            value={step.args.join(" ")}
            onChange={(e) => onUpdate(step.id, { args: e.target.value.split(/\s+/).filter(Boolean) })}
            placeholder="-d {target} -silent"
            className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 placeholder:text-muted-foreground/15 outline-none focus:border-accent/30 transition-colors"
          />
        </div>

        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Requires</label>
            <RequiresInput value={step.requires || ""} onChange={(v) => onUpdate(step.id, { requires: v || null })} knownTypes={knownTypes} />
          </div>
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Input From</label>
            <MiniDropdown
              value={step.input_from || ""}
              onChange={(v) => onUpdate(step.id, { input_from: v || null })}
              options={[{ value: "", label: "Prev" }, ...allSteps.filter((s) => s.id !== step.id).map((s) => ({ value: s.id, label: s.tool_name }))]}
            />
          </div>
        </div>

        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Iterate</label>
            <MiniDropdown value={step.iterate_over || ""} onChange={(v) => onUpdate(step.id, { iterate_over: v || null })} options={ITERATE_OPTS} />
          </div>
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider flex items-center gap-0.5"><Database className="w-2 h-2" /> Store</label>
            <MiniDropdown value={step.db_action || ""} onChange={(v) => onUpdate(step.id, { db_action: v || null })} options={DB_ACTIONS} />
          </div>
        </div>

        <div>
          <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Step Type</label>
          <MiniDropdown
            value={step.step_type}
            onChange={(v) => onUpdate(step.id, { step_type: v })}
            options={[
              { value: "shell_command", label: "Shell Command" },
              { value: "sub_pipeline", label: "Sub-Pipeline" },
              { value: "foreach", label: "For-Each Loop" },
            ]}
          />
        </div>

        {step.step_type === "sub_pipeline" && (
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Sub-Pipeline Template</label>
            <input
              value={step.sub_pipeline || ""}
              onChange={(e) => onUpdate(step.id, { sub_pipeline: e.target.value || null })}
              placeholder="template-id"
              className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 placeholder:text-muted-foreground/15 outline-none focus:border-accent/30 transition-colors"
            />
          </div>
        )}

        {step.step_type === "foreach" && (
          <>
            <div>
              <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Foreach Source</label>
              <MiniDropdown
                value={step.foreach_source || ""}
                onChange={(v) => onUpdate(step.id, { foreach_source: v || null })}
                options={[{ value: "", label: "None" }, ...allSteps.filter((s) => s.id !== step.id).map((s) => ({ value: s.id, label: s.tool_name }))]}
              />
            </div>
            <div>
              <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Max Parallel</label>
              <input
                type="number"
                min={1}
                value={step.max_parallel ?? ""}
                onChange={(e) => onUpdate(step.id, { max_parallel: e.target.value ? Number(e.target.value) : null })}
                placeholder="4"
                className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 placeholder:text-muted-foreground/15 outline-none focus:border-accent/30 transition-colors"
              />
            </div>
          </>
        )}

        <div>
          <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Timeout (secs)</label>
          <input
            type="number"
            min={0}
            value={step.timeout_secs ?? ""}
            onChange={(e) => onUpdate(step.id, { timeout_secs: e.target.value ? Number(e.target.value) : null })}
            placeholder="300"
            className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 placeholder:text-muted-foreground/15 outline-none focus:border-accent/30 transition-colors"
          />
        </div>

        <button
          type="button"
          onClick={() => { onRemove(step.id); onClose(); }}
          className="w-full flex items-center justify-center gap-1 py-1.5 text-[9px] text-muted-foreground/25 hover:text-red-400 rounded-md hover:bg-red-500/5 transition-all border border-white/[0.04] mt-2"
        >
          <Trash2 className="w-2.5 h-2.5" /> Remove Step
        </button>
      </div>
    </div>
  );
}

/* ── DAG Canvas ── */

function DagCanvas({ pipeline, selectedStepId, onSelectStep, previewTargetType, updateStep, removeStep, knownTypes }: {
  pipeline: Pipeline;
  selectedStepId: string | null;
  onSelectStep: (id: string | null) => void;
  previewTargetType: string;
  updateStep: (id: string, patch: Partial<PipelineStep>) => void;
  removeStep: (id: string) => void;
  knownTypes: string[];
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const { nodes: nodeMap, width: dagW, height: dagH } = useMemo(
    () => layoutDag(pipeline.steps, pipeline.connections),
    [pipeline.steps, pipeline.connections],
  );

  const selectedStep = pipeline.steps.find((s) => s.id === selectedStepId) ?? null;

  return (
    <div className="flex-1 flex min-h-0">
      <div ref={scrollRef} className="flex-1 overflow-auto relative" onClick={(e) => { if (e.target === e.currentTarget || (e.target as HTMLElement).closest("[data-dag-bg]")) onSelectStep(null); }}>
        {pipeline.steps.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <GitBranch className="w-8 h-8" />
            <p className="text-[11px]">Add tools to build your pipeline</p>
          </div>
        ) : (
          <div data-dag-bg className="relative" style={{ width: dagW, height: dagH, minHeight: "100%" }}>
            <svg className="absolute inset-0 pointer-events-none" width={dagW} height={dagH}>
              <DagEdges connections={pipeline.connections} nodeMap={nodeMap} />
            </svg>
            {pipeline.steps.map((step) => {
              const layout = nodeMap.get(step.id);
              if (!layout) return null;
              const skipped = !!(previewTargetType && step.requires && step.requires !== previewTargetType);
              return (
                <StepNode
                  key={step.id}
                  step={step}
                  layout={layout}
                  isSelected={selectedStepId === step.id}
                  onClick={() => onSelectStep(step.id)}
                  isSkipped={skipped}
                />
              );
            })}
          </div>
        )}
      </div>

      {selectedStep && (
        <StepDetailPanel
          step={selectedStep}
          onUpdate={updateStep}
          onRemove={removeStep}
          onClose={() => onSelectStep(null)}
          allSteps={pipeline.steps}
          knownTypes={knownTypes}
        />
      )}
    </div>
  );
}

/* ── Main Component ── */

export function PipelinePanel() {
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [pipelines, setPipelines] = useState<Pipeline[]>([]);
  const [active, setActive] = useState<Pipeline | null>(null);
  const [tools, setTools] = useState<ToolWithMeta[]>([]);
  const [loading, setLoading] = useState(true);
  const [dirty, setDirty] = useState(false);
  const [showToolPicker, setShowToolPicker] = useState(false);
  const [toolCheck, setToolCheck] = useState<ReconToolCheck | null>(null);
  const [aiRunning, setAiRunning] = useState(false);
  const [aiProgress, setAiProgress] = useState<{ step: number; total: number; tool: string } | null>(null);
  const [aiResult, setAiResult] = useState<{ total_stored: number; steps: { tool_name: string; stored: number; exit_code: number | null }[] } | null>(null);
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null);
  const [previewTargetType, setPreviewTargetType] = useState<string>("");

  const knownTypes = useMemo(() => {
    const built = new Set(["domain", "ip", "url"]);
    if (active) for (const s of active.steps) { if (s.requires) built.add(s.requires); }
    return Array.from(built).sort();
  }, [active]);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [pl, tl] = await Promise.all([invoke<Pipeline[]>("pipeline_list", { projectPath: getProjectPath() }), scanTools()]);
      setPipelines(Array.isArray(pl) ? pl : []);
      setTools((tl?.tools || []).filter((t) => t.launchMode === "cli" && t.installed));
    } catch { /* */ }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load, currentProjectPath]);
  useEffect(() => {
    if (active?.workflow_id === "recon_basic") checkReconTools().then(setToolCheck).catch(() => setToolCheck(null));
    else setToolCheck(null);
  }, [active?.id, active?.workflow_id]);

  const handleNew = useCallback(() => {
    setActive({ id: "", name: "New Pipeline", description: "", is_template: false, steps: [], connections: [], created_at: 0, updated_at: 0 });
    setDirty(true);
  }, []);

  const handleSave = useCallback(async () => {
    if (!active) return;
    const id = await invoke<string>("pipeline_save", { pipeline: active, projectPath: getProjectPath() });
    setActive((p) => p ? { ...p, id } : null); setDirty(false); load();
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
    } catch { /* */ }
  }, []);

  const handleDelete = useCallback(async (id: string) => {
    await invoke("pipeline_delete", { id, projectPath: getProjectPath() });
    if (active?.id === id) setActive(null); load();
  }, [active, load]);

  const addStep = useCallback((tool: ToolWithMeta) => {
    if (!active) return;
    const s: PipelineStep = {
      id: uuid(), step_type: "shell_command", tool_name: tool.name, tool_id: tool.id,
      command_template: tool.executable || tool.name, args: [], params: {},
      input_from: null, exec_mode: "pipe", requires: null, db_action: null,
      x: 0, y: 0,
    };
    const conns = [...active.connections];
    if (active.steps.length > 0) conns.push({ from_step: active.steps[active.steps.length - 1].id, to_step: s.id });
    setActive({ ...active, steps: [...active.steps, s], connections: conns });
    setSelectedStepId(s.id);
    setDirty(true); setShowToolPicker(false);
  }, [active]);

  const updateStep = useCallback((id: string, patch: Partial<PipelineStep>) => {
    if (!active) return;
    setActive({ ...active, steps: active.steps.map((s) => s.id === id ? { ...s, ...patch } : s) }); setDirty(true);
  }, [active]);

  const removeStep = useCallback((id: string) => {
    if (!active) return;
    setActive({ ...active, steps: active.steps.filter((s) => s.id !== id), connections: active.connections.filter((c) => c.from_step !== id && c.to_step !== id) }); setDirty(true);
  }, [active]);

  useEffect(() => {
    const ul = listen<{ pipeline_id: string; step_index: number; total_steps: number; tool_name: string; status: string; store_stats?: { stored_count: number } }>("pipeline-event", (ev) => {
      const p = ev.payload;
      if (p.status === "running") { setAiRunning(true); setAiProgress({ step: p.step_index + 1, total: p.total_steps, tool: p.tool_name }); }
      else if (p.status === "completed" || p.status === "error") {
        setAiResult((prev) => ({ total_stored: (prev?.total_stored ?? 0) + (p.store_stats?.stored_count ?? 0), steps: [...(prev?.steps ?? []), { tool_name: p.tool_name, stored: p.store_stats?.stored_count ?? 0, exit_code: p.status === "completed" ? 0 : -1 }] }));
        if (p.step_index + 1 >= p.total_steps) { setAiRunning(false); setAiProgress(null); }
      }
    });
    return () => { ul.then((f) => f()); };
  }, []);

  if (loading) return <div className="flex-1 flex items-center justify-center"><Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" /></div>;

  const isReconBasic = active?.workflow_id === "recon_basic";

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div className="flex-shrink-0 h-10 px-4 border-b border-white/[0.06] flex items-center gap-3">
        <GitBranch className="w-3.5 h-3.5 text-accent" />
        <h2 className="text-[13px] font-semibold text-foreground/90 flex-1">Pipelines</h2>
        <button onClick={handleNew} className="flex items-center gap-1 px-2.5 py-1 text-[10px] font-medium rounded-md bg-accent/10 text-accent hover:bg-accent/20 transition-colors border border-accent/15">
          <Plus className="w-3 h-3" /> New
        </button>
        <button onClick={handleLoadTemplate} className="flex items-center gap-1 px-2.5 py-1 text-[10px] font-medium rounded-md text-muted-foreground/50 hover:text-foreground/70 border border-white/[0.08] hover:border-white/[0.15] transition-colors">
          <Download className="w-3 h-3" /> From Template
        </button>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* Sidebar */}
        <div className="w-[180px] flex-shrink-0 border-r border-white/[0.04] overflow-y-auto bg-white/[0.01]">
          {pipelines.length === 0 && !active ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/20 px-4 text-center">
              <GitBranch className="w-5 h-5" /><p className="text-[10px]">No pipelines yet</p>
            </div>
          ) : (
            <div className="py-0.5">
              {pipelines.map((p) => (
                <button key={p.id} onClick={() => { setActive(p); setDirty(false); setSelectedStepId(null); }}
                  className={cn("w-full text-left px-3 py-2 text-[11px] transition-all flex items-center gap-2 group", active?.id === p.id ? "bg-accent/10 text-accent border-l-2 border-accent" : "text-muted-foreground/50 hover:bg-white/[0.03] hover:text-foreground/70 border-l-2 border-transparent")}>
                  {p.workflow_id ? <Shield className="w-3 h-3 flex-shrink-0 text-emerald-400" /> : <GitBranch className="w-3 h-3 flex-shrink-0" />}
                  <span className="flex-1 truncate">{p.name}</span>
                  <span className="text-[9px] text-muted-foreground/25">{p.steps.length}</span>
                  <span role="button" tabIndex={0} onClick={(e) => { e.stopPropagation(); handleDelete(p.id); }} onKeyDown={(e) => { if (e.key === "Enter") { e.stopPropagation(); handleDelete(p.id); } }} className="p-0.5 opacity-0 group-hover:opacity-100 text-muted-foreground/20 hover:text-red-400 transition-all cursor-pointer">
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
                <input value={active.name} onChange={(e) => { setActive({ ...active, name: e.target.value }); setDirty(true); }} className="text-[13px] font-semibold bg-transparent outline-none flex-1 text-foreground/90" placeholder="Pipeline name" />
                {isReconBasic && <span className="text-[9px] px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/15">Recon</span>}
                {aiRunning && (
                  <span className="flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md bg-emerald-500/10 text-emerald-400 border border-emerald-500/15">
                    <Loader2 className="w-3 h-3 animate-spin" /> {aiProgress ? `${aiProgress.step}/${aiProgress.total} ${aiProgress.tool}` : "Running..."}
                  </span>
                )}
                <button onClick={() => setShowToolPicker(!showToolPicker)} className="flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md border border-white/[0.08] text-muted-foreground/50 hover:text-foreground/70 hover:border-white/[0.15] transition-colors">
                  <Plus className="w-3 h-3" /> Add Step
                </button>
                <button onClick={handleSave} disabled={!dirty} className={cn("flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md transition-colors", dirty ? "bg-accent/15 text-accent hover:bg-accent/25 border border-accent/20" : "bg-white/[0.03] text-muted-foreground/25 cursor-not-allowed border border-white/[0.04]")}>
                  <Save className="w-3 h-3" /> Save
                </button>
                <button onClick={handleSaveAsTemplate} className="flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md border border-white/[0.08] text-muted-foreground/50 hover:text-foreground/70 hover:border-white/[0.15] transition-colors">
                  <Download className="w-3 h-3" /> Save Template
                </button>
              </div>

              {/* Description */}
              <div className="flex-shrink-0 px-4 py-1.5 border-b border-white/[0.03]">
                <input value={active.description} onChange={(e) => { setActive({ ...active, description: e.target.value }); setDirty(true); }} className="w-full text-[10px] text-muted-foreground/40 bg-transparent outline-none placeholder:text-muted-foreground/15" placeholder="Pipeline description..." />
              </div>

              {/* Tool check */}
              {isReconBasic && toolCheck && (
                <div className={cn("flex-shrink-0 px-4 py-2 border-b flex flex-col gap-1.5", toolCheck.all_ready ? "border-emerald-500/10 bg-emerald-500/[0.03]" : "border-amber-500/10 bg-amber-500/[0.03]")}>
                  <div className="flex items-center gap-2">
                    {toolCheck.all_ready ? <CheckCircle2 className="w-3 h-3 text-emerald-400" /> : <AlertTriangle className="w-3 h-3 text-amber-400" />}
                    <span className={cn("text-[10px] font-medium", toolCheck.all_ready ? "text-emerald-400" : "text-amber-400")}>{toolCheck.all_ready ? "All tools ready" : `${toolCheck.missing.length} tools missing`}</span>
                  </div>
                  <div className="flex flex-wrap gap-1">
                    {toolCheck.tools.map((t) => (
                      <span key={t.name} className={cn("inline-flex items-center gap-0.5 px-1.5 py-[2px] text-[9px] rounded-md border", t.installed ? "border-emerald-500/10 bg-emerald-500/5 text-emerald-400/80" : "border-amber-500/15 bg-amber-500/5 text-amber-400/80")}>
                        {t.installed ? <CheckCircle2 className="w-2 h-2" /> : <AlertTriangle className="w-2 h-2" />} {t.name}
                      </span>
                    ))}
                  </div>
                  {!toolCheck.all_ready && <p className="text-[9px] text-amber-400/50">Install missing tools in the Tool Manager before running.</p>}
                </div>
              )}

              {/* Tool picker */}
              {showToolPicker && (
                <div className="flex-shrink-0 px-4 py-2.5 border-b border-white/[0.04] bg-white/[0.02]">
                  <div className="flex flex-wrap gap-1.5 max-h-[100px] overflow-y-auto">
                    {tools.map((tool) => (
                      <button key={tool.id} onClick={() => addStep(tool)} className="flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border border-white/[0.06] bg-white/[0.02] hover:bg-accent/10 hover:border-accent/20 hover:text-accent transition-all">
                        {tool.icon && <span className="text-[10px]">{tool.icon}</span>} <span>{tool.name}</span>
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* Preview toolbar */}
              {active.steps.length > 0 && (
                <div className="flex-shrink-0 h-8 px-4 border-b border-white/[0.03] flex items-center gap-2 bg-white/[0.01]">
                  <span className="text-[9px] text-muted-foreground/25 uppercase tracking-wider mr-1">Preview</span>
                  {["", ...knownTypes].map((t) => (
                    <button key={t || "all"} onClick={() => setPreviewTargetType(t)} className={cn("px-2 py-[2px] text-[9px] rounded-md transition-all border", previewTargetType === t ? "bg-accent/15 text-accent border-accent/20" : "text-muted-foreground/30 border-transparent hover:text-foreground/50 hover:bg-white/[0.03]")}>
                      {t ? t.charAt(0).toUpperCase() + t.slice(1) : "All"}
                    </button>
                  ))}
                  {previewTargetType && (
                    <span className="ml-2 text-[9px] text-muted-foreground/20">
                      {active.steps.filter((s) => !s.requires || s.requires === previewTargetType).length}/{active.steps.length} steps will run
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
                    <span className="text-[11px] font-medium text-foreground/70">{aiResult.total_stored} items stored</span>
                    <button onClick={() => setAiResult(null)} className="ml-auto p-0.5 text-muted-foreground/20 hover:text-foreground transition-colors"><X className="w-3 h-3" /></button>
                  </div>
                  <div className="flex flex-wrap gap-1">
                    {aiResult.steps.map((s, i) => (
                      <span key={i} className={cn("inline-flex items-center gap-0.5 px-1.5 py-[2px] text-[9px] rounded-md border", s.exit_code === 0 ? "border-emerald-500/10 bg-emerald-500/5 text-emerald-400/80" : "border-red-500/10 bg-red-500/5 text-red-400/80")}>
                        {s.exit_code === 0 ? <CheckCircle2 className="w-2 h-2" /> : <AlertTriangle className="w-2 h-2" />} {s.tool_name}
                        {s.stored > 0 && <span className="text-blue-400/70 ml-0.5">+{s.stored}</span>}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </>
          ) : (
            <div className="flex-1 flex flex-col items-center justify-center gap-3 text-muted-foreground/15">
              <GitBranch className="w-10 h-10" />
              <p className="text-[13px] font-medium text-muted-foreground/30">Select or create a pipeline</p>
              <p className="text-[10px] text-muted-foreground/20">Chain tools together for automated reconnaissance</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
