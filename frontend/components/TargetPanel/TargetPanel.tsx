import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { QuickNotes } from "@/components/QuickNotes/QuickNotes";
import {
  ChevronDown, Crosshair, Globe, Hash, Map as MapIcon, Network,
  Play, Plus, Radar, Search, Server, Shield, ShieldOff, Tag, Trash2, Wifi, X, Zap,
} from "lucide-react";
import { TopologyView } from "@/components/TopologyView/TopologyView";
import { stripAllAnsi } from "@/lib/ansi";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";
import { scanTools, type ToolConfig } from "@/lib/pentest/api";
import { useCreateTerminalTab } from "@/hooks/useCreateTerminalTab";

type TargetStatus = "new" | "recon" | "recondone" | "scanning" | "tested";

interface PortInfo {
  port: number;
  protocol?: string;
  service?: string;
  state?: string;
}

interface Target {
  id: string;
  name: string;
  type: "domain" | "ip" | "cidr" | "url" | "wildcard";
  value: string;
  tags: string[];
  notes: string;
  scope: "in" | "out";
  group: string;
  status: TargetStatus;
  source: string;
  parent_id: string | null;
  ports: PortInfo[];
  technologies: string[];
  created_at: number;
  updated_at: number;
}

interface TargetStore {
  targets: Target[];
  groups: string[];
}

interface PipelineInfo {
  id: string;
  name: string;
  description: string;
  workflow_id?: string;
  steps: { id: string; command_template: string; tool_name: string; args: string[] }[];
}

interface ToolCheckResult {
  tools: { name: string; installed: boolean }[];
  all_ready: boolean;
  missing: string[];
}

const ALL_TOOLS_BY_TYPE: Record<string, string[]> = {
  domain: ["nmap", "subfinder", "httpx", "nuclei", "whatweb", "katana"],
  ip: ["nmap", "masscan", "rustscan", "nuclei"],
  cidr: ["nmap", "masscan", "rustscan"],
  url: ["nmap", "nikto", "ffuf", "gobuster", "dirsearch", "feroxbuster", "nuclei", "whatweb", "katana"],
  wildcard: ["subfinder", "httpx"],
};

const TYPE_ICONS: Record<string, React.ReactNode> = {
  domain: <Globe className="w-3.5 h-3.5 text-blue-400" />,
  ip: <Hash className="w-3.5 h-3.5 text-green-400" />,
  cidr: <Network className="w-3.5 h-3.5 text-yellow-400" />,
  url: <Globe className="w-3.5 h-3.5 text-purple-400" />,
  wildcard: <Crosshair className="w-3.5 h-3.5 text-orange-400" />,
};

const TYPE_LABELS: Record<string, string> = {
  domain: "targets.domain",
  ip: "targets.ip",
  cidr: "targets.cidr",
  url: "targets.url",
  wildcard: "targets.wildcard",
};

const STATUS_CONFIG: Record<TargetStatus, { label: string; color: string; bg: string }> = {
  new: { label: "New", color: "text-gray-400", bg: "bg-gray-500/10" },
  recon: { label: "Recon", color: "text-blue-400", bg: "bg-blue-500/10" },
  recondone: { label: "Recon Done", color: "text-cyan-400", bg: "bg-cyan-500/10" },
  scanning: { label: "Scanning", color: "text-yellow-400", bg: "bg-yellow-500/10" },
  tested: { label: "Tested", color: "text-green-400", bg: "bg-green-500/10" },
};

function MiniDropdown({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const selected = options.find((o) => o.value === value);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        className={cn(
          "flex items-center gap-1.5 px-2 py-1.5 text-xs rounded-md border transition-colors",
          "bg-[var(--bg-hover)]/30 border-border/30 text-foreground",
          "hover:bg-[var(--bg-hover)]/60 hover:border-border/50",
          open && "border-accent/40 bg-[var(--bg-hover)]/50",
        )}
        onClick={() => setOpen(!open)}
      >
        <span className="truncate max-w-[80px]">{selected?.label ?? value}</span>
        <ChevronDown className={cn("w-3 h-3 text-muted-foreground transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute top-full left-0 mt-1 min-w-[120px] max-h-48 overflow-y-auto rounded-lg border border-border/30 bg-card shadow-lg z-50 py-1">
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              className={cn(
                "w-full text-left px-3 py-1.5 text-xs transition-colors",
                "hover:bg-[var(--bg-hover)]/60",
                opt.value === value && "text-accent bg-accent/5 font-medium",
              )}
              onClick={() => { onChange(opt.value); setOpen(false); }}
            >
              {opt.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

type TargetTab = "targets" | "topology";

export function TargetPanel() {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [activeTab, setActiveTab] = useState<TargetTab>("targets");
  const [store, setStore] = useState<TargetStore>({ targets: [], groups: ["default"] });
  const [search, setSearch] = useState("");
  const [scopeFilter, setScopeFilter] = useState<"all" | "in" | "out">("all");
  const [groupFilter, setGroupFilter] = useState<string>("all");
  const [showAdd, setShowAdd] = useState(false);
  const [showBatch, setShowBatch] = useState(false);
  const [batchInput, setBatchInput] = useState("");
  const [addForm, setAddForm] = useState({ name: "", value: "", group: "default", notes: "", tags: "" });
  const [editingId, setEditingId] = useState<string | null>(null);
  const [showNewGroup, setShowNewGroup] = useState(false);
  const [newGroupName, setNewGroupName] = useState("");
  const [autoPipeline, setAutoPipeline] = useState(() => localStorage.getItem("golish-auto-pipeline") === "true");
  const [pipelineMenuOpen, setPipelineMenuOpen] = useState(false);
  const [pipelineList, setPipelineList] = useState<PipelineInfo[]>([]);
  const [toolCheck, setToolCheck] = useState<Map<string, boolean>>(new Map());
  const [selectedPipelineId, setSelectedPipelineId] = useState<string>(
    () => localStorage.getItem("golish-auto-pipeline-id") || ""
  );
  const pipelineMenuRef = useRef<HTMLDivElement>(null);
  const { createTerminalTab } = useCreateTerminalTab();

  const getToolCommand = useCallback((toolId: string, targetValue: string) => {
    const map: Record<string, string> = {
      nmap: `nmap -sC -sV ${targetValue}`,
      masscan: `masscan ${targetValue} -p1-65535 --rate=1000`,
      rustscan: `rustscan -a ${targetValue}`,
      subfinder: `subfinder -d ${targetValue}`,
      httpx: `echo "${targetValue}" | httpx`,
      nuclei: `nuclei -u ${targetValue}`,
      nikto: `nikto -h ${targetValue}`,
      ffuf: `ffuf -u ${targetValue}/FUZZ -w /usr/share/wordlists/common.txt`,
      gobuster: `gobuster dir -u ${targetValue} -w /usr/share/wordlists/common.txt`,
      dirsearch: `dirsearch -u ${targetValue}`,
      feroxbuster: `feroxbuster -u ${targetValue}`,
      whatweb: `whatweb ${targetValue}`,
      katana: `katana -u ${targetValue}`,
    };
    return map[toolId] || `${toolId} ${targetValue}`;
  }, []);

  const loadTargets = useCallback(async () => {
    try {
      const data = await invoke<TargetStore>("target_list", { projectPath: getProjectPath() });
      setStore(data);
    } catch (e) {
      console.error("Failed to load targets:", e);
    }
  }, []);

  useEffect(() => { loadTargets(); }, [loadTargets, currentProjectPath]);

  useEffect(() => {
    const REFRESH_TOOLS = new Set(["manage_targets", "record_finding"]);
    const unlisten = listen<{ type: string; tool_name?: string }>("ai-event", (event) => {
      if (event.payload.type === "tool_result" && event.payload.tool_name && REFRESH_TOOLS.has(event.payload.tool_name)) {
        loadTargets();
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [loadTargets]);

  useEffect(() => {
    async function loadPipelines() {
      try {
        const [pList, check] = await Promise.all([
          invoke<PipelineInfo[]>("pipeline_list", { projectPath: getProjectPath() }),
          invoke<ToolCheckResult>("check_recon_tools_cmd"),
        ]);
        setPipelineList(pList);

        const statusMap = new Map<string, boolean>();
        for (const t of check.tools) statusMap.set(t.name, t.installed);
        setToolCheck(statusMap);

        const storedId = localStorage.getItem("golish-auto-pipeline-id");
        if ((!storedId || !pList.find((p) => p.id === storedId)) && pList.length > 0) {
          const defaultId = pList.find((p) => p.workflow_id === "recon_basic")?.id ?? pList[0].id;
          setSelectedPipelineId(defaultId);
          localStorage.setItem("golish-auto-pipeline-id", defaultId);
        }
      } catch (e) {
        console.error("Failed to load pipelines:", e);
      }
    }
    loadPipelines();
  }, [currentProjectPath]);

  useEffect(() => {
    if (!pipelineMenuOpen) return;
    const handler = (e: MouseEvent) => {
      if (pipelineMenuRef.current && !pipelineMenuRef.current.contains(e.target as Node)) {
        setPipelineMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [pipelineMenuOpen]);

  const toggleAutoPipeline = useCallback(() => {
    setAutoPipeline((prev) => {
      const next = !prev;
      localStorage.setItem("golish-auto-pipeline", String(next));
      return next;
    });
  }, []);

  const sendPipelineResultsToAI = useCallback((
    sessionId: string,
    blockId: string,
    pipelineName: string,
    target: string,
  ) => {
    const store = useStore.getState();
    const convId = store.activeConversationId;
    if (!convId) return;

    const timeline = store.timelines[sessionId] ?? [];
    const pipelineBlock = timeline.find(
      (b) => b.type === "pipeline_progress" && b.id === blockId,
    );
    if (!pipelineBlock || pipelineBlock.type !== "pipeline_progress") return;

    const resultsText = pipelineBlock.data.steps
      .filter((s) => s.output?.trim())
      .map((s) => {
        const output = stripAllAnsi(s.output || "").slice(0, 2000);
        return `### \`${s.command}\` (exit: ${s.exitCode ?? "?"})\n\`\`\`\n${output}\n\`\`\``;
      })
      .join("\n\n");

    if (!resultsText) return;

    const analysisPrompt = `[Pipeline "${pipelineName}" completed for target: ${target}]\n\nPlease analyze these reconnaissance results and provide a brief summary of findings:\n\n${resultsText}`;

    window.dispatchEvent(new CustomEvent("pipeline-analysis-request", {
      detail: { convId, prompt: analysisPrompt },
    }));
  }, []);

  const runPipelineOnTarget = useCallback(async (targetValue: string) => {
    try {
      const pipelines = await invoke<PipelineInfo[]>("pipeline_list", { projectPath: getProjectPath() });
      const selected = selectedPipelineId ? pipelines.find((p) => p.id === selectedPipelineId) : null;
      const recon = selected ?? pipelines.find((p) => p.workflow_id === "recon_basic") ?? pipelines[0];
      if (!recon || recon.steps.length === 0) return;

      const steps = recon.steps
        .map((s) => {
          const cmd = s.command_template || "";
          const args = (s.args || []).join(" ");
          const command = `${cmd} ${args}`.replace(/\{target\}/g, targetValue).trim();
          return { stepId: s.id, name: s.tool_name || cmd, command };
        })
        .filter((s) => s.command);
      if (steps.length === 0) return;

      // Use the active conversation's terminal (1:1 model)
      const store = useStore.getState();
      const convId = store.activeConversationId;
      const convTerminals = convId ? store.conversationTerminals[convId] ?? [] : [];
      let sessionId: string = convTerminals[0] ?? "";

      if (!sessionId) {
        const newId = await createTerminalTab();
        if (!newId) return;
        sessionId = newId;
      }
      useStore.getState().setActiveSession(sessionId);
      window.dispatchEvent(new CustomEvent("close-activity-view"));

      // Create pipeline progress block
      const execution = {
        pipelineId: recon.id,
        pipelineName: recon.name || recon.id,
        target: targetValue,
        steps: steps.map((s) => ({
          stepId: s.stepId,
          name: s.name,
          command: s.command,
          status: "pending" as const,
        })),
        status: "running" as const,
        startedAt: new Date().toISOString(),
      };
      useStore.getState().startPipelineExecution(sessionId, execution);

      // Update conversation title to reflect the target being scanned
      if (convId) {
        useStore.getState().updateConversation(convId, { title: targetValue });
      }

      const timeline = useStore.getState().timelines[sessionId] ?? [];
      const progressBlock = timeline[timeline.length - 1];
      const blockId = progressBlock?.id ?? "";

      // Execute steps sequentially
      const { ptyWrite } = await import("@/lib/tauri");
      await new Promise((r) => setTimeout(r, 500));

      for (let i = 0; i < steps.length; i++) {
        const step = steps[i];

        useStore.getState().updatePipelineStep(sessionId, blockId, step.stepId, {
          status: "running",
          startedAt: new Date().toISOString(),
        });

        // Tag subsequent command blocks as pipeline-sourced
        useStore.getState().setPipelineCommandSource(sessionId, true);

        const cmdBlockCountBefore = (useStore.getState().timelines[sessionId] ?? [])
          .filter((b) => b.type === "command").length;

        await ptyWrite(sessionId, step.command + "\n");

        // Wait for the command to complete (new command block appears)
        const stepStartTime = Date.now();
        await new Promise<void>((resolve) => {
          const check = () => {
            const currentCmdCount = (useStore.getState().timelines[sessionId!] ?? [])
              .filter((b) => b.type === "command").length;
            if (currentCmdCount > cmdBlockCountBefore) {
              resolve();
              return;
            }
            if (Date.now() - stepStartTime > 120000) {
              resolve();
              return;
            }
            setTimeout(check, 300);
          };
          setTimeout(check, 500);
        });

        // Get exit code and output from the last command block
        const updatedTimeline = useStore.getState().timelines[sessionId] ?? [];
        const lastCmdBlock = [...updatedTimeline].reverse().find((b) => b.type === "command");
        const exitCode = lastCmdBlock?.type === "command" ? lastCmdBlock.data.exitCode : null;
        const cmdOutput = lastCmdBlock?.type === "command" ? lastCmdBlock.data.output : "";
        const success = exitCode === null || exitCode === 0;
        const finishedAt = new Date().toISOString();
        const durationMs = Date.now() - stepStartTime;

        useStore.getState().updatePipelineStep(sessionId, blockId, step.stepId, {
          status: success ? "success" : "failed",
          exitCode,
          finishedAt,
          durationMs,
          output: cmdOutput || "",
        });

        if (!success) {
          for (let j = i + 1; j < steps.length; j++) {
            useStore.getState().updatePipelineStep(sessionId, blockId, steps[j].stepId, {
              status: "skipped",
            });
          }
          useStore.getState().completePipelineExecution(sessionId, blockId, "failed");
          useStore.getState().setPipelineCommandSource(sessionId, false);
          sendPipelineResultsToAI(sessionId, blockId, recon.name || recon.id, targetValue);
          return;
        }
      }

      useStore.getState().completePipelineExecution(sessionId, blockId, "completed");
      useStore.getState().setPipelineCommandSource(sessionId, false);
      sendPipelineResultsToAI(sessionId, blockId, recon.name || recon.id, targetValue);
    } catch (e) {
      console.error("Failed to run pipeline:", e);
    }
  }, [createTerminalTab, selectedPipelineId]);

  const [addError, setAddError] = useState<string | null>(null);

  const handleAdd = useCallback(async () => {
    if (!addForm.value.trim()) return;
    setAddError(null);
    try {
      await invoke("target_add", {
        name: addForm.name,
        value: addForm.value.trim(),
        group: addForm.group || "default",
        notes: addForm.notes,
        tags: addForm.tags ? addForm.tags.split(",").map((s) => s.trim()).filter(Boolean) : [],
        projectPath: getProjectPath(),
      });
      const val = addForm.value.trim();
      setAddForm({ name: "", value: "", group: "default", notes: "", tags: "" });
      setShowAdd(false);
      loadTargets();
      emit("targets-changed").catch(() => {});
      invoke("audit_log", { action: "target_added", category: "targets", details: val, projectPath: getProjectPath() }).catch(() => {});
      if (autoPipeline) {
        runPipelineOnTarget(val);
      }
    } catch (e) {
      const msg = String(e);
      if (msg.includes("duplicate") || msg.includes("unique") || msg.includes("already exists")) {
        setAddError("Target already exists");
      } else {
        setAddError(msg.slice(0, 100));
      }
      console.error("Failed to add target:", e);
    }
  }, [addForm, loadTargets, autoPipeline, runPipelineOnTarget]);

  const handleBatchAdd = useCallback(async () => {
    if (!batchInput.trim()) return;
    try {
      const added = await invoke<Target[]>("target_batch_add", {
        values: batchInput,
        group: groupFilter !== "all" ? groupFilter : "default",
        projectPath: getProjectPath(),
      });
      setBatchInput("");
      setShowBatch(false);
      loadTargets();
      if (added.length > 0) {
        console.info(`Imported ${added.length} targets`);
      }
    } catch (e) {
      console.error("Failed to batch add:", e);
    }
  }, [batchInput, groupFilter, loadTargets]);

  const handleDelete = useCallback(async (id: string) => {
    try {
      await invoke("target_delete", { id, projectPath: getProjectPath() });
      loadTargets();
      invoke("audit_log", { action: "target_deleted", category: "targets", details: id, entityType: "target", entityId: id, projectPath: getProjectPath() }).catch(() => {});
    } catch (e) {
      console.error("Failed to delete target:", e);
    }
  }, [loadTargets]);

  const handleToggleScope = useCallback(async (target: Target) => {
    try {
      await invoke("target_update", {
        id: target.id,
        scope: target.scope === "in" ? "out" : "in",
        projectPath: getProjectPath(),
      });
      loadTargets();
    } catch (e) {
      console.error("Failed to update scope:", e);
    }
  }, [loadTargets]);

  const handleUpdateNotes = useCallback(async (id: string, notes: string) => {
    try {
      await invoke("target_update", { id, notes, projectPath: getProjectPath() });
      loadTargets();
    } catch (e) {
      console.error("Failed to update notes:", e);
    }
  }, [loadTargets]);

  const handleAddGroup = useCallback(async () => {
    if (!newGroupName.trim()) return;
    try {
      await invoke("target_add_group", { name: newGroupName.trim(), projectPath: getProjectPath() });
      setNewGroupName("");
      setShowNewGroup(false);
      loadTargets();
    } catch (e) {
      console.error("Failed to add group:", e);
    }
  }, [newGroupName, loadTargets]);

  const handleClearAll = useCallback(async () => {
    if (!confirm(t("targets.clearConfirm"))) return;
    try {
      await invoke("target_clear_all", { projectPath: getProjectPath() });
      loadTargets();
    } catch (e) {
      console.error("Failed to clear:", e);
    }
  }, [t, loadTargets]);

  const handleStartRecon = useCallback(async () => {
    const inScopeTargets = store.targets.filter((t) => t.scope === "in");
    for (const target of inScopeTargets) {
      await runPipelineOnTarget(target.value);
    }
  }, [store.targets, runPipelineOnTarget]);

  const filtered = useMemo(() => {
    let list = store.targets;
    if (search) {
      const q = search.toLowerCase();
      list = list.filter((t) =>
        t.name.toLowerCase().includes(q) ||
        t.value.toLowerCase().includes(q) ||
        t.tags.some((tag) => tag.toLowerCase().includes(q))
      );
    }
    if (scopeFilter !== "all") {
      list = list.filter((t) => t.scope === scopeFilter);
    }
    if (groupFilter !== "all") {
      list = list.filter((t) => t.group === groupFilter);
    }
    return list;
  }, [store.targets, search, scopeFilter, groupFilter]);

  const stats = useMemo(() => ({
    total: store.targets.length,
    inScope: store.targets.filter((t) => t.scope === "in").length,
    outOfScope: store.targets.filter((t) => t.scope === "out").length,
  }), [store.targets]);

  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; target: Target } | null>(null);
  const [tools, setTools] = useState<ToolConfig[]>([]);
  const ctxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scanTools().then((r) => setTools(r.tools || [])).catch(() => {});
  }, []);

  useEffect(() => {
    if (!ctxMenu) return;
    const close = (e: MouseEvent) => {
      if (ctxRef.current && !ctxRef.current.contains(e.target as Node)) setCtxMenu(null);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [ctxMenu]);

  const handleRunTool = useCallback(async (toolId: string, target: Target) => {
    const cmd = getToolCommand(toolId, target.value);
    const sessionId = await createTerminalTab();
    if (sessionId) {
      useStore.getState().setActiveSession(sessionId);
      window.dispatchEvent(new CustomEvent("close-activity-view"));
      const { ptyWrite } = await import("@/lib/tauri");
      setTimeout(async () => {
        await ptyWrite(sessionId, cmd + "\n").catch(() => {});
      }, 500);
    }
    setCtxMenu(null);
  }, [createTerminalTab, getToolCommand]);

  const contextToolList = useMemo(() => {
    if (!ctxMenu) return [];
    const relevantIds = ALL_TOOLS_BY_TYPE[ctxMenu.target.type] || [];
    return relevantIds.map((id) => {
      const match = tools.find((t) => t.id === id || t.name.toLowerCase() === id);
      return { id, name: match?.name || id, installed: !!match };
    });
  }, [ctxMenu, tools]);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2.5 border-b border-border/50">
        <div className="flex items-center gap-1">
          <button
            type="button"
            className={cn(
              "flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md transition-colors",
              activeTab === "targets"
                ? "bg-accent/15 text-accent font-medium"
                : "text-muted-foreground hover:text-foreground hover:bg-muted/40",
            )}
            onClick={() => setActiveTab("targets")}
          >
            <Crosshair className="w-3.5 h-3.5" />
            {t("targets.title")}
            <span className="text-[10px] text-muted-foreground/60 tabular-nums">{stats.total}</span>
          </button>
          <button
            type="button"
            className={cn(
              "flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md transition-colors",
              activeTab === "topology"
                ? "bg-accent/15 text-accent font-medium"
                : "text-muted-foreground hover:text-foreground hover:bg-muted/40",
            )}
            onClick={() => setActiveTab("topology")}
          >
            <MapIcon className="w-3.5 h-3.5" />
            {t("activity.topology")}
          </button>
        </div>
        {activeTab === "targets" && (
        <div className="flex items-center gap-1">
          <div className="relative" ref={pipelineMenuRef}>
            <button
              className={cn(
                "p-1.5 rounded transition-colors relative",
                autoPipeline
                  ? "bg-accent/15 text-accent hover:bg-accent/25"
                  : "text-muted-foreground hover:text-foreground hover:bg-muted/50"
              )}
              onClick={() => setPipelineMenuOpen(!pipelineMenuOpen)}
              title={autoPipeline ? "Auto-pipeline ON — click to configure" : "Auto-pipeline OFF — click to configure"}
            >
              <Radar className="w-3.5 h-3.5" />
              {autoPipeline && <span className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-accent" />}
            </button>

            {pipelineMenuOpen && (
              <div className="absolute top-full right-0 mt-1 w-[260px] rounded-lg border border-border/30 bg-card shadow-xl z-50 overflow-hidden">
                <div className="flex items-center gap-2 px-3 py-1.5 border-b border-border/20">
                  <Radar className="w-3 h-3 text-muted-foreground flex-shrink-0" />
                  <span className="text-[11px] font-medium text-foreground flex-1">Auto Pipeline</span>
                  <button
                    className={cn(
                      "w-7 h-3.5 rounded-full transition-colors relative flex-shrink-0",
                      autoPipeline ? "bg-accent" : "bg-muted-foreground/30"
                    )}
                    onClick={toggleAutoPipeline}
                  >
                    <span className={cn(
                      "absolute left-0 top-[3px] w-2 h-2 rounded-full bg-white shadow-sm transition-transform",
                      autoPipeline ? "translate-x-[16px]" : "translate-x-[2px]"
                    )} />
                  </button>
                </div>

                <div className="max-h-[300px] overflow-y-auto py-0.5">
                  {pipelineList.length === 0 ? (
                    <div className="px-3 py-3 text-[10px] text-muted-foreground text-center">No pipelines available</div>
                  ) : (
                    pipelineList.map((pipeline) => {
                      const isSelected = pipeline.id === selectedPipelineId;
                      const stepTools = pipeline.steps.map((s) => s.command_template).filter(Boolean);
                      const uniqueTools = [...new Set(stepTools)];
                      const toolAvail = uniqueTools.map((t) => ({ name: t, installed: toolCheck.get(t) ?? false }));
                      const allAvail = toolAvail.length > 0 && toolAvail.every((t) => t.installed);
                      const someAvail = toolAvail.some((t) => t.installed);

                      return (
                        <button
                          key={pipeline.id}
                          className={cn(
                            "w-full text-left px-3 py-2 hover:bg-muted/30 transition-colors",
                            isSelected && "bg-accent/5"
                          )}
                          onClick={() => {
                            setSelectedPipelineId(pipeline.id);
                            localStorage.setItem("golish-auto-pipeline-id", pipeline.id);
                          }}
                        >
                          <div className="flex items-center gap-1.5">
                            <span className={cn(
                              "w-2.5 h-2.5 rounded-full border-[1.5px] flex-shrink-0 flex items-center justify-center",
                              isSelected ? "border-accent" : "border-muted-foreground/30"
                            )}>
                              {isSelected && <span className="w-1 h-1 rounded-full bg-accent" />}
                            </span>
                            <span className="text-[11px] font-medium flex-1 truncate">{pipeline.name}</span>
                            <span className={cn(
                              "w-1.5 h-1.5 rounded-full flex-shrink-0",
                              allAvail ? "bg-green-400" : someAvail ? "bg-yellow-400" : "bg-red-400"
                            )} title={allAvail ? "All tools available" : `Missing: ${toolAvail.filter((t) => !t.installed).map((t) => t.name).join(", ")}`} />
                          </div>
                          {pipeline.description && (
                            <p className="text-[10px] text-muted-foreground/50 mt-0.5 ml-4 line-clamp-1">{pipeline.description}</p>
                          )}
                          <div className="flex flex-wrap gap-0.5 mt-1 ml-4">
                            {toolAvail.map((tool) => (
                              <span
                                key={tool.name}
                                className={cn(
                                  "text-[9px] px-1 py-px rounded font-mono",
                                  tool.installed
                                    ? "bg-green-500/10 text-green-400/80"
                                    : "bg-red-500/10 text-red-400/80"
                                )}
                              >
                                {tool.name}
                              </span>
                            ))}
                          </div>
                        </button>
                      );
                    })
                  )}
                </div>
              </div>
            )}
          </div>
          {stats.total > 0 && (
            <button
              className="p-1.5 rounded hover:bg-accent/20 text-muted-foreground hover:text-accent transition-colors"
              onClick={handleStartRecon}
              title="Run pipeline on all in-scope targets"
            >
              <Play className="w-3.5 h-3.5" />
            </button>
          )}
          <button
            className="p-1.5 rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
            onClick={() => { setShowBatch(true); setShowAdd(false); }}
            title={t("targets.batchAdd")}
          >
            <Hash className="w-3.5 h-3.5" />
          </button>
          <button
            className="p-1.5 rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
            onClick={() => { setShowAdd(true); setShowBatch(false); }}
            title={t("targets.addTarget")}
          >
            <Plus className="w-3.5 h-3.5" />
          </button>
          {stats.total > 0 && (
            <button
              className="p-1.5 rounded hover:bg-red-500/20 text-muted-foreground hover:text-red-400 transition-colors"
              onClick={handleClearAll}
              title={t("targets.clearAll")}
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          )}
        </div>
        )}
      </div>

      {activeTab === "topology" ? (
        <div className="flex-1 min-h-0"><TopologyView /></div>
      ) : (
      <>
      {/* Filter bar */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border/30">
        <div className="flex-1 relative">
          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
          <input
            className="w-full pl-7 pr-2 py-1.5 text-xs bg-background border border-border/50 rounded focus:border-accent outline-none"
            placeholder={t("common.search")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <MiniDropdown
          value={scopeFilter}
          onChange={(v) => setScopeFilter(v as "all" | "in" | "out")}
          options={[
            { value: "all", label: t("targets.all") },
            { value: "in", label: t("targets.inScope") },
            { value: "out", label: t("targets.outOfScope") },
          ]}
        />
        <div className="flex items-center gap-1">
          <MiniDropdown
            value={groupFilter}
            onChange={setGroupFilter}
            options={[
              { value: "all", label: t("targets.allGroups") },
              ...store.groups.map((g) => ({ value: g, label: g })),
            ]}
          />
          <button
            className="p-1 rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground"
            onClick={() => setShowNewGroup(true)}
            title={t("targets.newGroup")}
          >
            <Plus className="w-3 h-3" />
          </button>
        </div>
      </div>

      {/* New group dialog (inline) */}
      {showNewGroup && (
        <div className="px-4 py-2 border-b border-border/30 bg-muted/20 flex items-center gap-2">
          <input
            className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
            placeholder={t("targets.groupName")}
            value={newGroupName}
            onChange={(e) => setNewGroupName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") handleAddGroup(); if (e.key === "Escape") setShowNewGroup(false); }}
            autoFocus
          />
          <button className="text-xs text-accent hover:underline" onClick={handleAddGroup}>{t("common.confirm")}</button>
          <button className="text-xs text-muted-foreground hover:text-foreground" onClick={() => setShowNewGroup(false)}>{t("common.cancel")}</button>
        </div>
      )}

      {/* Add single target form */}
      {showAdd && (
        <div className="px-4 py-3 border-b border-border/30 bg-muted/10 space-y-2">
          <div className="flex items-center gap-2">
            <input
              className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
              placeholder={`${t("targets.value")} *  (e.g. example.com, 192.168.1.0/24, https://...)`}
              value={addForm.value}
              onChange={(e) => setAddForm((f) => ({ ...f, value: e.target.value }))}
              onKeyDown={(e) => { if (e.key === "Enter") handleAdd(); if (e.key === "Escape") setShowAdd(false); }}
              autoFocus
            />
          </div>
          <div className="flex items-center gap-2">
            <input
              className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
              placeholder={`${t("targets.name")} (${t("common.default")}: ${t("targets.value")})`}
              value={addForm.name}
              onChange={(e) => setAddForm((f) => ({ ...f, name: e.target.value }))}
            />
            <MiniDropdown
              value={addForm.group}
              onChange={(v) => setAddForm((f) => ({ ...f, group: v }))}
              options={store.groups.map((g) => ({ value: g, label: g }))}
            />
          </div>
          <div className="flex items-center gap-2">
            <input
              className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
              placeholder={`${t("targets.tags")} (comma separated)`}
              value={addForm.tags}
              onChange={(e) => setAddForm((f) => ({ ...f, tags: e.target.value }))}
            />
            <input
              className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
              placeholder={t("targets.notes")}
              value={addForm.notes}
              onChange={(e) => setAddForm((f) => ({ ...f, notes: e.target.value }))}
            />
          </div>
          {addError && (
            <p className="text-[11px] text-red-400">{addError}</p>
          )}
          <div className="flex justify-end gap-2">
            <button
              className="px-3 py-1 text-xs rounded bg-muted/50 hover:bg-muted text-foreground"
              onClick={() => { setShowAdd(false); setAddError(null); }}
            >{t("common.cancel")}</button>
            <button
              className="px-3 py-1 text-xs rounded bg-accent text-accent-foreground hover:bg-accent/90"
              onClick={handleAdd}
            >{t("targets.addTarget")}</button>
          </div>
        </div>
      )}

      {/* Batch import */}
      {showBatch && (
        <div className="px-4 py-3 border-b border-border/30 bg-muted/10 space-y-2">
          <textarea
            className="w-full h-32 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent resize-none font-mono"
            placeholder={t("targets.batchPlaceholder")}
            value={batchInput}
            onChange={(e) => setBatchInput(e.target.value)}
            autoFocus
          />
          <div className="flex justify-end gap-2">
            <button
              className="px-3 py-1 text-xs rounded bg-muted/50 hover:bg-muted text-foreground"
              onClick={() => setShowBatch(false)}
            >{t("common.cancel")}</button>
            <button
              className="px-3 py-1 text-xs rounded bg-accent text-accent-foreground hover:bg-accent/90"
              onClick={handleBatchAdd}
            >{t("targets.batchAdd")}</button>
          </div>
        </div>
      )}

      {/* Stats bar */}
      {stats.total > 0 && (
        <div className="flex items-center gap-3 px-4 py-1.5 border-b border-border/20 text-[11px] text-muted-foreground">
          <span className="flex items-center gap-1">
            <Shield className="w-3 h-3 text-green-400" />
            {stats.inScope} {t("targets.inScope")}
          </span>
          <span className="flex items-center gap-1">
            <ShieldOff className="w-3 h-3 text-red-400" />
            {stats.outOfScope} {t("targets.outOfScope")}
          </span>
        </div>
      )}

      {/* Target list */}
      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
            <Crosshair className="w-8 h-8 mb-2 opacity-30" />
            <p className="text-xs">{t("targets.noTargets")}</p>
          </div>
        ) : (
          <div className="divide-y divide-border/20">
            {filtered.map((target) => (
              <div
                key={target.id}
                className={cn(
                  "px-4 py-2.5 hover:bg-muted/30 transition-colors group cursor-pointer",
                  target.scope === "out" && "opacity-50",
                  editingId === target.id && "bg-muted/20",
                )}
                onClick={() => setEditingId(editingId === target.id ? null : target.id)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setCtxMenu({ x: e.clientX, y: e.clientY, target });
                }}
              >
                <div className="flex items-center gap-2">
                  {/* Type icon */}
                  {TYPE_ICONS[target.type] || <Globe className="w-3.5 h-3.5" />}

                  {/* Scope indicator */}
                  <button
                    className={cn(
                      "p-0.5 rounded transition-colors",
                      target.scope === "in"
                        ? "text-green-400 hover:text-green-300"
                        : "text-red-400 hover:text-red-300",
                    )}
                    onClick={(e) => { e.stopPropagation(); handleToggleScope(target); }}
                    title={target.scope === "in" ? t("targets.inScope") : t("targets.outOfScope")}
                  >
                    {target.scope === "in" ? <Shield className="w-3 h-3" /> : <ShieldOff className="w-3 h-3" />}
                  </button>

                  {/* Value */}
                  <span className="text-xs font-mono text-foreground flex-1 truncate">{target.value}</span>

                  {/* Status badge */}
                  {target.status && target.status !== "new" && (() => {
                    const cfg = STATUS_CONFIG[target.status] || STATUS_CONFIG.new;
                    return (
                      <span className={cn("text-[10px] px-1.5 py-0.5 rounded font-medium", cfg.color, cfg.bg)}>
                        {cfg.label}
                      </span>
                    );
                  })()}

                  {/* Port count indicator */}
                  {target.ports && target.ports.length > 0 && (
                    <span className="flex items-center gap-0.5 text-[10px] text-emerald-400/80" title={`${target.ports.length} open port(s)`}>
                      <Wifi className="w-2.5 h-2.5" />
                      {target.ports.length}
                    </span>
                  )}

                  {/* Type label */}
                  <span className="text-[10px] text-muted-foreground px-1.5 py-0.5 rounded bg-muted/30">
                    {t(TYPE_LABELS[target.type] || target.type)}
                  </span>

                  {/* Group */}
                  {target.group !== "default" && (
                    <span className="text-[10px] text-muted-foreground px-1.5 py-0.5 rounded bg-accent/10 text-accent">
                      {target.group}
                    </span>
                  )}

                  {/* Delete */}
                  <button
                    className="p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-red-500/20 text-muted-foreground hover:text-red-400 transition-all"
                    onClick={(e) => { e.stopPropagation(); handleDelete(target.id); }}
                  >
                    <X className="w-3 h-3" />
                  </button>
                </div>

                {/* Tags */}
                {target.tags.length > 0 && (
                  <div className="flex items-center gap-1 mt-1 ml-5">
                    <Tag className="w-2.5 h-2.5 text-muted-foreground" />
                    {target.tags.map((tag) => (
                      <span key={tag} className="text-[10px] px-1.5 py-0.5 rounded bg-muted/40 text-muted-foreground">
                        {tag}
                      </span>
                    ))}
                  </div>
                )}

                {/* Expanded details */}
                {editingId === target.id && (
                  <div className="mt-2 ml-5 space-y-1.5" onClick={(e) => e.stopPropagation()}>
                    {target.name !== target.value && (
                      <div className="text-[11px] text-muted-foreground">
                        <span className="font-medium">{t("targets.name")}:</span> {target.name}
                      </div>
                    )}

                    {/* Source */}
                    {target.source && target.source !== "manual" && (
                      <div className="text-[11px] text-muted-foreground">
                        <span className="font-medium">Source:</span> {target.source}
                      </div>
                    )}

                    {/* Technologies */}
                    {target.technologies && target.technologies.length > 0 && (
                      <div className="flex flex-wrap items-center gap-1">
                        <Zap className="w-3 h-3 text-purple-400 shrink-0" />
                        {target.technologies.map((tech) => (
                          <span key={tech} className="text-[10px] px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400">
                            {tech}
                          </span>
                        ))}
                      </div>
                    )}

                    {/* Ports */}
                    {target.ports && target.ports.length > 0 && (
                      <div className="space-y-0.5">
                        <div className="flex items-center gap-1 text-[11px] text-muted-foreground font-medium">
                          <Server className="w-3 h-3" />
                          Open Ports ({target.ports.length})
                        </div>
                        <div className="grid grid-cols-2 gap-x-3 gap-y-0.5 pl-4">
                          {target.ports.slice(0, 20).map((p: PortInfo) => (
                            <div key={`${p.port}-${p.protocol || ""}`} className="text-[10px] font-mono text-muted-foreground">
                              <span className="text-emerald-400">{p.port}</span>
                              {p.protocol && <span className="text-muted-foreground/50">/{p.protocol}</span>}
                              {p.service && <span className="text-foreground/60 ml-1">{p.service}</span>}
                            </div>
                          ))}
                          {target.ports.length > 20 && (
                            <div className="text-[10px] text-muted-foreground/50">+{target.ports.length - 20} more</div>
                          )}
                        </div>
                      </div>
                    )}

                    <textarea
                      className="w-full text-[11px] bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent resize-none"
                      placeholder={t("targets.notes")}
                      rows={2}
                      defaultValue={target.notes}
                      onBlur={(e) => {
                        if (e.target.value !== target.notes) {
                          handleUpdateNotes(target.id, e.target.value);
                        }
                      }}
                    />
                    <QuickNotes entityType="target" entityId={target.id} compact />
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {ctxMenu && createPortal(
        <div
          ref={ctxRef}
          className="fixed z-[9999] min-w-[180px] rounded-lg border border-border/30 bg-[#1e1e2e] shadow-2xl py-1 overflow-hidden"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
        >
          <div className="px-3 py-1.5 text-[10px] text-muted-foreground/40 font-medium uppercase tracking-wide">
            Run tool on {ctxMenu.target.value}
          </div>
          <div className="border-t border-border/15 my-0.5" />
          {contextToolList.length === 0 ? (
            <div className="px-3 py-2 text-[10px] text-muted-foreground/30">No tools available</div>
          ) : (
            contextToolList.map((tool) => (
              <button
                key={tool.id}
                onClick={() => handleRunTool(tool.id, ctxMenu.target)}
                className="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] text-foreground/80 hover:bg-accent/10 transition-colors"
              >
                <Play className="w-3 h-3 text-accent/60" />
                <span className="flex-1 text-left">{tool.name}</span>
              </button>
            ))
          )}
        </div>,
        document.body,
      )}
      </>
      )}
    </div>
  );
}
