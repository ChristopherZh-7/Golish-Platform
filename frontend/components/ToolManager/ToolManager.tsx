import { useCallback, useEffect, useState, useRef } from "react";
import {
  ArrowLeft, ArrowUpDown, Check, Code2, Copy, Download, FileText,
  FolderOpen, Grid3X3, Loader2, List, Plus, RefreshCw, Save, Search, Trash2, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { scanTools, deleteTool, getCategories, fetchGitHubRelease, downloadAndExtract, installRuntime } from "@/lib/pentest/api";
import type { ToolConfig, ToolCategory } from "@/lib/pentest/types";
import { getSettings } from "@/lib/settings";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

type ToolWithMeta = ToolConfig & { categoryName?: string; subcategoryName?: string };
type ViewMode = "grid" | "list";
type SortKey = "name" | "status" | "category" | "runtime";

export function ToolManager() {
  const [tools, setTools] = useState<ToolWithMeta[]>([]);
  const [categories, setCategories] = useState<ToolCategory[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [sortKey, setSortKey] = useState<SortKey>("name");

  // Editor state
  const [editingTool, setEditingTool] = useState<ToolWithMeta | null>(null);
  const [editorMode, setEditorMode] = useState<"form" | "raw">("form");
  const [rawJson, setRawJson] = useState("");
  const [formData, setFormData] = useState<Record<string, unknown>>({});
  const [editorLoading, setEditorLoading] = useState(false);
  const [editorDirty, setEditorDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const [editorVisible, setEditorVisible] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const originalJsonRef = useRef("");

  // Context menu
  const [ctxMenu, setCtxMenu] = useState<{ tool: ToolWithMeta; x: number; y: number } | null>(null);
  const [uninstallTarget, setUninstallTarget] = useState<ToolWithMeta | null>(null);

  // Install progress
  const [installProgress, setInstallProgress] = useState<Record<string, string>>({});

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [scanResult, cats] = await Promise.all([
        scanTools().catch(() => ({ success: false, tools: [] as ToolConfig[] })),
        getCategories().catch(() => [] as ToolCategory[]),
      ]);
      setCategories(cats);
      const catMap = new Map<string, string>();
      const subMap = new Map<string, string>();
      for (const c of cats) {
        catMap.set(c.id, c.name);
        for (const s of c.items) subMap.set(`${c.id}/${s.id}`, s.name);
      }
      const enriched: ToolWithMeta[] = (scanResult.tools || []).map((t) => ({
        ...t,
        categoryName: catMap.get(t.category) || t.category,
        subcategoryName: subMap.get(`${t.category}/${t.subcategory}`) || t.subcategory,
      }));
      setTools(enriched);
    } catch (e) {
      setError(`加载失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { loadData(); }, [loadData]);
  useEffect(() => {
    const dismiss = () => setCtxMenu(null);
    window.addEventListener("click", dismiss);
    return () => window.removeEventListener("click", dismiss);
  }, []);

  const getProxy = useCallback(async () => {
    const s = await getSettings().catch(() => null);
    return s?.network?.proxy_url || undefined;
  }, []);

  const handleInstall = useCallback(async (tool: ToolWithMeta) => {
    const method = tool.install?.method;
    if (!method) { setError(`工具 ${tool.name} 没有配置安装方式`); return; }
    setBusy(tool.id);
    setInstallProgress((p) => ({ ...p, [tool.id]: "准备中..." }));
    setError(null);
    try {
      const proxyUrl = await getProxy();
      if (method === "github") {
        const source = tool.install?.source;
        if (!source) throw new Error("缺少 GitHub 仓库地址");
        const [owner, repo] = source.split("/");
        if (!owner || !repo) throw new Error("GitHub source 格式错误");
        setInstallProgress((p) => ({ ...p, [tool.id]: "获取版本..." }));
        const release = await fetchGitHubRelease(owner, repo);
        const platform = navigator.platform.toLowerCase();
        const isMac = platform.includes("mac") || platform.includes("darwin");
        const asset = release.assets.find((a) => {
          const n = a.name.toLowerCase();
          if (isMac) return n.includes("darwin") || n.includes("macos") || n.includes("mac") || n.includes("osx");
          return n.includes("linux");
        }) || release.assets.find((a) => {
          const n = a.name.toLowerCase();
          return n.endsWith(".zip") || n.endsWith(".tar.gz") || n.endsWith(".tgz");
        });
        let downloadUrl: string, fileName: string;
        if (!asset) {
          downloadUrl = `https://github.com/${source}/archive/refs/tags/${release.tag_name}.zip`;
          fileName = `${repo}-${release.tag_name}.zip`;
        } else {
          downloadUrl = asset.browser_download_url;
          fileName = asset.name;
        }
        setInstallProgress((p) => ({ ...p, [tool.id]: "下载中..." }));
        const result = await downloadAndExtract({ url: downloadUrl, fileName, useProxy: !!proxyUrl });
        if (!result.success) throw new Error(result.error || "下载失败");
        setInstallProgress((p) => ({ ...p, [tool.id]: "安装中..." }));
        if (result.extract_path && tool.executable) {
          const toolDir = tool.executable.split("/")[0];
          if (toolDir && !result.extract_path.endsWith(`/${toolDir}`)) {
            try {
              await invoke("pentest_rename_tool_dir", { fromPath: result.extract_path, toName: toolDir });
            } catch { /* ignore */ }
          }
        }
      } else if (method === "homebrew") {
        setInstallProgress((p) => ({ ...p, [tool.id]: "Homebrew 安装中..." }));
        const pkg = tool.install?.source || tool.name;
        await installRuntime(`brew:${pkg}`, proxyUrl);
      }
      await loadData();
    } catch (e) {
      setError(`安装失败: ${e}`);
    } finally {
      setBusy(null);
      setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
    }
  }, [getProxy, loadData]);

  const doUninstall = useCallback(async (tool: ToolWithMeta) => {
    setBusy(tool.id);
    try {
      const toolDir = tool.executable.split("/")[0];
      if (toolDir) await invoke("pentest_uninstall_tool_files", { toolDir });
      await loadData();
    } catch (e) {
      setError(`卸载失败: ${e}`);
    } finally {
      setBusy(null);
    }
  }, [loadData]);

  const handleUninstall = useCallback((tool: ToolWithMeta) => {
    setUninstallTarget(tool);
  }, []);

  const confirmUninstall = useCallback(async () => {
    if (!uninstallTarget) return;
    setUninstallTarget(null);
    await doUninstall(uninstallTarget);
  }, [uninstallTarget, doUninstall]);

  // Editor functions (must be defined before handleAddTool and ctxAction which reference them)
  const openEditor = useCallback(async (tool: ToolWithMeta) => {
    setEditingTool(tool);
    setEditorMode("form");
    setEditorDirty(false);
    setEditorLoading(true);
    requestAnimationFrame(() => setEditorVisible(true));
    try {
      const content: string = await invoke("pentest_read_tool_config", {
        category: tool.category, subcategory: tool.subcategory, toolId: tool.id,
      });
      originalJsonRef.current = content;
      setRawJson(content);
      setFormData((JSON.parse(content)).tool || JSON.parse(content));
    } catch {
      const fallback: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(tool)) {
        if (k !== "categoryName" && k !== "subcategoryName" && k !== "installed") fallback[k] = v;
      }
      const json = JSON.stringify({ tool: fallback }, null, 2);
      originalJsonRef.current = json;
      setRawJson(json);
      setFormData(fallback);
    } finally {
      setEditorLoading(false);
    }
  }, []);

  const animateClose = useCallback(() => {
    setEditorVisible(false);
    setTimeout(() => { setEditingTool(null); setEditorDirty(false); }, 180);
  }, []);

  const closeEditor = useCallback(() => {
    if (editorDirty) { setShowCloseConfirm(true); return; }
    animateClose();
  }, [editorDirty, animateClose]);

  const forceCloseEditor = useCallback(() => {
    setShowCloseConfirm(false);
    animateClose();
  }, [animateClose]);

  const syncFormToRaw = useCallback((data: Record<string, unknown>) => {
    setRawJson(JSON.stringify({ tool: data }, null, 2));
  }, []);

  const syncRawToForm = useCallback((json: string) => {
    try { const p = JSON.parse(json); setFormData(p.tool || p); } catch { /* ignore */ }
  }, []);

  const handleFormChange = useCallback((field: string, value: unknown) => {
    setFormData((prev) => {
      if (JSON.stringify(prev[field]) === JSON.stringify(value)) return prev;
      const next = { ...prev, [field]: value };
      syncFormToRaw(next);
      setEditorDirty(true);
      return next;
    });
  }, [syncFormToRaw]);

  const handleRawChange = useCallback((value: string) => {
    setRawJson(value);
    setEditorDirty(true);
    syncRawToForm(value);
  }, [syncRawToForm]);

  const handleSave = useCallback(async () => {
    if (!editingTool) return;
    setSaving(true);
    setError(null);
    try {
      let content: string;
      if (editorMode === "raw") { JSON.parse(rawJson); content = rawJson; }
      else { content = JSON.stringify({ tool: formData }, null, 2); }
      await invoke("pentest_save_tool_config", {
        category: editingTool.category, subcategory: editingTool.subcategory,
        toolId: editingTool.id, content,
      });
      setEditorDirty(false);
      await loadData();
    } catch (e) {
      setError(`保存失败: ${e}`);
    } finally {
      setSaving(false);
    }
  }, [editingTool, editorMode, rawJson, formData, loadData]);

  const handleSwitchMode = useCallback((mode: "form" | "raw") => {
    if (mode === "raw") syncFormToRaw(formData);
    else syncRawToForm(rawJson);
    setEditorMode(mode);
  }, [editorMode, formData, rawJson, syncFormToRaw, syncRawToForm]);

  // Add new tool
  const handleAddTool = useCallback(async () => {
    const id = Math.random().toString(36).substring(2, 10);
    const newTool = {
      tool: {
        id,
        name: "新工具",
        description: "",
        icon: "🔧",
        executable: "",
        runtime: "native",
        runtimeVersion: "",
        ui: "cli",
        params: [],
        install: { method: "", source: "" },
      },
    };
    const content = JSON.stringify(newTool, null, 2);
    try {
      await invoke("pentest_save_tool_config", {
        category: "misc",
        subcategory: "other",
        toolId: id,
        content,
      });
      await loadData();
    } catch (e) {
      setError(`创建失败: ${e}`);
    }
  }, [loadData]);

  // Context menu
  const handleContextMenu = useCallback((e: React.MouseEvent, tool: ToolWithMeta) => {
    e.preventDefault();
    e.stopPropagation();
    setCtxMenu({ tool, x: e.clientX, y: e.clientY });
  }, []);

  const ctxAction = useCallback((action: string) => {
    if (!ctxMenu) return;
    const tool = ctxMenu.tool;
    setCtxMenu(null);
    switch (action) {
      case "edit": openEditor(tool); break;
      case "uninstall": handleUninstall(tool); break;
      case "install": handleInstall(tool); break;
      case "copy-id": navigator.clipboard.writeText(tool.id); break;
      case "open-dir":
        invoke("pentest_open_tool_dir", { toolDir: tool.executable.split("/")[0] || tool.name }).catch(() => {});
        break;
    }
  }, [ctxMenu, openEditor, handleUninstall, handleInstall]);

  // Filtering and sorting
  const installedCount = tools.filter((t) => t.installed).length;
  const allCategories = Array.from(new Set(tools.map((t) => t.category)));

  const filteredTools = tools
    .filter((t) => {
      if (selectedCategory && t.category !== selectedCategory) return false;
      if (!search.trim()) return true;
      const q = search.trim().toLowerCase();
      return t.name.toLowerCase().includes(q) || t.description.toLowerCase().includes(q) || t.id.toLowerCase().includes(q);
    })
    .sort((a, b) => {
      switch (sortKey) {
        case "status": return (b.installed ? 1 : 0) - (a.installed ? 1 : 0);
        case "category": return (a.categoryName || "").localeCompare(b.categoryName || "");
        case "runtime": return a.runtime.localeCompare(b.runtime);
        default: return a.name.localeCompare(b.name);
      }
    });

  const categoryDisplayName = (catId: string) => categories.find((c) => c.id === catId)?.name || catId;

  const runtimeBadge = (runtime: string) => {
    const m: Record<string, string> = {
      java: "bg-orange-500/15 text-orange-400",
      python: "bg-blue-500/15 text-blue-400",
      node: "bg-green-500/15 text-green-400",
      native: "bg-zinc-500/15 text-zinc-400",
    };
    return m[runtime] || "bg-muted/50 text-muted-foreground";
  };

  const installMethodBadge = (method: string) => {
    const m: Record<string, string> = {
      github: "bg-purple-500/12 text-purple-400",
      homebrew: "bg-amber-500/12 text-amber-400",
    };
    return m[method] || "bg-muted/30 text-muted-foreground/50";
  };

  const installMethodLabel = (tool: ToolWithMeta) => {
    const method = tool.install?.method;
    if (!method || method === "manual") return "手动";
    if (method === "github") return "GitHub";
    if (method === "homebrew") return "Homebrew";
    return method;
  };

  // Editor sub-components
  const InlineSelect = ({ value, onChange, options }: {
    value: string; onChange: (v: string) => void;
    options: { value: string; label: string }[];
  }) => (
    <Select value={value || undefined} onValueChange={onChange}>
      <SelectTrigger size="sm" className="flex-1 h-7 border-transparent bg-transparent hover:border-border/20 text-[12px] shadow-none px-2 gap-1">
        <SelectValue placeholder="选择..." />
      </SelectTrigger>
      <SelectContent position="popper" className="min-w-[120px]">
        {options.map((o) => <SelectItem key={o.value} value={o.value || "_none"} className="text-[12px]">{o.label}</SelectItem>)}
      </SelectContent>
    </Select>
  );

  const FieldRow = ({ label, field, placeholder, mono, type = "text", options }: {
    label: string; field: string; placeholder?: string; mono?: boolean;
    type?: "text" | "select"; options?: { value: string; label: string }[];
  }) => {
    const val = (formData[field] as string) ?? "";
    return (
      <div className="flex items-center gap-3 py-2 px-3 rounded-lg hover:bg-[var(--bg-hover)]/30 transition-colors">
        <span className="text-[12px] text-muted-foreground/60 w-24 flex-shrink-0">{label}</span>
        {type === "select" && options ? (
          <InlineSelect value={val} onChange={(v) => handleFormChange(field, v === "_none" ? "" : v)} options={options} />
        ) : (
          <input type="text" value={val} onChange={(e) => handleFormChange(field, e.target.value)} placeholder={placeholder}
            className={cn("flex-1 h-7 px-2 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors", mono && "font-mono text-[11px]")} />
        )}
      </div>
    );
  };

  const InstallFieldRow = ({ label, subField, placeholder, mono, type = "text", options }: {
    label: string; subField: string; placeholder?: string; mono?: boolean;
    type?: "text" | "select"; options?: { value: string; label: string }[];
  }) => {
    const install = (formData.install as Record<string, string>) || {};
    const val = install[subField] || "";
    const onChange = (v: string) => handleFormChange("install", { ...install, [subField]: v === "_none" ? "" : v });
    return (
      <div className="flex items-center gap-3 py-2 px-3 rounded-lg hover:bg-[var(--bg-hover)]/30 transition-colors">
        <span className="text-[12px] text-muted-foreground/60 w-24 flex-shrink-0">{label}</span>
        {type === "select" && options ? (
          <InlineSelect value={val} onChange={onChange} options={options} />
        ) : (
          <input type="text" value={val} onChange={(e) => onChange(e.target.value)} placeholder={placeholder}
            className={cn("flex-1 h-7 px-2 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors", mono && "font-mono text-[11px]")} />
        )}
      </div>
    );
  };

  const ParamsEditor = () => {
    const params = (formData.params as Array<Record<string, unknown>>) || [];
    const updateParam = (idx: number, key: string, value: unknown) => {
      const next = [...params]; next[idx] = { ...next[idx], [key]: value };
      handleFormChange("params", next);
    };
    const removeParam = (idx: number) => handleFormChange("params", params.filter((_, i) => i !== idx));
    const addParam = () => handleFormChange("params", [...params, { label: "", flag: "", type: "string" }]);

    return (
      <div className="px-3">
        {params.length === 0 ? (
          <p className="text-[12px] text-muted-foreground/25 py-3 text-center">暂无参数</p>
        ) : (
          <div className="space-y-1">
            {params.map((p, i) => (
              <div key={i} className="flex items-center gap-2 py-1.5 group/param rounded-lg hover:bg-[var(--bg-hover)]/30 px-1">
                <input value={(p.label as string) || ""} onChange={(e) => updateParam(i, "label", e.target.value)}
                  placeholder="标签" className="flex-[3] h-7 px-2 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors" />
                <input value={(p.flag as string) || ""} onChange={(e) => updateParam(i, "flag", e.target.value)}
                  placeholder="--flag" className="flex-[2] h-7 px-2 text-[11px] font-mono rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors" />
                <div className="flex-[1.5]">
                  <Select value={(p.type as string) || "string"} onValueChange={(v) => updateParam(i, "type", v)}>
                    <SelectTrigger size="sm" className="h-7 border-transparent bg-transparent hover:border-border/20 text-[11px] shadow-none px-2 gap-1 w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent position="popper" className="min-w-[100px]">
                      {["string", "number", "boolean", "file"].map((t) => <SelectItem key={t} value={t} className="text-[12px]">{t}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
                <button type="button" onClick={() => removeParam(i)}
                  className="p-0.5 text-muted-foreground/15 opacity-0 group-hover/param:opacity-100 hover:text-destructive transition-all flex-shrink-0">
                  <X className="w-3 h-3" />
                </button>
              </div>
            ))}
          </div>
        )}
        <button type="button" onClick={addParam}
          className="flex items-center gap-1 text-[11px] text-accent/60 hover:text-accent transition-colors mt-2 px-1">
          <Plus className="w-3 h-3" /> 添加参数
        </button>
      </div>
    );
  };

  // Tool card (used in both grid and list views)
  const ActionButton = ({ tool }: { tool: ToolWithMeta }) => {
    const isBusy = busy === tool.id;
    const progress = installProgress[tool.id];
    return (
      <div className="flex items-center gap-1 flex-shrink-0" onClick={(e) => e.stopPropagation()}>
        {isBusy ? (
          <div className="flex items-center gap-1.5">
            <Loader2 className="w-3.5 h-3.5 animate-spin text-accent/60" />
            {progress && <span className="text-[10px] text-accent/50 whitespace-nowrap">{progress}</span>}
          </div>
        ) : tool.installed ? (
          <button type="button" onClick={() => handleUninstall(tool)}
            className="p-1 rounded text-muted-foreground/30 opacity-0 group-hover:opacity-100 hover:text-destructive transition-all" title="卸载">
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        ) : (
          <button type="button" onClick={() => handleInstall(tool)}
            className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
            title={`通过 ${installMethodLabel(tool)} 安装`}>
            <Download className="w-3 h-3" /> 安装
          </button>
        )}
      </div>
    );
  };

  const TagBadges = ({ tool, compact }: { tool: ToolWithMeta; compact?: boolean }) => (
    <div className="flex items-center gap-1.5 flex-wrap">
      <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", runtimeBadge(tool.runtime))}>
        {tool.runtime}{tool.runtimeVersion ? ` ${tool.runtimeVersion}` : ""}
      </span>
      {tool.install?.method && tool.install.method !== "manual" && (
        <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full", installMethodBadge(tool.install.method))}>
          {installMethodLabel(tool)}
        </span>
      )}
      <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-muted/30 text-muted-foreground/50">
        {tool.categoryName}
      </span>
      {!compact && tool.subcategoryName && (
        <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-muted/30 text-muted-foreground/50">
          {tool.subcategoryName}
        </span>
      )}
    </div>
  );

  const GridCard = ({ tool }: { tool: ToolWithMeta }) => (
    <div
      onClick={() => openEditor(tool)}
      onContextMenu={(e) => handleContextMenu(e, tool)}
      className={cn(
        "group rounded-xl border transition-all cursor-pointer p-4",
        tool.installed
          ? "border-border/15 bg-[var(--bg-hover)]/20 hover:bg-[var(--bg-hover)]/40"
          : "border-border/10 bg-[var(--bg-hover)]/8 hover:bg-[var(--bg-hover)]/20"
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            {tool.icon && <span className="text-[14px] flex-shrink-0">{tool.icon}</span>}
            <span className={cn("text-[13px] font-medium truncate", tool.installed ? "text-foreground" : "text-foreground/60")}>{tool.name}</span>
            {tool.installed && (
              <span className="text-[9px] px-1.5 py-px rounded-full bg-green-500/15 text-green-400 flex-shrink-0 flex items-center gap-0.5">
                <Check className="w-2.5 h-2.5" /> 已安装
              </span>
            )}
          </div>
          <p className={cn("text-[11px] mt-1 line-clamp-2",
            tool.installed ? "text-muted-foreground/50" : "text-muted-foreground/35"
          )}>{tool.description}</p>
        </div>
        <ActionButton tool={tool} />
      </div>
      <div className="mt-3"><TagBadges tool={tool} /></div>
    </div>
  );

  const ListRow = ({ tool }: { tool: ToolWithMeta }) => (
    <div
      onClick={() => openEditor(tool)}
      onContextMenu={(e) => handleContextMenu(e, tool)}
      className={cn(
        "group rounded-xl border transition-all cursor-pointer px-4 py-2.5 flex items-center",
        tool.installed
          ? "border-border/15 bg-[var(--bg-hover)]/20 hover:bg-[var(--bg-hover)]/40"
          : "border-border/10 bg-[var(--bg-hover)]/8 hover:bg-[var(--bg-hover)]/20"
      )}
    >
      <div className="flex items-center gap-2 w-[200px] flex-shrink-0">
        {tool.icon && <span className="text-[14px] flex-shrink-0">{tool.icon}</span>}
        <span className={cn("text-[13px] font-medium truncate", tool.installed ? "text-foreground" : "text-foreground/60")}>{tool.name}</span>
        {tool.installed && (
          <span className="text-[9px] px-1.5 py-px rounded-full bg-green-500/15 text-green-400 flex-shrink-0 flex items-center gap-0.5">
            <Check className="w-2.5 h-2.5" /> 已安装
          </span>
        )}
      </div>
      <p className="flex-1 min-w-0 text-[11px] text-muted-foreground/40 truncate px-4">{tool.description}</p>
      <div className="flex-shrink-0 mr-3">
        <TagBadges tool={tool} compact />
      </div>
      <div className="w-[70px] flex justify-end flex-shrink-0">
        <ActionButton tool={tool} />
      </div>
    </div>
  );

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-6 py-4 border-b border-border/15 flex-shrink-0">
        {editingTool ? (
          <>
            <div className={cn("flex items-center gap-3 transition-all duration-[180ms] ease-out",
              editorVisible ? "opacity-100 translate-x-0" : "opacity-0 translate-x-2")}>
              <button type="button" onClick={closeEditor}
                className="p-1.5 rounded-lg text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">
                <ArrowLeft className="w-4 h-4" />
              </button>
              <div>
                <div className="flex items-center gap-2">
                  {editingTool.icon && <span className="text-[14px]">{editingTool.icon}</span>}
                  <h1 className="text-[16px] font-semibold text-foreground">{editingTool.name}</h1>
                  {editorDirty && <span className="w-2 h-2 rounded-full bg-accent/60 flex-shrink-0" title="有未保存的修改" />}
                </div>
                <p className="text-[11px] text-muted-foreground/50 mt-0.5">编辑工具配置</p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <div className="flex items-center rounded-lg border border-border/15 overflow-hidden">
                <button type="button" onClick={() => handleSwitchMode("form")}
                  className={cn("flex items-center gap-1.5 px-3 py-1.5 text-[11px] transition-colors",
                    editorMode === "form" ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                  <FileText className="w-3 h-3" /> 界面
                </button>
                <button type="button" onClick={() => handleSwitchMode("raw")}
                  className={cn("flex items-center gap-1.5 px-3 py-1.5 text-[11px] transition-colors",
                    editorMode === "raw" ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                  <Code2 className="w-3 h-3" /> JSON
                </button>
              </div>
              <button type="button" onClick={handleSave} disabled={saving || !editorDirty}
                className={cn("flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium transition-colors",
                  editorDirty ? "bg-accent text-accent-foreground hover:bg-accent/90" : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed")}>
                {saving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />} 保存
              </button>
            </div>
          </>
        ) : (
          <>
            <div>
              <h1 className="text-[16px] font-semibold text-foreground">工具管理</h1>
              <p className="text-[11px] text-muted-foreground/50 mt-0.5">
                {tools.length} 个工具 · {installedCount} 已安装
              </p>
            </div>
            <div className="flex items-center gap-1.5">
              <button type="button" onClick={handleAddTool} title="添加工具"
                className="p-2 rounded-lg text-muted-foreground/50 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors">
                <Plus className="w-4 h-4" />
              </button>
              <button type="button" onClick={loadData} disabled={loading} title="刷新"
                className="p-2 rounded-lg text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-30">
                <RefreshCw className={cn("w-4 h-4", loading && "animate-spin")} />
              </button>
            </div>
          </>
        )}
      </div>

      {error && (
        <div className="mx-6 mt-3 text-[11px] text-destructive/80 bg-destructive/5 rounded-md px-3 py-2 flex items-center justify-between">
          <span>{error}</span>
          <button type="button" onClick={() => setError(null)} className="ml-2 text-destructive/50 hover:text-destructive"><X className="w-3 h-3" /></button>
        </div>
      )}

      {/* Editor view */}
      {editingTool ? (
        <div className={cn("flex-1 overflow-y-auto px-6 py-4 transition-all duration-[180ms] ease-out",
          editorVisible ? "opacity-100 translate-x-0" : "opacity-0 translate-x-3")}>
          {editorLoading ? (
            <div className="flex items-center justify-center h-32">
              <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
            </div>
          ) : editorMode === "raw" ? (
            <textarea ref={textareaRef} value={rawJson} onChange={(e) => handleRawChange(e.target.value)} spellCheck={false}
              className="w-full h-full min-h-[400px] px-4 py-3 text-[11px] font-mono leading-[1.6] rounded-lg border border-border/10 bg-[var(--bg-hover)]/20 text-foreground outline-none focus:border-accent/30 transition-colors resize-none"
              style={{ tabSize: 2 }} />
          ) : (
            <div className="flex gap-4 h-full">
              <div className="flex-1 min-w-0 space-y-4 overflow-y-auto">
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">基本信息</span></div>
                  <FieldRow label="名称" field="name" placeholder="dirsearch" />
                  <FieldRow label="图标" field="icon" placeholder="📂" />
                  <div className="flex items-start gap-3 py-2 px-3 rounded-lg hover:bg-[var(--bg-hover)]/30 transition-colors">
                    <span className="text-[12px] text-muted-foreground/60 w-24 flex-shrink-0 mt-1.5">描述</span>
                    <textarea value={(formData.description as string) ?? ""} onChange={(e) => handleFormChange("description", e.target.value)}
                      placeholder="工具描述..." rows={2}
                      className="flex-1 px-2 py-1.5 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors resize-y" />
                  </div>
                  <FieldRow label="ID" field="id" mono placeholder="hash" />
                  <FieldRow label="可执行文件" field="executable" mono placeholder="tool/main.py" />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">运行环境</span></div>
                  <FieldRow label="运行时" field="runtime" type="select" options={[
                    { value: "python", label: "Python" }, { value: "java", label: "Java" },
                    { value: "node", label: "Node.js" }, { value: "native", label: "Native" },
                  ]} />
                  <FieldRow label="版本" field="runtimeVersion" placeholder="3.11" />
                  <FieldRow label="界面" field="ui" type="select" options={[
                    { value: "cli", label: "CLI" }, { value: "gui", label: "GUI" }, { value: "web", label: "Web" },
                  ]} />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">安装方式</span></div>
                  <InstallFieldRow label="安装方法" subField="method" type="select" options={[
                    { value: "", label: "无" }, { value: "github", label: "GitHub" },
                    { value: "homebrew", label: "Homebrew" }, { value: "manual", label: "手动" },
                  ]} />
                  <InstallFieldRow label="来源" subField="source" placeholder="owner/repo" mono />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">参数配置</span></div>
                  <div className="py-2"><ParamsEditor /></div>
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">分类</span></div>
                  <FieldRow label="分类" field="category" placeholder="web" />
                  <FieldRow label="子分类" field="subcategory" placeholder="fuzzing" />
                </div>
              </div>
              <div className="w-[380px] flex-shrink-0 rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden flex flex-col">
                <div className="px-3 py-2 border-b border-border/8 flex items-center gap-2">
                  <Code2 className="w-3 h-3 text-muted-foreground/30" />
                  <span className="text-[11px] font-medium text-muted-foreground/40">JSON 预览</span>
                </div>
                <pre className="flex-1 overflow-auto px-4 py-3 text-[10px] font-mono leading-[1.6] text-muted-foreground/60 select-all whitespace-pre">
                  {JSON.stringify({ tool: formData }, null, 2)}
                </pre>
              </div>
            </div>
          )}
        </div>
      ) : (
        <>
          {/* Search + filters toolbar */}
          <div className="px-6 py-3 flex items-center gap-3 border-b border-border/10 flex-shrink-0">
            <div className="relative flex-1 max-w-sm">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
              <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="搜索工具名称、描述..."
                className="w-full h-8 pl-8 pr-3 text-[12px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors" />
            </div>

            <div className="flex items-center gap-1">
              <button type="button" onClick={() => setSelectedCategory(null)}
                className={cn("text-[11px] px-2.5 py-1 rounded-md transition-colors",
                  !selectedCategory ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                全部
              </button>
              {allCategories.map((catId) => (
                <button key={catId} type="button" onClick={() => setSelectedCategory(selectedCategory === catId ? null : catId)}
                  className={cn("text-[11px] px-2.5 py-1 rounded-md transition-colors",
                    selectedCategory === catId ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                  {categoryDisplayName(catId)}
                </button>
              ))}
            </div>

            <div className="ml-auto flex items-center gap-1">
              {/* Sort */}
              <Select value={sortKey} onValueChange={(v) => setSortKey(v as SortKey)}>
                <SelectTrigger size="sm" className="h-7 w-auto border-transparent bg-transparent hover:bg-[var(--bg-hover)] text-[11px] shadow-none px-2 gap-1 text-muted-foreground/50">
                  <ArrowUpDown className="w-3 h-3" />
                  <SelectValue />
                </SelectTrigger>
                <SelectContent position="popper" className="min-w-[100px]">
                  <SelectItem value="name" className="text-[12px]">名称</SelectItem>
                  <SelectItem value="status" className="text-[12px]">安装状态</SelectItem>
                  <SelectItem value="category" className="text-[12px]">分类</SelectItem>
                  <SelectItem value="runtime" className="text-[12px]">运行时</SelectItem>
                </SelectContent>
              </Select>

              {/* View toggle */}
              <div className="flex items-center rounded-md border border-border/10 overflow-hidden">
                <button type="button" onClick={() => setViewMode("grid")} title="网格视图"
                  className={cn("p-1.5 transition-colors", viewMode === "grid" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground")}>
                  <Grid3X3 className="w-3.5 h-3.5" />
                </button>
                <button type="button" onClick={() => setViewMode("list")} title="列表视图"
                  className={cn("p-1.5 transition-colors", viewMode === "list" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground")}>
                  <List className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>
          </div>

          {/* Tool grid/list */}
          <div className="flex-1 overflow-y-auto px-6 py-4">
            {loading ? (
              <div className="flex items-center justify-center h-32">
                <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
              </div>
            ) : filteredTools.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-32 gap-2">
                <span className="text-[12px] text-muted-foreground/40">
                  {search.trim() ? "没有找到匹配的工具" : "暂无工具"}
                </span>
                {!search.trim() && (
                  <button type="button" onClick={handleAddTool}
                    className="text-[11px] text-accent/60 hover:text-accent transition-colors flex items-center gap-1">
                    <Plus className="w-3 h-3" /> 添加第一个工具
                  </button>
                )}
              </div>
            ) : viewMode === "grid" ? (
              <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
                {filteredTools.map((tool) => <GridCard key={tool.id} tool={tool} />)}
              </div>
            ) : (
              <div className="space-y-1">
                {filteredTools.map((tool) => <ListRow key={tool.id} tool={tool} />)}
              </div>
            )}
          </div>
        </>
      )}

      {/* Context menu */}
      {ctxMenu && (
        <div className="fixed z-50 rounded-lg border border-border/20 bg-popover shadow-xl py-1 min-w-[140px]"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          onClick={(e) => e.stopPropagation()}>
          <button type="button" onClick={() => ctxAction("edit")}
            className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
            <FileText className="w-3 h-3 text-muted-foreground/50" /> 编辑配置
          </button>
          {ctxMenu.tool.installed ? (
            <button type="button" onClick={() => ctxAction("uninstall")}
              className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
              <Trash2 className="w-3 h-3 text-muted-foreground/50" /> 卸载
            </button>
          ) : (
            <button type="button" onClick={() => ctxAction("install")}
              className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
              <Download className="w-3 h-3 text-muted-foreground/50" /> 安装
            </button>
          )}
          <div className="my-1 border-t border-border/10" />
          <button type="button" onClick={() => ctxAction("copy-id")}
            className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
            <Copy className="w-3 h-3 text-muted-foreground/50" /> 复制 ID
          </button>
          {ctxMenu.tool.installed && (
            <button type="button" onClick={() => ctxAction("open-dir")}
              className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
              <FolderOpen className="w-3 h-3 text-muted-foreground/50" /> 打开目录
            </button>
          )}
        </div>
      )}

      {/* Uninstall confirm */}
      {uninstallTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setUninstallTarget(null)}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-xs w-full" onClick={(e) => e.stopPropagation()}>
            <p className="text-[13px] text-foreground mb-1">卸载 {uninstallTarget.name}</p>
            <p className="text-[11px] text-muted-foreground/50 mb-4">配置保留，可重新安装</p>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setUninstallTarget(null)}
                className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">取消</button>
              <button type="button" onClick={confirmUninstall}
                className="text-[12px] px-3 py-1.5 rounded-lg bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">确认卸载</button>
            </div>
          </div>
        </div>
      )}

      {/* Close confirm */}
      {showCloseConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setShowCloseConfirm(false)}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-xs w-full" onClick={(e) => e.stopPropagation()}>
            <p className="text-[13px] text-foreground mb-1">有未保存的修改</p>
            <p className="text-[11px] text-muted-foreground/50 mb-4">关闭后修改将丢失</p>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setShowCloseConfirm(false)}
                className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">继续编辑</button>
              <button type="button" onClick={forceCloseEditor}
                className="text-[12px] px-3 py-1.5 rounded-lg bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">放弃修改</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
