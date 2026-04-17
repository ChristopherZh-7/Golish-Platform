import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Code2, Database, Download, GitBranch, Loader2, Plus, Save, Trash2, X,
  Shield, Globe, Server, Cpu, Wrench, ChevronDown, GripVertical,
  AlertTriangle, CheckCircle2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import { scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";
import { checkReconTools, type ReconToolCheck } from "@/lib/ai";
import { useStore } from "@/store";

type ExecMode = "pipe" | "sequential" | "parallel" | "on_success" | "on_failure";

const STEP_ICONS: Record<string, { icon: typeof Shield; color: string }> = {
  dns_lookup:       { icon: Globe,    color: "text-blue-400" },
  subdomain_enum:   { icon: Globe,    color: "text-cyan-400" },
  http_probe:       { icon: Globe,    color: "text-green-400" },
  port_scan:        { icon: Server,   color: "text-red-400" },
  tech_fingerprint: { icon: Cpu,      color: "text-purple-400" },
  js_collect:       { icon: Code2,    color: "text-amber-400" },
  js_harvest:       { icon: Download, color: "text-amber-500" },
  shell_command:    { icon: Wrench,   color: "text-muted-foreground/60" },
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
  iterate_over?: string | null;
  db_action?: string | null;
  x: number;
  y: number;
}

interface PipelineConnection { from_step: string; to_step: string; }
interface Pipeline {
  id: string; name: string; description: string; is_template: boolean;
  workflow_id?: string; steps: PipelineStep[]; connections: PipelineConnection[];
  created_at: number; updated_at: number;
}
type ToolWithMeta = ToolConfig & { categoryName?: string; subcategoryName?: string };

function uuid() { return Math.random().toString(36).slice(2, 10); }

/* ── Mini Dropdown ── */

function MiniDropdown({ value, onChange, options }: {
  value: string; onChange: (v: string) => void; options: { value: string; label: string }[];
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const selected = options.find((o) => o.value === value) ?? options[0];

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false); };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button type="button" onClick={() => setOpen(!open)} className="flex items-center gap-1 w-full px-1.5 py-[3px] text-[10px] rounded-md bg-white/[0.03] border border-white/[0.06] hover:border-white/[0.12] text-foreground/70 transition-colors">
        <span className="flex-1 text-left truncate">{selected.label}</span>
        <ChevronDown className={cn("w-2.5 h-2.5 text-muted-foreground/30 transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute z-50 mt-0.5 w-full min-w-[90px] rounded-md border border-white/[0.08] bg-[#1a1a1f] shadow-xl py-0.5 max-h-[180px] overflow-y-auto">
          {options.map((o) => (
            <button key={o.value} type="button" onClick={() => { onChange(o.value); setOpen(false); }} className={cn("w-full text-left px-2 py-1 text-[10px] transition-colors", o.value === value ? "bg-accent/15 text-accent" : "text-foreground/60 hover:bg-white/[0.05] hover:text-foreground")}>
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

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

/* ── Timeline Step Row ── */

function TimelineRow({ step, idx, total, isSkipped, isExpanded, toggleExpanded, updateStep, removeStep, allSteps, knownTypes }: {
  step: PipelineStep; idx: number; total: number;
  isSkipped: boolean; isExpanded: boolean;
  toggleExpanded: (id: string) => void;
  updateStep: (id: string, patch: Partial<PipelineStep>) => void;
  removeStep: (id: string) => void;
  allSteps: PipelineStep[]; knownTypes: string[];
}) {
  const meta = STEP_ICONS[step.step_type] || STEP_ICONS.shell_command;
  const StepIcon = meta.icon;
  const hasStorage = !!(step.db_action);
  const dbLabel = DB_ACTIONS.find((a) => a.value === step.db_action)?.label;
  const ts = step.requires ? typeStyle(step.requires) : null;
  const isLast = idx === total - 1;

  return (
    <div className={cn("flex gap-3 transition-all duration-200", isSkipped && "opacity-30")}>
      {/* Timeline rail */}
      <div className="flex flex-col items-center flex-shrink-0 w-8">
        <div className={cn(
          "w-6 h-6 rounded-full flex items-center justify-center text-[9px] font-bold border z-10",
          isSkipped
            ? "bg-white/[0.02] border-white/[0.06] text-muted-foreground/25"
            : ts
              ? `${ts.bg} ${ts.border} ${ts.text}`
              : "bg-accent/15 border-accent/20 text-accent",
        )}>
          {idx + 1}
        </div>
        {!isLast && <div className={cn("w-px flex-1 min-h-[12px]", isSkipped ? "bg-white/[0.03]" : "bg-white/[0.08]")} />}
      </div>

      {/* Card */}
      <div className={cn(
        "flex-1 mb-2 rounded-xl border transition-all min-w-[320px] max-w-[560px]",
        "bg-gradient-to-b from-white/[0.03] to-transparent",
        isSkipped
          ? "border-white/[0.03]"
          : "border-white/[0.06] hover:border-white/[0.12] shadow-[0_1px_6px_rgba(0,0,0,0.12)]",
      )}>
        {/* Header */}
        <div className="flex items-center gap-2.5 px-3 py-2 cursor-pointer select-none" onClick={() => toggleExpanded(step.id)}>
          <GripVertical className="w-3 h-3 text-muted-foreground/10 flex-shrink-0" />
          <div className={cn("w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0", `${meta.color.replace("text-", "bg-")}/10`)}>
            <StepIcon className={cn("w-3.5 h-3.5", meta.color)} />
          </div>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-[12px] font-semibold text-foreground/85">{step.tool_name}</span>
              {isSkipped && <span className="px-1.5 py-[1px] text-[7px] font-medium rounded bg-white/[0.05] text-muted-foreground/30 border border-white/[0.04] uppercase">Skip</span>}
            </div>
            <div className="text-[9px] text-muted-foreground/30 font-mono truncate">{step.command_template} {step.args.join(" ")}</div>
          </div>
          {/* Badges */}
          <div className="flex items-center gap-1.5 flex-shrink-0">
            {step.requires ? (
              <span className={cn("px-1.5 py-[2px] text-[8px] font-semibold rounded-md uppercase", ts!.bg, ts!.text, "border", ts!.border)}>
                {step.requires}
              </span>
            ) : (
              <span className="px-1.5 py-[2px] text-[8px] rounded-md bg-white/[0.04] text-muted-foreground/25 border border-white/[0.04]">any</span>
            )}
            {step.iterate_over && <span className="px-1 py-[1px] text-[8px] rounded bg-purple-500/10 text-purple-400/70 border border-purple-500/10">iter</span>}
            {hasStorage && (
              <span className="px-1.5 py-[1px] text-[8px] rounded bg-emerald-500/10 text-emerald-400/70 border border-emerald-500/10 flex items-center gap-0.5">
                <Database className="w-2 h-2" /> {dbLabel}
              </span>
            )}
          </div>
          <ChevronDown className={cn("w-3 h-3 text-muted-foreground/20 transition-transform flex-shrink-0", isExpanded && "rotate-180")} />
        </div>

        {/* Expanded editor */}
        {isExpanded && (
          <div className="px-3 pb-3 space-y-2.5 border-t border-white/[0.04] pt-2.5">
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="text-[9px] text-muted-foreground/30 font-medium uppercase tracking-wider">Command</label>
                <input value={step.command_template} onChange={(e) => updateStep(step.id, { command_template: e.target.value })} className="w-full mt-0.5 px-2 py-1 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 outline-none focus:border-accent/30 transition-colors" />
              </div>
              <div>
                <label className="text-[9px] text-muted-foreground/30 font-medium uppercase tracking-wider">Arguments</label>
                <input value={step.args.join(" ")} onChange={(e) => updateStep(step.id, { args: e.target.value.split(/\s+/).filter(Boolean) })} placeholder="-d {target} -silent" className="w-full mt-0.5 px-2 py-1 text-[10px] font-mono rounded-md bg-white/[0.03] border border-white/[0.06] text-foreground/80 placeholder:text-muted-foreground/15 outline-none focus:border-accent/30 transition-colors" />
              </div>
            </div>
            <div className="grid grid-cols-4 gap-1.5">
              <div>
                <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Requires</label>
                <RequiresInput value={step.requires || ""} onChange={(v) => updateStep(step.id, { requires: v || null })} knownTypes={knownTypes} />
              </div>
              <div>
                <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Input</label>
                <MiniDropdown value={step.input_from || ""} onChange={(v) => updateStep(step.id, { input_from: v || null })} options={[{ value: "", label: "Prev" }, ...allSteps.filter((s) => s.id !== step.id).map((s) => ({ value: s.id, label: s.tool_name }))]} />
              </div>
              <div>
                <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider">Iterate</label>
                <MiniDropdown value={step.iterate_over || ""} onChange={(v) => updateStep(step.id, { iterate_over: v || null })} options={ITERATE_OPTS} />
              </div>
              <div>
                <label className="text-[8px] text-muted-foreground/25 uppercase tracking-wider flex items-center gap-0.5"><Database className="w-2 h-2" /> Store</label>
                <MiniDropdown value={step.db_action || ""} onChange={(v) => updateStep(step.id, { db_action: v || null })} options={DB_ACTIONS} />
              </div>
            </div>
            <button onClick={() => removeStep(step.id)} className="w-full flex items-center justify-center gap-1 py-1 text-[9px] text-muted-foreground/25 hover:text-red-400 rounded-md hover:bg-red-500/5 transition-all">
              <Trash2 className="w-2.5 h-2.5" /> Remove Step
            </button>
          </div>
        )}
      </div>
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
  const [expandedSteps, setExpandedSteps] = useState<Set<string>>(new Set());
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
      setTools((tl?.tools || []).filter((t) => t.ui === "cli" && t.installed));
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
    setExpandedSteps((p) => new Set([...p, s.id]));
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

  const toggleExpanded = useCallback((id: string) => {
    setExpandedSteps((p) => { const n = new Set(p); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  }, []);

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
                <button key={p.id} onClick={() => { setActive(p); setDirty(false); setExpandedSteps(new Set()); }}
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

              {/* Timeline */}
              <div className="flex-1 overflow-auto px-5 py-4">
                {active.steps.length === 0 ? (
                  <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
                    <GitBranch className="w-8 h-8" /><p className="text-[11px]">Add tools to build your pipeline</p>
                  </div>
                ) : (
                  <div className="flex flex-col">
                    {active.steps.map((step, idx) => {
                      const skipped = !!(previewTargetType && step.requires && step.requires !== previewTargetType);
                      return (
                        <TimelineRow
                          key={step.id} step={step} idx={idx} total={active.steps.length}
                          isSkipped={skipped} isExpanded={expandedSteps.has(step.id)}
                          toggleExpanded={toggleExpanded} updateStep={updateStep} removeStep={removeStep}
                          allSteps={active.steps} knownTypes={knownTypes}
                        />
                      );
                    })}
                  </div>
                )}
              </div>

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
