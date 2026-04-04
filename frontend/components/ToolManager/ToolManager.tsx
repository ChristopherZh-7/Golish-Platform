import { useCallback, useEffect, useState, useRef } from "react";
import { createPortal } from "react-dom";
import {
  ArrowLeft, ArrowUpDown, Check, Code2, Copy, Download, FileText,
  FolderOpen, Grid3X3, Loader2, List, Plus, RefreshCw, Save, Search, Trash2, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { scanTools, deleteTool, getCategories, fetchGitHubRelease, downloadAndExtract, installRuntime, createPythonEnv, listPythonEnvs, listInstalledJava, listAvailableJava, installJavaVersion } from "@/lib/pentest/api";
import type { ToolConfig, ToolCategory } from "@/lib/pentest/types";
import { getSettings } from "@/lib/settings";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useTranslation } from "react-i18next";

type ToolWithMeta = ToolConfig & { categoryName?: string; subcategoryName?: string };
type ViewMode = "grid" | "list";
type SortKey = "name" | "status" | "category" | "runtime";

export function ToolManager() {
  const { t } = useTranslation();
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
  const [deleteTarget, setDeleteTarget] = useState<ToolWithMeta | null>(null);

  // Install progress
  const [installProgress, setInstallProgress] = useState<Record<string, string>>({});
  const [dlProgress, setDlProgress] = useState<{ downloaded: number; total: number } | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<{ downloaded: number; total: number }>("download-progress", (e) => {
      setDlProgress(e.payload);
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

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
      setError(t("toolManager.loadFailed", { error: e }));
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
    if (!method) { setError(t("toolManager.noInstallMethod", { name: tool.name })); return; }

    const proxyUrl = await getProxy();

    if (tool.runtime === "python" && tool.runtimeVersion) {
      const ver = tool.runtimeVersion;
      const envName = `python${ver}_env`;
      console.log(`[Install] ${tool.name}: 检查 Python 环境 ${envName}...`);

      let envExists = false;
      try {
        const envsResult = await listPythonEnvs();
        if (envsResult.success) {
          envExists = envsResult.versions.some((v) => v.vendor === envName);
          console.log(`[Install] 已有环境:`, envsResult.versions.map((v) => v.vendor));
        }
      } catch { /* assume not exists */ }

      console.log(`[Install] ${envName} 存在: ${envExists}`);
      if (!envExists) {
        setError(null);
        setBusy(tool.id);
        setInstallProgress((p) => ({ ...p, [tool.id]: t("install.missingPythonEnv", { ver }) }));
        try {
          const envResult = await createPythonEnv(envName, ver, proxyUrl);
          if (!envResult.success) {
            setError(t("install.pythonEnvFailed", { ver, error: envResult.message }));
            setBusy(null);
            setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
            return;
          }
        } catch (e) {
          setError(t("install.pythonEnvFailed", { ver, error: e }));
          setBusy(null);
          setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
          return;
        }
      }
    }

    if (tool.runtime === "java") {
      const requiredMajor = tool.runtimeVersion || "17";
      console.log(`[Install] ${tool.name}: 检查 Java ${requiredMajor} 环境...`);
      let javaReady = false;
      try {
        const javaResult = await listInstalledJava();
        if (javaResult.success && javaResult.versions.length > 0) {
          console.log(`[Install] 已安装 Java 版本:`, javaResult.versions.map((v) => v.version));
          javaReady = javaResult.versions.some((v) => v.version.startsWith(`${requiredMajor}.`) || v.version === requiredMajor);
        }
      } catch { /* assume not installed */ }

      console.log(`[Install] Java ${requiredMajor} 已安装: ${javaReady}`);
      if (!javaReady) {
        setError(null);
        setBusy(tool.id);
        setInstallProgress((p) => ({ ...p, [tool.id]: t("install.missingJava", { ver: requiredMajor }) }));
        try {
          let identifier = "";
          const available = await listAvailableJava();
          if (available.success) {
            const majorMatches = available.versions.filter((v) =>
              v.version.startsWith(`${requiredMajor}.`) || v.version === requiredMajor
            );
            const fxMatch = majorMatches.find((v) => v.version.includes("-fx"));
            const temMatch = majorMatches.find((v) => v.version.endsWith("-tem"));
            const match = fxMatch || temMatch || majorMatches[0];
            if (match) identifier = match.version;
            console.log(`[Install] Java ${requiredMajor} 可选版本: ${majorMatches.map(v => v.version).join(", ")}, 选择: ${identifier}`);
          }
          if (!identifier) {
            setError(t("install.javaNotFound", { ver: requiredMajor }));
            setBusy(null);
            setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
            return;
          }
          console.log(`[Install] 自动安装 Java: ${identifier}`);
          setInstallProgress((p) => ({ ...p, [tool.id]: t("install.installingJava", { id: identifier }) }));
          const javaInstall = await installJavaVersion(identifier, proxyUrl);
          if (!javaInstall.success) {
            setError(t("install.javaFailed", { ver: requiredMajor, error: javaInstall.message }));
            setBusy(null);
            setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
            return;
          }
        } catch (e) {
          setError(t("install.javaFailed", { ver: requiredMajor, error: e }));
          setBusy(null);
          setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
          return;
        }
      }
    }

    setBusy(tool.id);
    setInstallProgress((p) => ({ ...p, [tool.id]: t("common.preparing") }));
    setDlProgress(null);
    setError(null);
    try {
      if (method === "github") {
        const source = tool.install?.source;
        if (!source) throw new Error(t("install.missingGithubSource"));
        const [owner, repo] = source.split("/");
        if (!owner || !repo) throw new Error(t("install.githubSourceFormat"));
        setInstallProgress((p) => ({ ...p, [tool.id]: t("install.detectMethod") }));

        let binaryAsset: { browser_download_url: string; name: string } | null = null;
        let releaseVersion: string | null = null;
        try {
          console.log(`[Install] ${tool.name}: 获取 GitHub release from ${owner}/${repo}`);
          const release = await fetchGitHubRelease(owner, repo);
          releaseVersion = release.tag_name;
          console.log(`[Install] ${tool.name}: Release tag=${release.tag_name}, assets=[${release.assets.map((a) => a.name).join(", ")}]`);
          const platform = navigator.platform.toLowerCase();
          const isMac = platform.includes("mac") || platform.includes("darwin");
          binaryAsset = release.assets.find((a) => {
            const n = a.name.toLowerCase();
            if (isMac) return n.includes("darwin") || n.includes("macos") || n.includes("mac") || n.includes("osx");
            return n.includes("linux");
          }) || release.assets.find((a) => {
            const n = a.name.toLowerCase();
            return n.endsWith(".zip") || n.endsWith(".tar.gz") || n.endsWith(".tgz") || n.endsWith(".jar");
          }) || null;
          console.log(`[Install] ${tool.name}: 选中的 asset = ${binaryAsset?.name || "null"}`);
        } catch (releaseErr) {
          const errStr = String(releaseErr);
          console.error(`[Install] ${tool.name}: Release 获取失败:`, errStr);
          if (errStr.includes("403")) {
            throw new Error(t("install.githubRateLimit"));
          }
          console.warn(`[Install] ${tool.name}: 非限流错误, 将尝试 git clone:`, errStr);
        }

        if (binaryAsset) {
          console.log(`[Install] ${tool.name}: 开始下载 ${binaryAsset.name} from ${binaryAsset.browser_download_url}`);
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.downloadRelease") }));
          const result = await downloadAndExtract({ url: binaryAsset.browser_download_url, fileName: binaryAsset.name, useProxy: !!proxyUrl });
          console.log(`[Install] ${tool.name}: 下载结果:`, JSON.stringify(result));
          if (!result.success) throw new Error(result.error || t("install.downloadFailed"));
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.installing") }));

          const stableDirName = tool.name;
          console.log(`[Install] ${tool.name}: 稳定目录名 = ${stableDirName}, extract_path = ${result.extract_path}`);
          if (result.extract_path) {
            const actualDir = result.extract_path.split("/").pop() || "";
            console.log(`[Install] ${tool.name}: 实际目录 = ${actualDir}, 目标 = ${stableDirName}`);
            if (actualDir && actualDir !== stableDirName) {
              try {
                await invoke("pentest_rename_tool_dir", { fromPath: result.extract_path, toName: stableDirName });
                console.log(`[Install] ${tool.name}: 重命名成功 ${actualDir} → ${stableDirName}`);
              } catch (renameErr) {
                console.error(`[Install] ${tool.name}: 重命名失败:`, renameErr);
              }
            }
          }

          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.detectExecutable") }));
          try {
            const execs: string[] = await invoke("pentest_find_tool_executables", {
              toolDir: stableDirName,
              runtime: tool.runtime || null,
            });
            console.log(`[Install] ${tool.name}: 扫描到可执行文件:`, execs);

            let selectedExec: string | null = null;
            if (execs.length === 1) {
              selectedExec = execs[0];
              console.log(`[Install] ${tool.name}: 自动选择: ${selectedExec}`);
            } else if (execs.length > 1) {
              console.log(`[Install] ${tool.name}: 多个可执行文件，弹出选择器`);
              selectedExec = await new Promise<string | null>((resolve) => {
                setExecPicker({ tool, dirName: stableDirName, candidates: execs, resolve });
              });
              console.log(`[Install] ${tool.name}: 用户选择: ${selectedExec}`);
            } else {
              console.warn(`[Install] ${tool.name}: 未找到可执行文件！`);
            }

            if (selectedExec) {
              const newExecutable = `${stableDirName}/${selectedExec}`;
              console.log(`[Install] ${tool.name}: 当前 executable = ${tool.executable}, 新 = ${newExecutable}`);
              await invoke("pentest_update_tool_executable", {
                toolId: tool.id,
                category: tool.category,
                subcategory: tool.subcategory,
                newExecutable,
                version: releaseVersion || undefined,
                lastUpdated: new Date().toISOString().slice(0, 10),
              });
              console.log(`[Install] ${tool.name}: executable 已更新, version=${releaseVersion}`);
            } else if (releaseVersion) {
              await invoke("pentest_update_tool_executable", {
                toolId: tool.id,
                category: tool.category,
                subcategory: tool.subcategory,
                newExecutable: tool.executable,
                version: releaseVersion,
                lastUpdated: new Date().toISOString().slice(0, 10),
              });
            }
          } catch (scanErr) {
            console.error(`[Install] ${tool.name}: 扫描/更新失败:`, scanErr);
          }
        } else {
          console.log(`[Install] ${tool.name}: 无 binary asset，使用 git clone`);
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.gitCloning") }));
          const toolDir = tool.executable?.split("/")[0] || tool.name;
          const cloneUrl = `https://github.com/${source}.git`;
          console.log(`[Install] ${tool.name}: git clone ${cloneUrl} → ${toolDir}`);
          await invoke("pentest_git_clone_tool", { source: cloneUrl, toolDir, proxyUrl: proxyUrl || null });
          console.log(`[Install] ${tool.name}: git clone 完成`);
        }
      } else if (method === "homebrew") {
        setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.brewInstalling") }));
        const pkg = tool.install?.source || tool.name;
        const brewResult = await installRuntime(`brew:${pkg}`, proxyUrl);
        const brewVerMatch = brewResult.message?.match(/BREW_VERSION=(.+)/);
        if (brewVerMatch) {
          await invoke("pentest_update_tool_executable", {
            toolId: tool.id,
            category: tool.category,
            subcategory: tool.subcategory,
            newExecutable: tool.executable,
            version: brewVerMatch[1],
            lastUpdated: new Date().toISOString().slice(0, 10),
          });
        }
      }

      if (tool.runtime === "python" && tool.runtimeVersion) {
        const toolDir = tool.executable?.split("/")[0] || tool.name;
        try {
          const hasReqs = await invoke<boolean>("pentest_check_requirements", { toolDir });
          console.log(`[Install] ${tool.name}: requirements.txt exists = ${hasReqs}`);
          if (hasReqs) {
            setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.installingPythonDeps") }));
            await invoke("pentest_install_requirements", {
              toolDir,
              pythonVersion: tool.runtimeVersion,
              proxyUrl: proxyUrl || null,
            });
            console.log(`[Install] ${tool.name}: requirements.txt 依赖安装完成`);
          }
        } catch (e) {
          console.warn(`[Install] ${tool.name}: requirements.txt 安装失败:`, e);
        }
      }

      console.log(`[Install] ${tool.name}: 安装流程完成`);
      await loadData();
    } catch (e) {
      setError(t("toolManager.installFailed", { error: e }));
    } finally {
      setBusy(null);
      setDlProgress(null);
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
      setError(t("toolManager.uninstallFailed", { error: e }));
    } finally {
      setBusy(null);
    }
  }, [loadData]);

  const [depPicker, setDepPicker] = useState<{ tool: ToolWithMeta; files: string[] } | null>(null);
  const [execPicker, setExecPicker] = useState<{
    tool: ToolWithMeta;
    dirName: string;
    candidates: string[];
    resolve: (v: string | null) => void;
  } | null>(null);

  const handleInstallDeps = useCallback(async (tool: ToolWithMeta) => {
    if (tool.runtime !== "python" || !tool.runtimeVersion) return;
    const toolDir = tool.executable?.split("/")[0] || tool.name;
    try {
      const files = await invoke<string[]>("pentest_list_dep_files", { toolDir });
      if (files.length === 0) {
        setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.noDepFiles") }));
        setTimeout(() => setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; }), 2000);
        return;
      }
      if (files.length === 1 && files[0].toLowerCase() === "requirements.txt") {
        await doInstallDepFile(tool, files[0]);
      } else {
        setDepPicker({ tool, files });
      }
    } catch (e) {
      setError(t("toolManager.scanFailed", { error: e }));
    }
  }, []);

  const doInstallDepFile = useCallback(async (tool: ToolWithMeta, fileName: string) => {
    setDepPicker(null);
    const toolDir = tool.executable?.split("/")[0] || tool.name;
    setBusy(tool.id);
    setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.installingDeps", { file: fileName }) }));
    setError(null);
    try {
      const proxyUrl = await getProxy();
      await invoke("pentest_install_dep_file", {
        toolDir,
        fileName,
        pythonVersion: tool.runtimeVersion || "",
        proxyUrl: proxyUrl || null,
      });
      setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.depInstallDone") }));
      await new Promise((r) => setTimeout(r, 1500));
    } catch (e) {
      setError(t("toolManager.depInstallFailed", { error: e }));
    } finally {
      setBusy(null);
      setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
    }
  }, [getProxy]);

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

      // Use category/subcategory from form data (user may have changed them)
      const category = (formData.category as string) || editingTool.category || "misc";
      const subcategory = (formData.subcategory as string) || editingTool.subcategory || "other";
      const toolId = (formData.id as string) || editingTool.id;

      // If category changed, delete old config first
      if (editingTool.category && editingTool.subcategory &&
          (editingTool.category !== category || editingTool.subcategory !== subcategory)) {
        await invoke("pentest_delete_tool", {
          toolId: editingTool.id, category: editingTool.category,
          subcategory: editingTool.subcategory, toolFolder: null,
        }).catch(() => {});
      }

      await invoke("pentest_save_tool_config", { category, subcategory, toolId, content });
      setEditorDirty(false);
      await loadData();
    } catch (e) {
      setError(t("toolManager.saveFailed", { error: e }));
    } finally {
      setSaving(false);
    }
  }, [editingTool, editorMode, rawJson, formData, loadData]);

  const handleSwitchMode = useCallback((mode: "form" | "raw") => {
    if (mode === "raw") syncFormToRaw(formData);
    else syncRawToForm(rawJson);
    setEditorMode(mode);
  }, [editorMode, formData, rawJson, syncFormToRaw, syncRawToForm]);

  // Add new tool — open editor directly without creating a file
  const handleAddTool = useCallback(() => {
    const id = Math.random().toString(36).substring(2, 10);
    const defaults: Record<string, unknown> = {
      id,
      name: "",
      description: "",
      icon: "🔧",
      executable: "",
      runtime: "native",
      runtimeVersion: "",
      ui: "cli",
      params: [],
      install: { method: "", source: "" },
    };
    const json = JSON.stringify({ tool: defaults }, null, 2);
    const placeholder: ToolWithMeta = {
      ...defaults,
      name: t("toolManager.newTool"),
      category: "misc",
      subcategory: "other",
      installed: false,
      categoryName: "misc",
      subcategoryName: "other",
    } as unknown as ToolWithMeta;

    setEditingTool(placeholder);
    setEditorMode("form");
    setEditorDirty(true);
    setEditorLoading(false);
    originalJsonRef.current = json;
    setRawJson(json);
    setFormData(defaults);
    requestAnimationFrame(() => setEditorVisible(true));
  }, []);

  // Context menu
  const handleContextMenu = useCallback((e: React.MouseEvent, tool: ToolWithMeta) => {
    e.preventDefault();
    e.stopPropagation();
    setCtxMenu({ tool, x: e.clientX, y: e.clientY });
  }, []);

  const ctxAction = useCallback(async (action: string) => {
    if (!ctxMenu) return;
    const tool = ctxMenu.tool;
    setCtxMenu(null);
    switch (action) {
      case "edit": openEditor(tool); break;
      case "uninstall": handleUninstall(tool); break;
      case "install": handleInstall(tool); break;
      case "install-deps": handleInstallDeps(tool); break;
      case "copy-id": navigator.clipboard.writeText(tool.id); break;
      case "open-dir":
        invoke("pentest_open_directory", { executable: tool.executable || tool.name }).catch(() => {});
        break;
      case "delete":
        setDeleteTarget(tool);
        break;
    }
  }, [ctxMenu, openEditor, handleUninstall, handleInstall, handleInstallDeps, loadData]);

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
    if (!method || method === "manual") return t("toolManager.manual");
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
        <SelectValue placeholder={t("common.select")} />
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
          <p className="text-[12px] text-muted-foreground/25 py-3 text-center">{t("toolManager.noParams")}</p>
        ) : (
          <div className="space-y-1">
            {params.map((p, i) => (
              <div key={i} className="flex items-center gap-2 py-1.5 group/param rounded-lg hover:bg-[var(--bg-hover)]/30 px-1">
                <input value={(p.label as string) || ""} onChange={(e) => updateParam(i, "label", e.target.value)}
                  placeholder={t("toolManager.label")} className="flex-[3] h-7 px-2 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors" />
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
          <Plus className="w-3 h-3" /> {t("toolManager.addParam")}
        </button>
      </div>
    );
  };

  const CircleProgress = ({ size = 28, pct }: { size?: number; pct: number }) => {
    const r = (size - 4) / 2;
    const circ = 2 * Math.PI * r;
    const offset = circ * (1 - Math.min(Math.max(pct, 0), 1));
    return (
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="flex-shrink-0">
        <circle cx={size / 2} cy={size / 2} r={r} fill="none"
          stroke="currentColor" strokeWidth="2.5" className="text-muted-foreground/10" />
        <circle cx={size / 2} cy={size / 2} r={r} fill="none"
          stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"
          strokeDasharray={`${circ}`} strokeDashoffset={`${offset}`}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
          className="text-accent transition-[stroke-dashoffset] duration-200" />
      </svg>
    );
  };

  // Tool card (used in both grid and list views)
  const ActionButton = ({ tool }: { tool: ToolWithMeta }) => {
    const isBusy = busy === tool.id;
    const progress = installProgress[tool.id];
    const hasDlProgress = isBusy && dlProgress && dlProgress.total > 0;
    const dlPct = hasDlProgress ? dlProgress!.downloaded / dlProgress!.total : 0;
    return (
      <div className="flex items-center gap-1 flex-shrink-0" onClick={(e) => e.stopPropagation()}>
        {isBusy ? (
          <div className="flex items-center gap-1.5">
            {hasDlProgress ? (
              <CircleProgress pct={dlPct} size={22} />
            ) : (
              <Loader2 className="w-3.5 h-3.5 animate-spin text-accent/60" />
            )}
            {progress && <span className="text-[10px] text-accent/50 whitespace-nowrap max-w-[80px] truncate">{progress}</span>}
          </div>
        ) : tool.installed ? (
          <button type="button" onClick={() => handleUninstall(tool)}
            className="p-1 rounded text-muted-foreground/30 opacity-0 group-hover:opacity-100 hover:text-destructive transition-all" title={t("common.uninstall")}>
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        ) : (
          <button type="button" onClick={() => handleInstall(tool)}
            className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
            title={t("toolManager.installVia", { method: installMethodLabel(tool) })}>
            <Download className="w-3 h-3" /> {t("common.install")}
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
        "group rounded-xl border transition-colors cursor-pointer p-4",
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
                <Check className="w-2.5 h-2.5" /> {t("common.installed")}
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
        "group rounded-xl border transition-colors cursor-pointer px-4 py-2.5 flex items-center",
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
            <Check className="w-2.5 h-2.5" /> {t("common.installed")}
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
                  {editorDirty && <span className="w-2 h-2 rounded-full bg-accent/60 flex-shrink-0" title={t("toolManager.unsavedChanges")} />}
                </div>
                <p className="text-[11px] text-muted-foreground/50 mt-0.5">{t("toolManager.editToolConfig")}</p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <div className="flex items-center rounded-lg border border-border/15 overflow-hidden">
                <button type="button" onClick={() => handleSwitchMode("form")}
                  className={cn("flex items-center gap-1.5 px-3 py-1.5 text-[11px] transition-colors",
                    editorMode === "form" ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                  <FileText className="w-3 h-3" /> {t("toolManager.form")}
                </button>
                <button type="button" onClick={() => handleSwitchMode("raw")}
                  className={cn("flex items-center gap-1.5 px-3 py-1.5 text-[11px] transition-colors",
                    editorMode === "raw" ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                  <Code2 className="w-3 h-3" /> {t("toolManager.json")}
                </button>
              </div>
              <button type="button" onClick={handleSave} disabled={saving || !editorDirty}
                className={cn("flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium transition-colors",
                  editorDirty ? "bg-accent text-accent-foreground hover:bg-accent/90" : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed")}>
                {saving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />} {t("common.save")}
              </button>
            </div>
          </>
        ) : (
          <>
            <div>
              <h1 className="text-[16px] font-semibold text-foreground">{t("toolManager.title")}</h1>
              <p className="text-[11px] text-muted-foreground/50 mt-0.5">
                {t("toolManager.toolCount", { count: tools.length, installed: installedCount })}
              </p>
            </div>
            <div className="flex items-center gap-1.5">
              <button type="button" onClick={handleAddTool} title={t("toolManager.addTool")}
                className="p-2 rounded-lg text-muted-foreground/50 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors">
                <Plus className="w-4 h-4" />
              </button>
              <button type="button" onClick={loadData} disabled={loading} title={t("common.refresh")}
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
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">{t("toolManager.basicInfo")}</span></div>
                  <FieldRow label={t("toolManager.name")} field="name" placeholder="dirsearch" />
                  <FieldRow label={t("toolManager.icon")} field="icon" placeholder="📂" />
                  <div className="flex items-start gap-3 py-2 px-3 rounded-lg hover:bg-[var(--bg-hover)]/30 transition-colors">
                    <span className="text-[12px] text-muted-foreground/60 w-24 flex-shrink-0 mt-1.5">{t("toolManager.description")}</span>
                    <textarea value={(formData.description as string) ?? ""} onChange={(e) => handleFormChange("description", e.target.value)}
                      placeholder={t("toolManager.descriptionPlaceholder")} rows={2}
                      className="flex-1 px-2 py-1.5 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors resize-y" />
                  </div>
                  <FieldRow label={t("common.version")} field="version" placeholder="1.0.0" />
                  <FieldRow label="ID" field="id" mono placeholder="hash" />
                  <FieldRow label={t("toolManager.executable")} field="executable" mono placeholder="tool/main.py" />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">{t("toolManager.runtime")}</span></div>
                  <FieldRow label={t("toolManager.runtimeLabel")} field="runtime" type="select" options={[
                    { value: "python", label: "Python" }, { value: "java", label: "Java" },
                    { value: "node", label: "Node.js" }, { value: "native", label: "Native" },
                  ]} />
                  {formData.runtime !== "native" && (
                    <FieldRow label={t("toolManager.runtimeVersion")} field="runtimeVersion" placeholder={
                      formData.runtime === "java" ? "17" : formData.runtime === "node" ? "20" : "3.11"
                    } />
                  )}
                  <FieldRow label={t("toolManager.uiLabel")} field="ui" type="select" options={[
                    { value: "cli", label: "CLI" }, { value: "gui", label: "GUI" }, { value: "web", label: "Web" },
                  ]} />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">{t("toolManager.installMethod")}</span></div>
                  <InstallFieldRow label={t("toolManager.installMethodLabel")} subField="method" type="select" options={[
                    { value: "", label: t("common.none") }, { value: "github", label: "GitHub" },
                    { value: "homebrew", label: "Homebrew" }, { value: "manual", label: t("toolManager.manual") },
                  ]} />
                  <InstallFieldRow label={t("toolManager.source")} subField="source"
                    placeholder={
                      ((formData.install as Record<string, string>)?.method === "github") ? "owner/repo" :
                      ((formData.install as Record<string, string>)?.method === "homebrew") ? "formula-name" :
                      t("toolManager.source")
                    } mono />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">{t("toolManager.paramConfig")}</span></div>
                  <div className="py-2"><ParamsEditor /></div>
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/40">{t("toolManager.category")}</span></div>
                  <FieldRow label={t("toolManager.category")} field="category" type="select" options={
                    categories.length > 0
                      ? categories.map((c) => ({ value: c.id, label: c.name }))
                      : [{ value: "misc", label: "misc" }]
                  } />
                  <FieldRow label={t("toolManager.subcategory")} field="subcategory" type="select" options={(() => {
                    const cat = categories.find((c) => c.id === (formData.category as string));
                    if (cat && cat.items.length > 0) return cat.items.map((s) => ({ value: s.id, label: s.name }));
                    return [{ value: "other", label: "other" }];
                  })()} />
                </div>
              </div>
              <div className="w-[380px] flex-shrink-0 rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden flex flex-col">
                <div className="px-3 py-2 border-b border-border/8 flex items-center gap-2">
                  <Code2 className="w-3 h-3 text-muted-foreground/30" />
                  <span className="text-[11px] font-medium text-muted-foreground/40">{t("toolManager.jsonPreview")}</span>
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
              <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("toolManager.searchPlaceholder")}
                className="w-full h-8 pl-8 pr-3 text-[12px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors" />
            </div>

            <div className="flex items-center gap-1">
              <button type="button" onClick={() => setSelectedCategory(null)}
                className={cn("text-[11px] px-2.5 py-1 rounded-md transition-colors",
                  !selectedCategory ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                {t("common.all")}
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
                  <SelectItem value="name" className="text-[12px]">{t("toolManager.sortByName")}</SelectItem>
                  <SelectItem value="status" className="text-[12px]">{t("toolManager.sortByStatus")}</SelectItem>
                  <SelectItem value="category" className="text-[12px]">{t("toolManager.sortByCategory")}</SelectItem>
                  <SelectItem value="runtime" className="text-[12px]">{t("toolManager.sortByRuntime")}</SelectItem>
                </SelectContent>
              </Select>

              {/* View toggle */}
              <div className="flex items-center rounded-md border border-border/10 overflow-hidden">
                <button type="button" onClick={() => setViewMode("grid")} title={t("toolManager.gridView")}
                  className={cn("p-1.5 transition-colors", viewMode === "grid" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground")}>
                  <Grid3X3 className="w-3.5 h-3.5" />
                </button>
                <button type="button" onClick={() => setViewMode("list")} title={t("toolManager.listView")}
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
                  {search.trim() ? t("toolManager.noMatch") : t("toolManager.noTools")}
                </span>
                {!search.trim() && (
                  <button type="button" onClick={handleAddTool}
                    className="text-[11px] text-accent/60 hover:text-accent transition-colors flex items-center gap-1">
                    <Plus className="w-3 h-3" /> {t("toolManager.addFirstTool")}
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

      {/* Context menu - rendered via portal to avoid transform containing block offset */}
      {ctxMenu && createPortal(
        <div className="fixed z-50 rounded-lg border border-border/20 bg-popover shadow-xl py-1 min-w-[140px]"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          onClick={(e) => e.stopPropagation()}>
          <button type="button" onClick={() => ctxAction("edit")}
            className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
            <FileText className="w-3 h-3 text-muted-foreground/50" /> {t("toolManager.edit")}
          </button>
          {ctxMenu.tool.installed ? (
            <button type="button" onClick={() => ctxAction("uninstall")}
              className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
              <Trash2 className="w-3 h-3 text-muted-foreground/50" /> {t("common.uninstall")}
            </button>
          ) : (
            <button type="button" onClick={() => ctxAction("install")}
              className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
              <Download className="w-3 h-3 text-muted-foreground/50" /> {t("common.install")}
            </button>
          )}
          <div className="my-1 border-t border-border/10" />
          <button type="button" onClick={() => ctxAction("copy-id")}
            className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
            <Copy className="w-3 h-3 text-muted-foreground/50" /> {t("toolManager.copyId")}
          </button>
          {ctxMenu.tool.installed && (
            <button type="button" onClick={() => ctxAction("open-dir")}
              className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
              <FolderOpen className="w-3 h-3 text-muted-foreground/50" /> {t("toolManager.openDir")}
            </button>
          )}
          {ctxMenu.tool.installed && ctxMenu.tool.runtime === "python" && (
            <button type="button" onClick={() => ctxAction("install-deps")}
              className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
              <Download className="w-3 h-3 text-muted-foreground/50" /> {t("toolManager.installDeps")}
            </button>
          )}
          <div className="my-1 border-t border-border/10" />
          <button type="button" onClick={() => ctxAction("delete")}
            className="w-full text-left px-3 py-1.5 text-[12px] text-red-400 hover:bg-red-500/10 transition-colors flex items-center gap-2">
            <Trash2 className="w-3 h-3" /> {t("toolManager.deleteConfig")}
          </button>
        </div>,
        document.body
      )}

      {/* Uninstall confirm */}
      {uninstallTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setUninstallTarget(null)}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-xs w-full" onClick={(e) => e.stopPropagation()}>
            <p className="text-[13px] text-foreground mb-1">{t("toolManager.uninstallConfirm", { name: uninstallTarget.name })}</p>
            <p className="text-[11px] text-muted-foreground/50 mb-4">{t("toolManager.uninstallKeepConfig")}</p>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setUninstallTarget(null)}
                className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("common.cancel")}</button>
              <button type="button" onClick={confirmUninstall}
                className="text-[12px] px-3 py-1.5 rounded-lg bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">{t("toolManager.confirmUninstall")}</button>
            </div>
          </div>
        </div>
      )}

      {/* Dep file picker */}
      {depPicker && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setDepPicker(null)}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-sm w-full" onClick={(e) => e.stopPropagation()}>
            <p className="text-[13px] text-foreground mb-1">{t("toolManager.selectDepFile")}</p>
            <p className="text-[11px] text-muted-foreground/50 mb-3">{t("toolManager.depFileHint", { name: depPicker.tool.name })}</p>
            <div className="space-y-1 max-h-48 overflow-y-auto mb-4">
              {depPicker.files.map((f) => (
                <button key={f} type="button" onClick={() => doInstallDepFile(depPicker.tool, f)}
                  className="w-full text-left px-3 py-2 rounded-lg text-[12px] font-mono text-foreground hover:bg-accent/10 transition-colors">
                  {f}
                </button>
              ))}
            </div>
            <div className="flex justify-end">
              <button type="button" onClick={() => setDepPicker(null)}
                className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("common.cancel")}</button>
            </div>
          </div>
        </div>
      )}

      {/* Executable picker */}
      {execPicker && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
          onClick={() => { execPicker.resolve(null); setExecPicker(null); }}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-sm w-full" onClick={(e) => e.stopPropagation()}>
            <p className="text-[13px] text-foreground mb-1">{t("toolManager.selectExecutable")}</p>
            <p className="text-[11px] text-muted-foreground/50 mb-3">
              {t("toolManager.multipleExecsDetected", { name: execPicker.tool.name })}
            </p>
            <div className="space-y-1 max-h-48 overflow-y-auto mb-4">
              {execPicker.candidates.map((f, i) => (
                <button key={f} type="button"
                  onClick={() => { execPicker.resolve(f); setExecPicker(null); }}
                  className={cn(
                    "w-full text-left px-3 py-2 rounded-lg text-[12px] font-mono transition-colors",
                    i === 0
                      ? "bg-accent/15 text-accent hover:bg-accent/20 font-semibold"
                      : "text-foreground hover:bg-accent/10",
                  )}>
                  {f}{i === 0 && <span className="ml-2 text-[10px] text-accent/70 font-normal">{t("common.recommended")}</span>}
                </button>
              ))}
            </div>
            <div className="flex justify-end">
              <button type="button" onClick={() => { execPicker.resolve(null); setExecPicker(null); }}
                className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("common.skip")}</button>
            </div>
          </div>
        </div>
      )}

      {/* Delete config confirm */}
      {deleteTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setDeleteTarget(null)}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-xs w-full" onClick={(e) => e.stopPropagation()}>
            <p className="text-[13px] text-foreground mb-1">{t("toolManager.deleteConfirmTitle", { name: deleteTarget.name })}</p>
            <p className="text-[11px] text-muted-foreground/50 mb-4">
              {t("toolManager.deleteConfirmMsg")}
              {deleteTarget.installed && t("toolManager.deleteKeepFiles")}
            </p>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setDeleteTarget(null)}
                className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("common.cancel")}</button>
              <button type="button" onClick={async () => {
                const tool = deleteTarget;
                setDeleteTarget(null);
                try {
                  await invoke("pentest_delete_tool", {
                    toolId: tool.id,
                    category: tool.category,
                    subcategory: tool.subcategory,
                    toolFolder: null,
                  });
                  await loadData();
                } catch (e) {
                  setError(t("toolManager.deleteFailed", { error: e }));
                }
              }}
                className="text-[12px] px-3 py-1.5 rounded-lg bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors">{t("toolManager.confirmDelete")}</button>
            </div>
          </div>
        </div>
      )}

      {/* Close confirm */}
      {showCloseConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setShowCloseConfirm(false)}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-xs w-full" onClick={(e) => e.stopPropagation()}>
            <p className="text-[13px] text-foreground mb-1">{t("toolManager.unsavedChanges")}</p>
            <p className="text-[11px] text-muted-foreground/50 mb-4">{t("toolManager.unsavedChangesMsg")}</p>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setShowCloseConfirm(false)}
                className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("toolManager.continueEditing")}</button>
              <button type="button" onClick={forceCloseEditor}
                className="text-[12px] px-3 py-1.5 rounded-lg bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">{t("toolManager.discardChanges")}</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
