import { useCallback, useEffect, useState, useRef } from "react";
import {
  ArrowLeft, ArrowUpCircle, ArrowUpDown, BookOpen, Check, Code2,
  FileText, Github, Grid3X3, Loader2, List, Pencil, Plus, RefreshCw,
  Save, Search, Trash2, X, FolderOpen,
} from "lucide-react";
import { copyToClipboard } from "@/lib/clipboard";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { scanTools, getCategories, fetchGitHubRelease, downloadAndExtract, cancelDownload, installRuntime, createPythonEnv, listPythonEnvs, listInstalledJava, listAvailableJava, installJavaVersion, listSkills, readSkill, writeSkill, deleteSkill, type SkillFileInfo } from "@/lib/pentest/api";
import type { ToolConfig, ToolCategory } from "@/lib/pentest/types";
import { getSettings } from "@/lib/settings";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useTranslation } from "react-i18next";

import { type ToolWithMeta, type ViewMode, type SortKey, OutputParserEditor } from "./OutputParserEditor";
import { GridCard, ListRow, type ActionButtonProps } from "./ToolCards";
import { FieldRow, InstallFieldRow, ParamsEditor, type EditorFieldsContext } from "./EditorFields";
import {
  ContextMenu, UninstallConfirmDialog, DepPickerDialog,
  ExecPickerDialog, DeleteConfirmDialog, CloseConfirmDialog,
  UpdatesDialog, GitHubImportDialog,
  type CtxMenuState, type ExecPickerState, type ToolUpdateInfo,
} from "./Dialogs";

export function ToolManager() {
  const { t } = useTranslation();
  const [tools, setTools] = useState<ToolWithMeta[]>([]);
  const [categories, setCategories] = useState<ToolCategory[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [selectedTier, setSelectedTier] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const cancelRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [sortKey, setSortKey] = useState<SortKey>("name");

  // Editor state
  const [editingTool, setEditingTool] = useState<ToolWithMeta | null>(null);
  const [editorMode, setEditorMode] = useState<"form" | "raw" | "skills" | "output">("form");
  const [rawJson, setRawJson] = useState("");
  const [formData, setFormData] = useState<Record<string, unknown>>({});
  const [editorLoading, setEditorLoading] = useState(false);
  const [editorDirty, setEditorDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const [editorVisible, setEditorVisible] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const originalJsonRef = useRef("");

  // Skills state
  const [skillsList, setSkillsList] = useState<SkillFileInfo[]>([]);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [skillContent, setSkillContent] = useState("");
  const [skillDirty, setSkillDirty] = useState(false);
  const [skillSaving, setSkillSaving] = useState(false);
  const [newSkillName, setNewSkillName] = useState("");
  const [showNewSkill, setShowNewSkill] = useState(false);

  // GitHub import state
  const [showGithubImport, setShowGithubImport] = useState(false);
  const [githubUrl, setGithubUrl] = useState("");
  const [githubAnalyzing, setGithubAnalyzing] = useState(false);

  // Context menu
  const [ctxMenu, setCtxMenu] = useState<CtxMenuState | null>(null);
  const [uninstallTarget, setUninstallTarget] = useState<ToolWithMeta | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ToolWithMeta | null>(null);

  // Install progress
  const [installProgress, setInstallProgress] = useState<Record<string, string>>({});
  const [dlProgress, setDlProgress] = useState<{ downloaded: number; total: number } | null>(null);

  // Tool update check
  const [toolUpdates, setToolUpdates] = useState<ToolUpdateInfo[]>([]);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [showUpdates, setShowUpdates] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let lastUpdate = 0;
    let rafId: number | null = null;
    let pending: { downloaded: number; total: number } | null = null;

    const flush = () => {
      if (pending) {
        setDlProgress(pending);
        pending = null;
      }
      rafId = null;
      lastUpdate = Date.now();
    };

    listen<{ downloaded: number; total: number }>("download-progress", (e) => {
      pending = e.payload;
      const now = Date.now();
      if (now - lastUpdate >= 250) {
        flush();
      } else if (!rafId) {
        rafId = window.setTimeout(flush, 250 - (now - lastUpdate));
      }
    }).then((fn) => { unlisten = fn; });

    return () => {
      unlisten?.();
      if (rafId) clearTimeout(rafId);
    };
  }, []);

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [scanResult, cats] = await Promise.all([
        scanTools().catch(() => ({ success: false, tools: [] as ToolConfig[] })),
        getCategories().catch(() => [] as ToolCategory[]),
      ]);
      const safeCats = Array.isArray(cats) ? cats : [];
      setCategories(safeCats);
      const catMap = new Map<string, string>();
      const subMap = new Map<string, string>();
      for (const c of safeCats) {
        catMap.set(c.id, c.name);
        for (const s of c.items) subMap.set(`${c.id}/${s.id}`, s.name);
      }
      const seen = new Set<string>();
      const enriched: ToolWithMeta[] = [];
      for (const t of (scanResult.tools || [])) {
        if (seen.has(t.id)) continue;
        seen.add(t.id);
        enriched.push({
          ...t,
          categoryName: catMap.get(t.category) || t.category,
          subcategoryName: subMap.get(`${t.category}/${t.subcategory}`) || t.subcategory,
        });
      }
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

  const handleCancelInstall = useCallback(() => {
    cancelRef.current = true;
    cancelDownload().catch(() => {});
    setBusy(null);
    setDlProgress(null);
    setInstallProgress({});
  }, []);

  const handleInstall = useCallback(async (tool: ToolWithMeta) => {
    if (busy) return;
    cancelRef.current = false;
    const method = tool.install?.method;
    if (!method) { setError(t("toolManager.noInstallMethod", { name: tool.name })); return; }

    const proxyUrl = await getProxy();

    if (tool.runtime === "python" && tool.runtimeVersion) {
      const ver = tool.runtimeVersion;
      const envName = `python${ver}_env`;
      let envExists = false;
      try {
        const envsResult = await listPythonEnvs();
        if (envsResult.success) {
          envExists = envsResult.versions.some((v) => v.vendor === envName);
        }
      } catch { /* assume not exists */ }

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
      let javaReady = false;
      try {
        const javaResult = await listInstalledJava();
        if (javaResult.success && javaResult.versions.length > 0) {
          javaReady = javaResult.versions.some((v) => v.version.startsWith(`${requiredMajor}.`) || v.version === requiredMajor);
        }
      } catch { /* assume not installed */ }

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
          }
          if (!identifier) {
            setError(t("install.javaNotFound", { ver: requiredMajor }));
            setBusy(null);
            setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
            return;
          }
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
          const release = await fetchGitHubRelease(owner, repo);
          releaseVersion = release.tag_name;
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
        } catch (releaseErr) {
          const errStr = String(releaseErr);
          if (errStr.includes("403")) {
            throw new Error(t("install.githubRateLimit"));
          }
        }

        if (binaryAsset) {
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.downloadRelease") }));
          const result = await downloadAndExtract({ url: binaryAsset.browser_download_url, fileName: binaryAsset.name, useProxy: !!proxyUrl });
          if (cancelRef.current) return;
          if (!result.success) throw new Error(result.error || t("install.downloadFailed"));
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.installing") }));

          const stableDirName = tool.name;
          if (result.extract_path) {
            const actualDir = result.extract_path.split("/").pop() || "";
            if (actualDir && actualDir !== stableDirName) {
              try {
                await invoke("pentest_rename_tool_dir", { fromPath: result.extract_path, toName: stableDirName });
              } catch { /* rename failed, non-critical */ }
            }
          }

          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.detectExecutable") }));
          try {
            const execs: string[] = await invoke("pentest_find_tool_executables", {
              toolDir: stableDirName, runtime: tool.runtime || null,
            });

            let selectedExec: string | null = null;
            if (execs.length === 1) {
              selectedExec = execs[0];
            } else if (execs.length > 1) {
              selectedExec = await new Promise<string | null>((resolve) => {
                setExecPicker({ tool, dirName: stableDirName, candidates: execs, resolve });
              });
            }

            if (selectedExec) {
              const newExecutable = `${stableDirName}/${selectedExec}`;
              await invoke("pentest_update_tool_executable", {
                toolId: tool.id, category: tool.category, subcategory: tool.subcategory,
                newExecutable, version: releaseVersion || undefined,
                lastUpdated: new Date().toISOString().slice(0, 10),
              });
            } else if (releaseVersion) {
              await invoke("pentest_update_tool_executable", {
                toolId: tool.id, category: tool.category, subcategory: tool.subcategory,
                newExecutable: tool.executable, version: releaseVersion,
                lastUpdated: new Date().toISOString().slice(0, 10),
              });
            }
          } catch { /* scan/update failed, non-critical */ }
        } else {
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.gitCloning") }));
          const toolDir = tool.executable?.split("/")[0] || tool.name;
          const cloneUrl = `https://github.com/${source}.git`;
          await invoke("pentest_git_clone_tool", { source: cloneUrl, toolDir, proxyUrl: proxyUrl || null, runtime: tool.runtime || null });
        }
      } else if (method === "homebrew") {
        setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.brewInstalling") }));
        const pkg = tool.install?.source || tool.name;
        const brewResult = await installRuntime(`brew:${pkg}`, proxyUrl);
        const brewVerMatch = brewResult.message?.match(/BREW_VERSION=(.+)/);
        if (brewVerMatch) {
          await invoke("pentest_update_tool_executable", {
            toolId: tool.id, category: tool.category, subcategory: tool.subcategory,
            newExecutable: tool.executable, version: brewVerMatch[1],
            lastUpdated: new Date().toISOString().slice(0, 10),
          });
        }
      } else if (method === "gem") {
        const pkg = tool.install?.source || tool.name;
        setInstallProgress((p) => ({ ...p, [tool.id]: `Installing ${pkg} via gem...` }));
        const gemResult = await installRuntime(`gem:${pkg}`, proxyUrl);
        if (!gemResult.success) {
          throw new Error(gemResult.message || `gem install ${pkg} failed`);
        }
      }

      if (tool.runtime === "python" && tool.runtimeVersion) {
        const toolDir = tool.executable?.split("/")[0] || tool.name;
        try {
          const hasReqs = await invoke<boolean>("pentest_check_requirements", { toolDir });
          if (hasReqs) {
            setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.installingPythonDeps") }));
            await invoke("pentest_install_requirements", {
              toolDir, pythonVersion: tool.runtimeVersion, proxyUrl: proxyUrl || null,
            });
          }
        } catch { /* requirements install failed, non-critical */ }
      }

      await loadData();
    } catch (e) {
      setError(t("toolManager.installFailed", { error: e }));
    } finally {
      setBusy(null);
      setDlProgress(null);
      setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
    }
  }, [busy, getProxy, loadData]);

  const doUninstall = useCallback(async (tool: ToolWithMeta) => {
    if (busy) return;
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
  }, [busy, loadData]);

  const [depPicker, setDepPicker] = useState<{ tool: ToolWithMeta; files: string[] } | null>(null);
  const [execPicker, setExecPicker] = useState<ExecPickerState | null>(null);

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
        toolDir, fileName, pythonVersion: tool.runtimeVersion || "", proxyUrl: proxyUrl || null,
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

  const checkForUpdates = useCallback(async () => {
    setCheckingUpdates(true);
    try {
      const updates = await invoke<ToolUpdateInfo[]>("pentest_check_tool_updates");
      setToolUpdates(updates);
      setShowUpdates(true);
    } catch {
      setToolUpdates([]);
    }
    setCheckingUpdates(false);
  }, []);

  // ── Editor functions ──

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
    if (editorDirty || skillDirty) { setShowCloseConfirm(true); return; }
    animateClose();
  }, [editorDirty, skillDirty, animateClose]);

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

      const category = (formData.category as string) || editingTool.category || "misc";
      const subcategory = (formData.subcategory as string) || editingTool.subcategory || "other";
      const toolId = (formData.id as string) || editingTool.id;

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

  const handleSwitchMode = useCallback((mode: "form" | "raw" | "skills" | "output") => {
    if (mode === "skills") {
      setEditorMode("skills");
      if (editingTool) {
        listSkills(editingTool.name).then(setSkillsList).catch(() => setSkillsList([]));
        setActiveSkillId(null);
        setSkillContent("");
        setSkillDirty(false);
      }
      return;
    }
    if (mode === "output") {
      syncRawToForm(rawJson);
      setEditorMode("output");
      return;
    }
    if (mode === "raw") syncFormToRaw(formData);
    else syncRawToForm(rawJson);
    setEditorMode(mode);
  }, [editorMode, formData, rawJson, syncFormToRaw, syncRawToForm, editingTool]);

  // ── Skills ──

  const loadSkillContent = useCallback(async (skillId: string) => {
    if (!editingTool) return;
    try {
      const content = await readSkill(editingTool.name, skillId);
      setActiveSkillId(skillId);
      setSkillContent(content);
      setSkillDirty(false);
    } catch {
      setActiveSkillId(skillId);
      setSkillContent("");
      setSkillDirty(false);
    }
  }, [editingTool]);

  const handleSaveSkill = useCallback(async () => {
    if (!editingTool || !activeSkillId) return;
    setSkillSaving(true);
    try {
      await writeSkill(editingTool.name, activeSkillId, skillContent);
      setSkillDirty(false);
    } catch (e) {
      console.error("[Skills] Save failed:", e);
    } finally {
      setSkillSaving(false);
    }
  }, [editingTool, activeSkillId, skillContent]);

  const handleCreateSkill = useCallback(async () => {
    if (!editingTool || !newSkillName.trim()) return;
    const id = newSkillName.trim().toLowerCase().replace(/\s+/g, "-").replace(/[^a-z0-9-]/g, "");
    if (!id) return;
    const template = `# ${newSkillName.trim()}\n\n## Description\n\nDescribe what this skill does.\n\n## Usage\n\n\`\`\`bash\n${editingTool.name} <args>\n\`\`\`\n\n## Notes\n\n- Add notes here\n`;
    try {
      await writeSkill(editingTool.name, id, template);
      const updated = await listSkills(editingTool.name);
      setSkillsList(updated);
      setActiveSkillId(id);
      setSkillContent(template);
      setSkillDirty(false);
      setNewSkillName("");
      setShowNewSkill(false);
    } catch (e) {
      console.error("[Skills] Create failed:", e);
    }
  }, [editingTool, newSkillName]);

  const handleDeleteSkill = useCallback(async (skillId: string) => {
    if (!editingTool) return;
    try {
      await deleteSkill(editingTool.name, skillId);
      const updated = await listSkills(editingTool.name);
      setSkillsList(updated);
      if (activeSkillId === skillId) {
        setActiveSkillId(null);
        setSkillContent("");
        setSkillDirty(false);
      }
    } catch (e) {
      console.error("[Skills] Delete failed:", e);
    }
  }, [editingTool, activeSkillId]);

  // ── Add / Import ──

  const handleAddTool = useCallback(() => {
    const id = Math.random().toString(36).substring(2, 10);
    const defaults: Record<string, unknown> = {
      id, name: "", description: "", icon: "🔧", executable: "",
      runtime: "native", runtimeVersion: "", ui: "cli", params: [],
      install: { method: "", source: "" },
    };
    const json = JSON.stringify({ tool: defaults }, null, 2);
    const placeholder: ToolWithMeta = {
      ...defaults, name: t("toolManager.newTool"),
      category: "misc", subcategory: "other", installed: false,
      categoryName: "misc", subcategoryName: "other",
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

  const handleGithubImport = useCallback(async () => {
    const url = githubUrl.trim();
    if (!url) return;
    let owner = "", repo = "";
    const ghMatch = url.match(/github\.com\/([^/]+)\/([^/\s#?]+)/);
    if (ghMatch) {
      owner = ghMatch[1];
      repo = ghMatch[2].replace(/\.git$/, "");
    } else if (url.includes("/") && !url.includes(" ")) {
      const parts = url.split("/");
      owner = parts[0];
      repo = parts[1]?.replace(/\.git$/, "") || "";
    }
    if (!owner || !repo) {
      setError("Invalid GitHub URL. Use owner/repo or https://github.com/owner/repo");
      return;
    }

    setGithubAnalyzing(true);
    setError(null);
    try {
      const suggestion = await invoke<{
        name: string; description: string; icon: string; runtime: string;
        runtime_version: string; ui: string; install_method: string;
        install_source: string; executable: string; category: string;
        subcategory: string; readme_excerpt: string;
      }>("pentest_analyze_github_tool", { owner, repo });

      const id = Math.random().toString(36).substring(2, 10);
      const toolData: Record<string, unknown> = {
        id, name: suggestion.name, description: suggestion.description,
        icon: suggestion.icon, executable: suggestion.executable,
        runtime: suggestion.runtime, runtimeVersion: suggestion.runtime_version,
        ui: suggestion.ui, params: [],
        install: { method: suggestion.install_method, source: suggestion.install_source },
      };
      const json = JSON.stringify({ tool: toolData }, null, 2);
      const placeholder: ToolWithMeta = {
        ...toolData, category: suggestion.category, subcategory: suggestion.subcategory,
        installed: false, categoryName: suggestion.category, subcategoryName: suggestion.subcategory,
      } as unknown as ToolWithMeta;

      setShowGithubImport(false);
      setGithubUrl("");
      setEditingTool(placeholder);
      setEditorMode("form");
      setEditorDirty(true);
      setEditorLoading(false);
      originalJsonRef.current = json;
      setRawJson(json);
      setFormData(toolData);
      requestAnimationFrame(() => setEditorVisible(true));
    } catch (e) {
      setError(String(e));
    } finally {
      setGithubAnalyzing(false);
    }
  }, [githubUrl]);

  // ── Context menu ──

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
      case "copy-id": copyToClipboard(tool.id); break;
      case "open-dir":
        invoke("pentest_open_directory", { executable: tool.executable || tool.name }).catch(() => {});
        break;
      case "delete": setDeleteTarget(tool); break;
    }
  }, [ctxMenu, openEditor, handleUninstall, handleInstall, handleInstallDeps, loadData]);

  const handleDeleteTool = useCallback(async (tool: ToolWithMeta) => {
    setDeleteTarget(null);
    try {
      await invoke("pentest_delete_tool", {
        toolId: tool.id, category: tool.category, subcategory: tool.subcategory, toolFolder: null,
      });
      await loadData();
    } catch (e) {
      setError(t("toolManager.deleteFailed", { error: e }));
    }
  }, [loadData]);

  // ── Filtering and sorting ──

  const installedCount = tools.filter((t) => t.installed).length;
  const allCategories = Array.from(new Set(tools.map((t) => t.category)));

  const filteredTools = tools
    .filter((t) => {
      if (selectedCategory && t.category !== selectedCategory) return false;
      if (selectedTier && (t.tier || "optional") !== selectedTier) return false;
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

  const categoryDisplayName = (catId: string) => (categories ?? []).find((c) => c.id === catId)?.name || catId;

  const fieldCtx: EditorFieldsContext = { formData, handleFormChange };
  const actionCtx: ActionButtonProps = {
    busy, installProgress, dlProgress,
    onCancel: handleCancelInstall, onUninstall: handleUninstall, onInstall: handleInstall,
  };

  const renderToolList = (items: ToolWithMeta[]) =>
    viewMode === "grid" ? (
      <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
        {items.map((tool) => <GridCard key={tool.id} tool={tool} onOpen={openEditor} onContextMenu={handleContextMenu} actionCtx={actionCtx} />)}
      </div>
    ) : (
      <div className="space-y-1">
        {items.map((tool) => <ListRow key={tool.id} tool={tool} onOpen={openEditor} onContextMenu={handleContextMenu} actionCtx={actionCtx} />)}
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
                  {(editorDirty || skillDirty) && <span className="w-2 h-2 rounded-full bg-accent/60 flex-shrink-0" title={t("toolManager.unsavedChanges")} />}
                </div>
                <p className="text-[11px] text-muted-foreground/50 mt-0.5">{t("toolManager.editToolConfig")}</p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <div className="flex items-center rounded-lg border border-border/15 overflow-hidden">
                {(["form", "skills", "output", "raw"] as const).map((mode) => (
                  <button key={mode} type="button" onClick={() => handleSwitchMode(mode)}
                    className={cn("flex items-center gap-1.5 px-3 py-1.5 text-[11px] transition-colors",
                      editorMode === mode ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                    {mode === "form" && <><FileText className="w-3 h-3" /> {t("toolManager.form")}</>}
                    {mode === "skills" && <><BookOpen className="w-3 h-3" /> Skills</>}
                    {mode === "output" && <><ArrowUpDown className="w-3 h-3" /> Output</>}
                    {mode === "raw" && <><Code2 className="w-3 h-3" /> {t("toolManager.json")}</>}
                  </button>
                ))}
              </div>
              {editorMode === "skills" ? (
                <button type="button" onClick={handleSaveSkill} disabled={skillSaving || !skillDirty}
                  className={cn("flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium transition-colors",
                    skillDirty ? "bg-accent text-accent-foreground hover:bg-accent/90" : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed")}>
                  {skillSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />} {t("common.save")}
                </button>
              ) : (
                <button type="button" onClick={handleSave} disabled={saving || !editorDirty}
                  className={cn("flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium transition-colors",
                    editorDirty ? "bg-accent text-accent-foreground hover:bg-accent/90" : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed")}>
                  {saving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />} {t("common.save")}
                </button>
              )}
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
              <button type="button" onClick={() => setShowGithubImport(true)} title={t("toolManager.importGithub")}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors">
                <Github className="w-3.5 h-3.5" /> {t("toolManager.importGithub")}
              </button>
              <button type="button" onClick={handleAddTool} title={t("toolManager.addTool")}
                className="p-2 rounded-lg text-muted-foreground/50 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors">
                <Plus className="w-4 h-4" />
              </button>
              <button type="button" onClick={checkForUpdates} disabled={checkingUpdates} title="Check for Updates"
                className={cn("p-2 rounded-lg text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-30",
                  toolUpdates.some((u) => u.has_update) && "text-amber-400")}>
                <ArrowUpCircle className={cn("w-4 h-4", checkingUpdates && "animate-spin")} />
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
          ) : editorMode === "skills" ? (
            <div className="flex gap-4 h-full min-h-[400px]">
              <div className="w-[220px] flex-shrink-0 rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden flex flex-col">
                <div className="px-3 py-2 border-b border-border/8 flex items-center justify-between">
                  <span className="text-[11px] font-medium text-muted-foreground/60">Skills</span>
                  <button type="button" onClick={() => setShowNewSkill(true)}
                    className="p-1 rounded text-muted-foreground/40 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors">
                    <Plus className="w-3 h-3" />
                  </button>
                </div>
                {showNewSkill && (
                  <div className="px-2 py-2 border-b border-border/8 flex gap-1.5">
                    <input value={newSkillName} onChange={(e) => setNewSkillName(e.target.value)}
                      onKeyDown={(e) => { if (e.key === "Enter") handleCreateSkill(); if (e.key === "Escape") { setShowNewSkill(false); setNewSkillName(""); } }}
                      placeholder={t("toolManager.newSkillName", "Skill name...")}
                      autoFocus
                      className="flex-1 px-2 py-1 text-[11px] rounded bg-background border border-border/20 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40" />
                    <button type="button" onClick={handleCreateSkill} disabled={!newSkillName.trim()}
                      className="p-1 rounded text-accent hover:bg-accent/10 disabled:opacity-30 transition-colors">
                      <Check className="w-3 h-3" />
                    </button>
                    <button type="button" onClick={() => { setShowNewSkill(false); setNewSkillName(""); }}
                      className="p-1 rounded text-muted-foreground/40 hover:text-foreground transition-colors">
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                )}
                <div className="flex-1 overflow-y-auto">
                  {skillsList.length === 0 ? (
                    <div className="flex flex-col items-center justify-center h-24 gap-2">
                      <BookOpen className="w-4 h-4 text-muted-foreground/40" />
                      <span className="text-[11px] text-muted-foreground/50">{t("toolManager.noSkills", "No skills yet")}</span>
                    </div>
                  ) : (
                    skillsList.map((skill) => (
                      <div key={skill.id}
                        className={cn("group flex items-center gap-2 px-3 py-2 cursor-pointer transition-colors",
                          activeSkillId === skill.id ? "bg-accent/10 text-accent" : "text-foreground/70 hover:bg-[var(--bg-hover)]")}
                        onClick={() => loadSkillContent(skill.id)}>
                        <BookOpen className="w-3 h-3 flex-shrink-0 opacity-40" />
                        <span className="flex-1 text-[12px] truncate">{skill.name}</span>
                        <button type="button"
                          onClick={(e) => { e.stopPropagation(); handleDeleteSkill(skill.id); }}
                          className="p-0.5 rounded opacity-0 group-hover:opacity-40 hover:!opacity-100 hover:text-destructive transition-all">
                          <Trash2 className="w-2.5 h-2.5" />
                        </button>
                      </div>
                    ))
                  )}
                </div>
              </div>
              <div className="flex-1 min-w-0 rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden flex flex-col">
                {activeSkillId ? (
                  <>
                    <div className="px-3 py-2 border-b border-border/8 flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        <Pencil className="w-3 h-3 text-muted-foreground/50" />
                        <span className="text-[11px] font-medium text-muted-foreground/60">{activeSkillId}.md</span>
                        {skillDirty && <span className="w-1.5 h-1.5 rounded-full bg-accent/60" />}
                      </div>
                      <button type="button" onClick={handleSaveSkill} disabled={!skillDirty || skillSaving}
                        className={cn("flex items-center gap-1 px-2 py-1 rounded text-[10px] font-medium transition-colors",
                          skillDirty ? "bg-accent text-accent-foreground hover:bg-accent/90" : "text-muted-foreground/30 cursor-not-allowed")}>
                        {skillSaving ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : <Save className="w-2.5 h-2.5" />}
                        {t("common.save")}
                      </button>
                    </div>
                    <textarea value={skillContent}
                      onChange={(e) => { setSkillContent(e.target.value); setSkillDirty(true); }}
                      spellCheck={false}
                      className="flex-1 w-full px-4 py-3 text-[12px] font-mono leading-[1.7] bg-transparent text-foreground outline-none resize-none"
                      style={{ tabSize: 2 }} />
                  </>
                ) : (
                  <div className="flex-1 flex flex-col items-center justify-center gap-3">
                    <BookOpen className="w-8 h-8 text-muted-foreground/30" />
                    <p className="text-[12px] text-muted-foreground/50">{t("toolManager.selectSkill", "Select a skill to edit or create a new one")}</p>
                  </div>
                )}
              </div>
            </div>
          ) : editorMode === "output" ? (
            <OutputParserEditor formData={formData} onChange={(output) => {
              const updated = { ...formData, output };
              setFormData(updated);
              syncFormToRaw(updated);
              setEditorDirty(true);
            }} />
          ) : (
            <div className="flex gap-4 h-full">
              <div className="flex-1 min-w-0 space-y-4 overflow-y-auto">
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/60">{t("toolManager.basicInfo")}</span></div>
                  <FieldRow label={t("toolManager.name")} field="name" placeholder="dirsearch" ctx={fieldCtx} />
                  <FieldRow label={t("toolManager.icon")} field="icon" placeholder="📂" ctx={fieldCtx} />
                  <div className="flex items-start gap-3 py-2 px-3 rounded-lg hover:bg-[var(--bg-hover)]/30 transition-colors">
                    <span className="text-[12px] text-muted-foreground/60 w-24 flex-shrink-0 mt-1.5">{t("toolManager.description")}</span>
                    <textarea value={(formData.description as string) ?? ""} onChange={(e) => handleFormChange("description", e.target.value)}
                      placeholder={t("toolManager.descriptionPlaceholder")} rows={2}
                      className="flex-1 px-2 py-1.5 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors resize-y" />
                  </div>
                  <FieldRow label={t("common.version")} field="version" placeholder="1.0.0" ctx={fieldCtx} />
                  <FieldRow label="ID" field="id" mono placeholder="hash" ctx={fieldCtx} />
                  <FieldRow label={t("toolManager.executable")} field="executable" mono placeholder="tool/main.py" ctx={fieldCtx} />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/60">{t("toolManager.runtime")}</span></div>
                  <FieldRow label={t("toolManager.runtimeLabel")} field="runtime" type="select" options={[
                    { value: "python", label: "Python" }, { value: "java", label: "Java" },
                    { value: "node", label: "Node.js" }, { value: "native", label: "Native" },
                  ]} ctx={fieldCtx} />
                  {formData.runtime !== "native" && (
                    <FieldRow label={t("toolManager.runtimeVersion")} field="runtimeVersion" placeholder={
                      formData.runtime === "java" ? "17" : formData.runtime === "node" ? "20" : "3.11"
                    } ctx={fieldCtx} />
                  )}
                  <FieldRow label={t("toolManager.uiLabel")} field="ui" type="select" options={[
                    { value: "cli", label: "CLI" }, { value: "gui", label: "GUI" }, { value: "web", label: "Web" },
                  ]} ctx={fieldCtx} />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/60">{t("toolManager.installMethod")}</span></div>
                  <InstallFieldRow label={t("toolManager.installMethodLabel")} subField="method" type="select" options={[
                    { value: "", label: t("common.none") }, { value: "github", label: "GitHub" },
                    { value: "homebrew", label: "Homebrew" }, { value: "manual", label: t("toolManager.manual") },
                  ]} ctx={fieldCtx} />
                  <InstallFieldRow label={t("toolManager.source")} subField="source"
                    placeholder={
                      ((formData.install as Record<string, string>)?.method === "github") ? "owner/repo" :
                      ((formData.install as Record<string, string>)?.method === "homebrew") ? "formula-name" :
                      t("toolManager.source")
                    } mono ctx={fieldCtx} />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/60">{t("toolManager.paramConfig")}</span></div>
                  <div className="py-2"><ParamsEditor ctx={fieldCtx} /></div>
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8"><span className="text-[11px] font-medium text-muted-foreground/60">{t("toolManager.category")}</span></div>
                  <FieldRow label={t("toolManager.category")} field="category" type="select" options={
                    (categories ?? []).length > 0
                      ? (categories ?? []).map((c) => ({ value: c.id, label: c.name }))
                      : [{ value: "misc", label: "misc" }]
                  } ctx={fieldCtx} />
                  <FieldRow label={t("toolManager.subcategory")} field="subcategory" type="select" options={(() => {
                    const cat = (categories ?? []).find((c) => c.id === (formData.category as string));
                    if (cat && cat.items.length > 0) return cat.items.map((s) => ({ value: s.id, label: s.name }));
                    return [{ value: "other", label: "other" }];
                  })()} ctx={fieldCtx} />
                </div>
              </div>
              <div className="w-[380px] flex-shrink-0 rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden flex flex-col">
                <div className="px-3 py-2 border-b border-border/8 flex items-center gap-2">
                  <Code2 className="w-3 h-3 text-muted-foreground/30" />
                  <span className="text-[11px] font-medium text-muted-foreground/60">{t("toolManager.jsonPreview")}</span>
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

            <Select value={selectedCategory ?? "_all"} onValueChange={(v) => setSelectedCategory(v === "_all" ? null : v)}>
              <SelectTrigger size="sm" className="h-7 w-auto min-w-[120px] border-border/15 bg-[var(--bg-hover)]/30 text-[11px] shadow-none px-2.5 gap-1.5">
                <FolderOpen className="w-3 h-3 text-muted-foreground/40" />
                <SelectValue placeholder={t("common.all")} />
              </SelectTrigger>
              <SelectContent position="popper" className="min-w-[140px]">
                <SelectItem value="_all" className="text-[12px]">{t("common.all")}</SelectItem>
                {allCategories.map((catId) => (
                  <SelectItem key={catId} value={catId} className="text-[12px]">{categoryDisplayName(catId)}</SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Select value={selectedTier ?? "_all"} onValueChange={(v) => setSelectedTier(v === "_all" ? null : v)}>
              <SelectTrigger size="sm" className="h-7 w-auto min-w-[110px] border-border/15 bg-[var(--bg-hover)]/30 text-[11px] shadow-none px-2.5 gap-1.5">
                <SelectValue placeholder={t("common.all")} />
              </SelectTrigger>
              <SelectContent position="popper" className="min-w-[130px]">
                <SelectItem value="_all" className="text-[12px]">{t("common.all")}</SelectItem>
                <SelectItem value="essential" className="text-[12px] text-red-400">{t("toolManager.tierEssential")}</SelectItem>
                <SelectItem value="recommended" className="text-[12px] text-amber-400">{t("toolManager.tierRecommended")}</SelectItem>
                <SelectItem value="optional" className="text-[12px]">{t("toolManager.tierOptional")}</SelectItem>
              </SelectContent>
            </Select>

            <div className="ml-auto flex items-center gap-1">
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

              <div className="flex items-center rounded-md border border-border/10 overflow-hidden">
                <button type="button" onClick={() => setViewMode("grid")} title={t("toolManager.gridView")}
                  className={cn("p-1.5 transition-colors", viewMode === "grid" ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground")}>
                  <Grid3X3 className="w-3.5 h-3.5" />
                </button>
                <button type="button" onClick={() => setViewMode("list")} title={t("toolManager.listView")}
                  className={cn("p-1.5 transition-colors", viewMode === "list" ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground")}>
                  <List className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>
          </div>

          {/* Tool grid/list */}
          <div className="flex-1 overflow-y-auto px-6 py-4">
            {loading ? (
              <div key="tm-loading" className="flex items-center justify-center h-32">
                <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/50" />
              </div>
            ) : filteredTools.length === 0 ? (
              <div key="tm-empty" className="flex flex-col items-center justify-center h-32 gap-2 overflow-hidden">
                <span className="text-[12px] text-muted-foreground/60">
                  {search.trim() ? t("toolManager.noMatch") : t("toolManager.noTools")}
                </span>
                {!search.trim() && (
                  <button type="button" onClick={handleAddTool}
                    className="text-[11px] text-accent/60 hover:text-accent transition-colors flex items-center gap-1">
                    <Plus className="w-3 h-3" /> {t("toolManager.addFirstTool")}
                  </button>
                )}
              </div>
            ) : (() => {
              const shouldGroup = !selectedTier && !search.trim();
              const requiredTools = filteredTools.filter((t) => t.tier === "essential" || t.tier === "recommended");
              const optionalTools = filteredTools.filter((t) => !t.tier || t.tier === "optional");

              if (!shouldGroup) return renderToolList(filteredTools);

              return (
                <div className="space-y-6">
                  {requiredTools.length > 0 && (
                    <div>
                      <div className="flex items-center gap-2 mb-3">
                        <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-red-500/10 border border-red-500/20">
                          <div className="w-1.5 h-1.5 rounded-full bg-red-500" />
                          <span className="text-[11px] font-medium text-red-400">
                            {t("toolManager.requiredSection", "Required")}
                          </span>
                          <span className="text-[10px] text-red-400/50">{requiredTools.length}</span>
                        </div>
                        <span className="text-[10px] text-muted-foreground/60">
                          {t("toolManager.requiredHint", "Core tools needed for full functionality")}
                        </span>
                      </div>
                      {renderToolList(requiredTools)}
                    </div>
                  )}
                  {optionalTools.length > 0 && (
                    <div>
                      <div className="flex items-center gap-2 mb-3">
                        <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-muted/40 border border-border/30">
                          <div className="w-1.5 h-1.5 rounded-full bg-muted-foreground/40" />
                          <span className="text-[11px] font-medium text-muted-foreground/70">
                            {t("toolManager.optionalSection", "Optional")}
                          </span>
                          <span className="text-[10px] text-muted-foreground/50">{optionalTools.length}</span>
                        </div>
                        <span className="text-[10px] text-muted-foreground/60">
                          {t("toolManager.optionalHint", "Install as needed for specific tasks")}
                        </span>
                      </div>
                      {renderToolList(optionalTools)}
                    </div>
                  )}
                </div>
              );
            })()}
          </div>
        </>
      )}

      {/* Dialogs */}
      {ctxMenu && <ContextMenu ctx={ctxMenu} onAction={ctxAction} />}
      {uninstallTarget && <UninstallConfirmDialog target={uninstallTarget} onCancel={() => setUninstallTarget(null)} onConfirm={confirmUninstall} />}
      {depPicker && <DepPickerDialog tool={depPicker.tool} files={depPicker.files} onPick={doInstallDepFile} onCancel={() => setDepPicker(null)} />}
      {execPicker && <ExecPickerDialog state={execPicker} onDismiss={() => setExecPicker(null)} />}
      {deleteTarget && <DeleteConfirmDialog target={deleteTarget} onCancel={() => setDeleteTarget(null)} onConfirm={handleDeleteTool} />}
      {showCloseConfirm && <CloseConfirmDialog onCancel={() => setShowCloseConfirm(false)} onDiscard={forceCloseEditor} />}
      {showUpdates && <UpdatesDialog updates={toolUpdates} onClose={() => setShowUpdates(false)} />}
      {showGithubImport && <GitHubImportDialog url={githubUrl} onUrlChange={setGithubUrl} analyzing={githubAnalyzing} onImport={handleGithubImport} onCancel={() => { setShowGithubImport(false); setGithubUrl(""); }} />}
    </div>
  );
}
