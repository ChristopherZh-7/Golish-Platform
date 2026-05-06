import {
  Bot,
  Code2,
  Cpu,
  Database,
  Download,
  GitBranch,
  Globe,
  Layers,
  Repeat,
  Server,
  type Shield,
  Terminal,
  Trash2,
  Wrench,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { MiniDropdown } from "@/components/ui/MiniDropdown";
import type { Pipeline, PipelineConnection, PipelineStep } from "@/lib/pentest/pipeline-types";
import { cn } from "@/lib/utils";

export const STEP_ICONS: Record<string, { icon: typeof Shield; color: string }> = {
  dns_lookup: { icon: Globe, color: "text-blue-400" },
  subdomain_enum: { icon: Globe, color: "text-cyan-400" },
  http_probe: { icon: Globe, color: "text-green-400" },
  port_scan: { icon: Server, color: "text-red-400" },
  tech_fingerprint: { icon: Cpu, color: "text-purple-400" },
  js_collect: { icon: Code2, color: "text-amber-400" },
  js_harvest: { icon: Download, color: "text-amber-500" },
  shell_command: { icon: Wrench, color: "text-muted-foreground/60" },
  sub_pipeline: { icon: Layers, color: "text-indigo-400" },
  foreach: { icon: Repeat, color: "text-orange-400" },
  ai_tool: { icon: Bot, color: "text-violet-400" },
};

const DB_ACTIONS = [
  { value: "", label: "No storage" },
  { value: "target_add", label: "Add Target" },
  { value: "target_update_recon", label: "Update Recon" },
  { value: "directory_entry_add", label: "Add Dir Entry" },
  { value: "finding_add", label: "Add Finding" },
];

const ITERATE_OPTS = [
  { value: "", label: "None" },
  { value: "ports", label: "Per Port" },
];

const TYPE_COLORS: Record<string, { bg: string; text: string; border: string; dot: string }> = {
  domain: {
    bg: "bg-cyan-500/10",
    text: "text-cyan-400",
    border: "border-cyan-500/20",
    dot: "bg-cyan-400",
  },
  ip: { bg: "bg-red-500/10", text: "text-red-400", border: "border-red-500/20", dot: "bg-red-400" },
  url: {
    bg: "bg-amber-500/10",
    text: "text-amber-400",
    border: "border-amber-500/20",
    dot: "bg-amber-400",
  },
};

function typeStyle(t: string) {
  return (
    TYPE_COLORS[t] ?? {
      bg: "bg-violet-500/10",
      text: "text-violet-400",
      border: "border-violet-500/20",
      dot: "bg-violet-400",
    }
  );
}

export function RequiresInput({
  value,
  onChange,
  knownTypes,
}: {
  value: string;
  onChange: (v: string) => void;
  knownTypes: string[];
}) {
  const [editing, setEditing] = useState(false);
  if (editing) {
    return (
      <input
        defaultValue={value}
        onBlur={(e) => {
          onChange(e.target.value.toLowerCase().trim());
          setEditing(false);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") setEditing(false);
        }}
        placeholder="e.g. webapp"
        className="w-full px-1.5 py-[3px] text-[10px] rounded-md bg-white/[0.03] border border-accent/30 text-foreground/80 outline-none"
      />
    );
  }
  return (
    <MiniDropdown
      value={value}
      onChange={(v) => {
        if (v === "__custom__") {
          setEditing(true);
          return;
        }
        onChange(v);
      }}
      options={[
        { value: "", label: "Any" },
        ...knownTypes.map((t) => ({ value: t, label: t.charAt(0).toUpperCase() + t.slice(1) })),
        { value: "__custom__", label: "+ Custom..." },
      ]}
    />
  );
}

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
  for (const id of ids) {
    inDeg.set(id, 0);
    children.set(id, []);
  }
  for (const c of connections) {
    if (!ids.has(c.from_step) || !ids.has(c.to_step)) continue;
    children.get(c.from_step)?.push(c.to_step);
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

function layoutDag(
  steps: PipelineStep[],
  connections: PipelineConnection[]
): { nodes: Map<string, NodeLayout>; width: number; height: number } {
  const layers = topoLayers(steps, connections);
  const byLayer = new Map<number, string[]>();
  for (const [id, l] of layers) {
    if (!byLayer.has(l)) byLayer.set(l, []);
    byLayer.get(l)?.push(id);
  }
  const nodes = new Map<string, NodeLayout>();
  let maxLayer = 0,
    maxInLayer = 0;
  for (const [l, ids] of byLayer) {
    maxLayer = Math.max(maxLayer, l);
    maxInLayer = Math.max(maxInLayer, ids.length);
    for (let i = 0; i < ids.length; i++) {
      nodes.set(ids[i], {
        id: ids[i],
        layer: l,
        posInLayer: i,
        x: PAD_X + l * LAYER_GAP_X,
        y: PAD_Y + i * NODE_GAP_Y,
      });
    }
  }
  return {
    nodes,
    width: Math.max(PAD_X * 2 + (maxLayer + 1) * LAYER_GAP_X, 400),
    height: Math.max(PAD_Y * 2 + maxInLayer * NODE_GAP_Y, 200),
  };
}

function DagEdges({
  connections,
  nodeMap,
}: {
  connections: PipelineConnection[];
  nodeMap: Map<string, NodeLayout>;
}) {
  return (
    <>
      {connections.map((c, i) => {
        const from = nodeMap.get(c.from_step),
          to = nodeMap.get(c.to_step);
        if (!from || !to) return null;
        const x1 = from.x + NODE_W,
          y1 = from.y + NODE_H / 2,
          x2 = to.x,
          y2 = to.y + NODE_H / 2;
        const dx = (x2 - x1) * 0.5;
        const path = `M${x1},${y1} C${x1 + dx},${y1} ${x2 - dx},${y2} ${x2},${y2}`;
        const hasCondition = !!c.condition;
        const midX = (x1 + x2) / 2,
          midY = (y1 + y2) / 2;
        return (
          <g key={`edge-${i}`}>
            <path
              d={path}
              fill="none"
              stroke={hasCondition ? "rgba(251,191,36,0.35)" : "rgba(255,255,255,0.1)"}
              strokeWidth={hasCondition ? 2 : 1.5}
              strokeDasharray={hasCondition ? "4 3" : undefined}
            />
            <polygon
              points={`${x2},${y2} ${x2 - 6},${y2 - 3} ${x2 - 6},${y2 + 3}`}
              fill={hasCondition ? "rgba(251,191,36,0.5)" : "rgba(255,255,255,0.15)"}
            />
            {hasCondition && (
              <g transform={`translate(${midX}, ${midY})`}>
                <rect
                  x={-40}
                  y={-9}
                  width={80}
                  height={18}
                  rx={4}
                  fill="rgba(251,191,36,0.08)"
                  stroke="rgba(251,191,36,0.2)"
                  strokeWidth={0.5}
                />
                <text
                  textAnchor="middle"
                  dy="0.35em"
                  fill="rgba(251,191,36,0.7)"
                  fontSize={9}
                  fontFamily="monospace"
                >
                  {c.condition}
                </text>
              </g>
            )}
          </g>
        );
      })}
    </>
  );
}

function StepNode({
  step,
  layout,
  isSelected,
  onClick,
  isSkipped,
}: {
  step: PipelineStep;
  layout: NodeLayout;
  isSelected: boolean;
  onClick: () => void;
  isSkipped: boolean;
}) {
  const meta = STEP_ICONS[step.step_type] || STEP_ICONS.shell_command;
  const StepIcon = meta.icon;
  const ts = step.requires ? typeStyle(step.requires) : null;
  const isSpecial = step.step_type === "sub_pipeline" || step.step_type === "foreach";
  const isAi = step.step_type === "ai_tool";

  return (
    <div
      className={cn(
        "absolute rounded-xl border transition-all cursor-pointer select-none bg-gradient-to-b from-white/[0.04] to-white/[0.01]",
        isSkipped && "opacity-30",
        isSelected
          ? "border-accent/40 shadow-[0_0_12px_rgba(var(--accent-rgb,99,102,241),0.15)] ring-1 ring-accent/20"
          : isAi
            ? "border-violet-500/20 hover:border-violet-500/40 shadow-[0_1px_6px_rgba(139,92,246,0.08)]"
            : "border-white/[0.08] hover:border-white/[0.15] shadow-[0_1px_6px_rgba(0,0,0,0.12)]"
      )}
      style={{ left: layout.x, top: layout.y, width: NODE_W, height: NODE_H }}
      onClick={onClick}
    >
      {/* Top-right kind badge — AI vs CLI — visible at a glance on the
          DAG so the user knows whether a step shells out or runs in-process. */}
      <span
        className={cn(
          "absolute top-1 right-1 inline-flex items-center gap-0.5 px-1 py-[1px] text-[7px] font-bold uppercase rounded border",
          isAi
            ? "bg-violet-500/15 text-violet-300 border-violet-500/25"
            : "bg-emerald-500/10 text-emerald-300/80 border-emerald-500/20"
        )}
        title={isAi ? "In-process AI tool (Tool::execute)" : "External CLI tool (sh -c)"}
      >
        {isAi ? <Bot className="w-2 h-2" /> : <Terminal className="w-2 h-2" />}
        {isAi ? "AI" : "CLI"}
      </span>
      <div className="flex items-center gap-2 px-3 py-2 h-full">
        <div
          className={cn(
            "w-8 h-8 rounded-lg flex items-center justify-center flex-shrink-0",
            `${meta.color.replace("text-", "bg-")}/10`
          )}
        >
          <StepIcon className={cn("w-4 h-4", meta.color)} />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] font-semibold text-foreground/85 truncate">
              {step.tool_name}
            </span>
            {isSpecial && (
              <span
                className={cn(
                  "px-1 py-[1px] text-[7px] font-bold uppercase rounded",
                  step.step_type === "sub_pipeline"
                    ? "bg-indigo-500/10 text-indigo-400 border border-indigo-500/15"
                    : "bg-orange-500/10 text-orange-400 border border-orange-500/15"
                )}
              >
                {step.step_type === "sub_pipeline" ? "nested" : "loop"}
              </span>
            )}
          </div>
          <div className="text-[8px] text-muted-foreground/30 font-mono truncate mt-0.5">
            {step.command_template} {step.args.join(" ")}
          </div>
          <div className="flex items-center gap-1 mt-1">
            {ts ? (
              <span
                className={cn(
                  "px-1 py-[1px] text-[7px] font-semibold rounded uppercase",
                  ts.bg,
                  ts.text,
                  "border",
                  ts.border
                )}
              >
                {step.requires}
              </span>
            ) : (
              <span className="px-1 py-[1px] text-[7px] rounded bg-white/[0.04] text-muted-foreground/25 border border-white/[0.04]">
                any
              </span>
            )}
            {step.db_action && (
              <span className="px-1 py-[1px] text-[7px] rounded bg-emerald-500/10 text-emerald-400/70 border border-emerald-500/10 flex items-center gap-0.5">
                <Database className="w-2 h-2" />
              </span>
            )}
            {step.iterate_over && (
              <span className="px-1 py-[1px] text-[7px] rounded bg-purple-500/10 text-purple-400/70 border border-purple-500/10">
                iter
              </span>
            )}
            {step.foreach_source && (
              <span className="px-1 py-[1px] text-[7px] rounded bg-orange-500/10 text-orange-400/70 border border-orange-500/10">
                foreach
              </span>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function StepDetailPanel({
  step,
  onUpdate,
  onRemove,
  onClose,
  allSteps,
  knownTypes,
}: {
  step: PipelineStep;
  onUpdate: (id: string, patch: Partial<PipelineStep>) => void;
  onRemove: (id: string) => void;
  onClose: () => void;
  allSteps: PipelineStep[];
  knownTypes: string[];
}) {
  const meta = STEP_ICONS[step.step_type] || STEP_ICONS.shell_command;
  const StepIcon = meta.icon;
  const isAiTool = step.step_type === "ai_tool";

  // Edit AI-tool args as raw JSON. We keep a local string state so that
  // mid-edit invalid JSON ("typing a comma") doesn't blow away the user's
  // input by reverting to the parsed value on every keystroke.
  const [paramsText, setParamsText] = useState(() => JSON.stringify(step.params ?? {}, null, 2));
  const [paramsError, setParamsError] = useState<string | null>(null);
  useEffect(() => {
    setParamsText(JSON.stringify(step.params ?? {}, null, 2));
    setParamsError(null);
  }, [step.id]);

  return (
    <div className="w-[280px] flex-shrink-0 border-l border-white/[0.06] bg-white/[0.02] overflow-y-auto">
      <div className="flex items-center gap-2 px-3 py-2.5 border-b border-white/[0.06]">
        <div
          className={cn(
            "w-6 h-6 rounded-md flex items-center justify-center",
            `${meta.color.replace("text-", "bg-")}/10`
          )}
        >
          <StepIcon className={cn("w-3.5 h-3.5", meta.color)} />
        </div>
        <span className="text-[12px] font-semibold text-foreground/85 flex-1 truncate">
          {step.tool_name}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="p-1 text-muted-foreground/30 hover:text-foreground/60 transition-colors"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>
      <div className="px-3 py-3 space-y-3">
        {isAiTool ? (
          <div>
            <div className="flex items-center justify-between mb-0.5">
              <label className="text-[9px] text-muted-foreground/30 font-medium uppercase tracking-wider">
                Tool Args (JSON)
              </label>
              {paramsError ? (
                <span className="text-[9px] text-red-400/70" title={paramsError}>
                  invalid JSON
                </span>
              ) : (
                <span className="text-[9px] text-emerald-400/60">parsed ✓</span>
              )}
            </div>
            <textarea
              value={paramsText}
              onChange={(e) => {
                const txt = e.target.value;
                setParamsText(txt);
                try {
                  const v = JSON.parse(txt || "{}");
                  if (typeof v !== "object" || Array.isArray(v) || v === null) {
                    setParamsError("Args must be a JSON object");
                    return;
                  }
                  setParamsError(null);
                  onUpdate(step.id, { params: v });
                } catch (err) {
                  setParamsError(err instanceof Error ? err.message : "parse error");
                }
              }}
              placeholder='{"target_url": "https://example.com", "min_confidence": 0.5}'
              spellCheck={false}
              className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 placeholder:text-muted-foreground/15 outline-none focus:border-accent/30 transition-colors min-h-[100px] resize-y"
            />
            <p className="text-[8px] text-muted-foreground/35 mt-1 leading-relaxed">
              In-process tool — runs via <code>Tool::execute</code>, not a shell. Pipeline
              auto-injects <code>target</code>/<code>target_url</code>/<code>project_path</code>{" "}
              when omitted.
            </p>
          </div>
        ) : (
          <>
            <div>
              <label className="text-[9px] text-muted-foreground/30 font-medium uppercase tracking-wider">
                Command
              </label>
              <input
                value={step.command_template}
                onChange={(e) => onUpdate(step.id, { command_template: e.target.value })}
                className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 outline-none focus:border-accent/30 transition-colors"
              />
            </div>
            <div>
              <label className="text-[9px] text-muted-foreground/30 font-medium uppercase tracking-wider">
                Arguments
              </label>
              <input
                value={step.args.join(" ")}
                onChange={(e) =>
                  onUpdate(step.id, { args: e.target.value.split(/\s+/).filter(Boolean) })
                }
                placeholder="-d {target} -silent"
                className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 placeholder:text-muted-foreground/15 outline-none focus:border-accent/30 transition-colors"
              />
            </div>
          </>
        )}
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">
              Requires
            </label>
            <RequiresInput
              value={step.requires || ""}
              onChange={(v) => onUpdate(step.id, { requires: v || null })}
              knownTypes={knownTypes}
            />
          </div>
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">
              Input From
            </label>
            <MiniDropdown
              value={step.input_from || ""}
              onChange={(v) => onUpdate(step.id, { input_from: v || null })}
              options={[
                { value: "", label: "Prev" },
                ...allSteps
                  .filter((s) => s.id !== step.id)
                  .map((s) => ({ value: s.id, label: s.tool_name })),
              ]}
            />
          </div>
        </div>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">
              Iterate
            </label>
            <MiniDropdown
              value={step.iterate_over || ""}
              onChange={(v) => onUpdate(step.id, { iterate_over: v || null })}
              options={ITERATE_OPTS}
            />
          </div>
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider flex items-center gap-0.5">
              <Database className="w-2 h-2" /> Store
            </label>
            <MiniDropdown
              value={step.db_action || ""}
              onChange={(v) => onUpdate(step.id, { db_action: v || null })}
              options={DB_ACTIONS}
            />
          </div>
        </div>
        <div>
          <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">
            Step Type
          </label>
          <MiniDropdown
            value={step.step_type}
            onChange={(v) => onUpdate(step.id, { step_type: v })}
            options={[
              { value: "shell_command", label: "Shell Command (CLI)" },
              { value: "ai_tool", label: "AI Tool (in-process)" },
              { value: "sub_pipeline", label: "Sub-Pipeline" },
              { value: "foreach", label: "For-Each Loop" },
            ]}
          />
        </div>
        {step.step_type === "sub_pipeline" && (
          <div>
            <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">
              Sub-Pipeline Template
            </label>
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
              <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">
                Foreach Source
              </label>
              <MiniDropdown
                value={step.foreach_source || ""}
                onChange={(v) => onUpdate(step.id, { foreach_source: v || null })}
                options={[
                  { value: "", label: "None" },
                  ...allSteps
                    .filter((s) => s.id !== step.id)
                    .map((s) => ({ value: s.id, label: s.tool_name })),
                ]}
              />
            </div>
            <div>
              <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">
                Max Parallel
              </label>
              <input
                type="number"
                min={1}
                value={step.max_parallel ?? ""}
                onChange={(e) =>
                  onUpdate(step.id, {
                    max_parallel: e.target.value ? Number(e.target.value) : null,
                  })
                }
                placeholder="4"
                className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 placeholder:text-muted-foreground/15 outline-none focus:border-accent/30 transition-colors"
              />
            </div>
          </>
        )}
        <div>
          <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">
            Timeout (secs)
          </label>
          <input
            type="number"
            min={0}
            value={step.timeout_secs ?? ""}
            onChange={(e) =>
              onUpdate(step.id, { timeout_secs: e.target.value ? Number(e.target.value) : null })
            }
            placeholder="300"
            className="w-full mt-0.5 px-2 py-1.5 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 placeholder:text-muted-foreground/15 outline-none focus:border-accent/30 transition-colors"
          />
        </div>
        <button
          type="button"
          onClick={() => {
            onRemove(step.id);
            onClose();
          }}
          className="w-full flex items-center justify-center gap-1 py-1.5 text-[9px] text-muted-foreground/25 hover:text-red-400 rounded-md hover:bg-red-500/5 transition-all border border-white/[0.04] mt-2"
        >
          <Trash2 className="w-2.5 h-2.5" /> Remove Step
        </button>
      </div>
    </div>
  );
}

export function DagCanvas({
  pipeline,
  selectedStepId,
  onSelectStep,
  previewTargetType,
  updateStep,
  removeStep,
  knownTypes,
}: {
  pipeline: Pipeline;
  selectedStepId: string | null;
  onSelectStep: (id: string | null) => void;
  previewTargetType: string;
  updateStep: (id: string, patch: Partial<PipelineStep>) => void;
  removeStep: (id: string) => void;
  knownTypes: string[];
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const {
    nodes: nodeMap,
    width: dagW,
    height: dagH,
  } = useMemo(
    () => layoutDag(pipeline.steps, pipeline.connections),
    [pipeline.steps, pipeline.connections]
  );
  const selectedStep = pipeline.steps.find((s) => s.id === selectedStepId) ?? null;

  return (
    <div className="flex-1 flex min-h-0">
      <div
        ref={scrollRef}
        className="flex-1 overflow-auto relative"
        onClick={(e) => {
          if (e.target === e.currentTarget || (e.target as HTMLElement).closest("[data-dag-bg]"))
            onSelectStep(null);
        }}
      >
        {pipeline.steps.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <GitBranch className="w-8 h-8" />
            <p className="text-[11px]">Add tools to build your pipeline</p>
          </div>
        ) : (
          <div
            data-dag-bg
            className="relative"
            style={{ width: dagW, height: dagH, minHeight: "100%" }}
          >
            <svg
              aria-hidden="true"
              className="absolute inset-0 pointer-events-none"
              width={dagW}
              height={dagH}
            >
              <DagEdges connections={pipeline.connections} nodeMap={nodeMap} />
            </svg>
            {pipeline.steps.map((step) => {
              const layout = nodeMap.get(step.id);
              if (!layout) return null;
              return (
                <StepNode
                  key={step.id}
                  step={step}
                  layout={layout}
                  isSelected={selectedStepId === step.id}
                  onClick={() => onSelectStep(step.id)}
                  isSkipped={
                    !!(previewTargetType && step.requires && step.requires !== previewTargetType)
                  }
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
