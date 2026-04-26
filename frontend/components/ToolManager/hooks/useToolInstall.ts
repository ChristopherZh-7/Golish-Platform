import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import {
  fetchGitHubRelease, downloadAndExtract, cancelDownload, installRuntime,
  createPythonEnv, listPythonEnvs, listInstalledJava, listAvailableJava,
  installJavaVersion, fixToolExecutablePermission, uninstallBrewPackage,
  uninstallGemPackage, getConfig,
} from "@/lib/pentest/api";
import { getSettings } from "@/lib/settings";
import type { ToolWithMeta } from "../OutputParserEditor";
import type { ExecPickerState, ToolUpdateInfo } from "../Dialogs";

export function useToolInstall(
  loadData: (silent?: boolean) => Promise<void>,
  setError: (err: string | null) => void,
) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState<string | null>(null);
  const cancelRef = useRef(false);
  const [installProgress, setInstallProgress] = useState<Record<string, string>>({});
  const [dlProgress, setDlProgress] = useState<{ downloaded: number; total: number } | null>(null);
  const [uninstallTarget, setUninstallTarget] = useState<ToolWithMeta | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ToolWithMeta | null>(null);
  const [depPicker, setDepPicker] = useState<{ tool: ToolWithMeta; files: string[] } | null>(null);
  const [execPicker, setExecPicker] = useState<ExecPickerState | null>(null);
  const [toolUpdates, setToolUpdates] = useState<ToolUpdateInfo[]>([]);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [showUpdates, setShowUpdates] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let lastUpdate = 0;
    let rafId: number | null = null;
    let pending: { downloaded: number; total: number } | null = null;
    const flush = () => { if (pending) { setDlProgress(pending); pending = null; } rafId = null; lastUpdate = Date.now(); };
    listen<{ downloaded: number; total: number }>("download-progress", (e) => {
      pending = e.payload;
      const now = Date.now();
      if (now - lastUpdate >= 250) flush();
      else if (!rafId) rafId = window.setTimeout(flush, 250 - (now - lastUpdate));
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); if (rafId) clearTimeout(rafId); };
  }, []);

  const getProxy = useCallback(async () => {
    const s = await getSettings().catch(() => null);
    return s?.network?.proxy_url || undefined;
  }, []);

  const handleCancelInstall = useCallback(() => {
    cancelRef.current = true;
    cancelDownload().catch(() => {});
    setBusy(null); setDlProgress(null); setInstallProgress({});
  }, []);

  const handleInstall = useCallback(async (tool: ToolWithMeta) => {
    if (busy) return;
    cancelRef.current = false;
    const method = tool.install?.method;
    if (!method) { setError(t("toolManager.noInstallMethod", { name: tool.name })); return; }
    const proxyUrl = await getProxy();

    // Python env setup
    if (tool.runtime === "python" && tool.runtimeVersion) {
      const ver = tool.runtimeVersion.replace(/\+$/, "");
      const envName = `python${ver}_env`;
      let envExists = false;
      try { const r = await listPythonEnvs(); if (r.success) envExists = r.versions.some((v) => v.vendor === envName); } catch {}
      if (!envExists) {
        setError(null); setBusy(tool.id);
        setInstallProgress((p) => ({ ...p, [tool.id]: t("install.missingPythonEnv", { ver }) }));
        try {
          const r = await createPythonEnv(envName, ver, proxyUrl);
          if (!r.success) { setError(t("install.pythonEnvFailed", { ver, error: r.message })); setBusy(null); setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; }); return; }
        } catch (e) { setError(t("install.pythonEnvFailed", { ver, error: e })); setBusy(null); setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; }); return; }
      }
    }

    // Java setup
    if (tool.runtime === "java") {
      const requiredMajor = tool.runtimeVersion || "17";
      let javaReady = false;
      try { const r = await listInstalledJava(); if (r.success && r.versions.length > 0) javaReady = r.versions.some((v) => v.version.startsWith(`${requiredMajor}.`) || v.version === requiredMajor); } catch {}
      if (!javaReady) {
        setError(null); setBusy(tool.id);
        setInstallProgress((p) => ({ ...p, [tool.id]: t("install.missingJava", { ver: requiredMajor }) }));
        try {
          let identifier = "";
          const available = await listAvailableJava();
          if (available.success) {
            const majorMatches = available.versions.filter((v) => v.version.startsWith(`${requiredMajor}.`) || v.version === requiredMajor);
            const match = majorMatches.find((v) => v.version.includes("-fx")) || majorMatches.find((v) => v.version.endsWith("-tem")) || majorMatches[0];
            if (match) identifier = match.version;
          }
          if (!identifier) { setError(t("install.javaNotFound", { ver: requiredMajor })); setBusy(null); setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; }); return; }
          setInstallProgress((p) => ({ ...p, [tool.id]: t("install.installingJava", { id: identifier }) }));
          const r = await installJavaVersion(identifier, proxyUrl);
          if (!r.success) { setError(t("install.javaFailed", { ver: requiredMajor, error: r.message })); setBusy(null); setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; }); return; }
        } catch (e) { setError(t("install.javaFailed", { ver: requiredMajor, error: e })); setBusy(null); setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; }); return; }
      }
    }

    setBusy(tool.id);
    setInstallProgress((p) => ({ ...p, [tool.id]: t("common.preparing") }));
    setDlProgress(null); setError(null);
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
          const isMac = navigator.platform.toLowerCase().includes("mac") || navigator.platform.toLowerCase().includes("darwin");
          const SKIP_EXTS = [".txt", ".md", ".sha256", ".sha512", ".asc", ".sig", ".pem"];
          const isSkippable = (name: string) => SKIP_EXTS.some((e) => name.toLowerCase().endsWith(e)) || /checksums?/i.test(name) || /\.sbom\b/i.test(name);
          const archiveExts = [".zip", ".tar.gz", ".tgz", ".jar"];
          const isArchive = (name: string) => archiveExts.some((e) => name.toLowerCase().endsWith(e));
          const platformAssets = release.assets.filter((a) => {
            if (isSkippable(a.name)) return false;
            const n = a.name.toLowerCase();
            if (isMac) return n.includes("darwin") || n.includes("macos") || n.includes("mac") || n.includes("osx");
            return n.includes("linux");
          });
          binaryAsset = platformAssets.find((a) => isArchive(a.name)) || platformAssets[0] || release.assets.find((a) => !isSkippable(a.name) && isArchive(a.name)) || null;
        } catch (releaseErr) {
          if (String(releaseErr).includes("403")) throw new Error(t("install.githubRateLimit"));
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
              try { await invoke("pentest_rename_tool_dir", { fromPath: result.extract_path, toName: stableDirName }); } catch {}
            }
          }
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.detectExecutable") }));
          try {
            let execs: string[] = await invoke("pentest_find_tool_executables", { toolDir: stableDirName, runtime: tool.runtime || null });
            if (execs.length === 0) {
              try {
                const NON_EXEC_NAMES = new Set(["license","licence","readme","readme.md","readme.txt","changelog","changelog.md","contributing","contributing.md","authors","notice","code_of_conduct.md","security.md","makefile","dockerfile","docker-compose.yml","cargo.toml","cargo.lock","package.json","package-lock.json","go.mod","go.sum","gemfile","gemfile.lock","requirements.txt","setup.py","setup.cfg","pyproject.toml"]);
                const NON_EXEC_EXTS = new Set(["md","txt","rst","html","css","json","yaml","yml","toml","xml","csv","log","lock","cfg","ini","conf","png","jpg","jpeg","gif","svg","ico","pdf","doc","zip","tar","gz"]);
                const allFiles: string[] = await invoke("pentest_list_tool_dir_files", { toolDir: stableDirName });
                execs = allFiles.filter((f) => {
                  const base = f.split("/").pop()?.toLowerCase() || "";
                  if (NON_EXEC_NAMES.has(base)) return false;
                  const ext = base.includes(".") ? base.split(".").pop() || "" : "";
                  return !(ext && NON_EXEC_EXTS.has(ext));
                });
              } catch {}
            }
            const cfg = await getConfig().catch(() => null);
            const toolDirAbs = cfg?.tools_dir ? `${cfg.tools_dir.replace(/[/\\]+$/, "")}/${stableDirName}` : stableDirName;
            const selectedExec = await new Promise<string | null>((resolve) => {
              setExecPicker({ tool, dirName: stableDirName, toolDirAbs, candidates: execs, resolve });
            });
            if (selectedExec) {
              const newExecutable = `${stableDirName}/${selectedExec.replace(/^[/\\]+/, "")}`;
              await invoke("pentest_update_tool_executable", { toolId: tool.id, category: tool.category, subcategory: tool.subcategory, newExecutable, version: releaseVersion || undefined, lastUpdated: new Date().toISOString().slice(0, 10) });
            } else if (releaseVersion) {
              await invoke("pentest_update_tool_executable", { toolId: tool.id, category: tool.category, subcategory: tool.subcategory, newExecutable: tool.executable, version: releaseVersion, lastUpdated: new Date().toISOString().slice(0, 10) });
            }
          } catch {}
        } else {
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.gitCloning") }));
          const toolDir = tool.executable?.split("/")[0] || tool.name;
          await invoke("pentest_git_clone_tool", { source: `https://github.com/${source}.git`, toolDir, proxyUrl: proxyUrl || null, runtime: tool.runtime || null });
        }
      } else if (method === "homebrew") {
        setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.brewInstalling") }));
        const pkg = tool.install?.source || tool.name;
        const r = await installRuntime(`brew:${pkg}`, proxyUrl);
        if (!r.success) throw new Error(r.message || `brew install ${pkg} failed`);
        const m = r.message?.match(/BREW_VERSION=(.+)/);
        if (m) await invoke("pentest_update_tool_executable", { toolId: tool.id, category: tool.category, subcategory: tool.subcategory, newExecutable: tool.executable, version: m[1], lastUpdated: new Date().toISOString().slice(0, 10) });
      } else if (method === "gem") {
        const pkg = tool.install?.source || tool.name;
        setInstallProgress((p) => ({ ...p, [tool.id]: `Installing ${pkg} via gem...` }));
        const r = await installRuntime(`gem:${pkg}`, proxyUrl);
        if (!r.success) throw new Error(r.message || `gem install ${pkg} failed`);
      }

      if (tool.runtime === "python" && tool.runtimeVersion) {
        const toolDir = tool.executable?.split("/")[0] || tool.name;
        try {
          const hasReqs = await invoke<boolean>("pentest_check_requirements", { toolDir });
          if (hasReqs) {
            setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.installingPythonDeps") }));
            await invoke("pentest_install_requirements", { toolDir, pythonVersion: tool.runtimeVersion, proxyUrl: proxyUrl || null });
          }
        } catch {}
      }
      await loadData(true);
    } catch (e) {
      setError(t("toolManager.installFailed", { error: e }));
    } finally {
      setBusy(null); setDlProgress(null);
      setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; });
    }
  }, [busy, getProxy, loadData, setError, t]);

  const handleFixExecutablePermission = useCallback(async (tool: ToolWithMeta) => {
    if (busy) return;
    setBusy(tool.id);
    setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.fixingPermission") }));
    setError(null);
    try { await fixToolExecutablePermission(tool.executable); await loadData(true); }
    catch (e) { setError(t("toolManager.fixPermissionFailed", { error: e })); }
    finally { setBusy(null); setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; }); }
  }, [busy, loadData, t, setError]);

  const doUninstall = useCallback(async (tool: ToolWithMeta) => {
    if (busy) return;
    setBusy(tool.id);
    try {
      const via = tool.installedVia;
      const method = tool.install?.method;
      const pkg = tool.install?.source?.trim() || tool.name;
      if (via === "homebrew" || method === "homebrew") await uninstallBrewPackage(pkg);
      else if (via === "gem" || method === "gem") await uninstallGemPackage(pkg);
      else {
        const execHead = (tool.executable || "").split("/")[0];
        const isAbs = tool.executable.startsWith("/") || /^[A-Za-z]:[\\/]/.test(tool.executable);
        if (!execHead || isAbs) throw new Error(t("toolManager.uninstallNotManaged", { executable: tool.executable || tool.name }));
        await invoke("pentest_uninstall_tool_files", { toolDir: execHead });
      }
      await loadData(true);
    } catch (e) { setError(t("toolManager.uninstallFailed", { error: e })); }
    finally { setBusy(null); }
  }, [busy, loadData, t, setError]);

  const handleUninstall = useCallback((tool: ToolWithMeta) => setUninstallTarget(tool), []);
  const confirmUninstall = useCallback(async () => { if (!uninstallTarget) return; setUninstallTarget(null); await doUninstall(uninstallTarget); }, [uninstallTarget, doUninstall]);

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
      if (files.length === 1 && files[0].toLowerCase() === "requirements.txt") await doInstallDepFile(tool, files[0]);
      else setDepPicker({ tool, files });
    } catch (e) { setError(t("toolManager.scanFailed", { error: e })); }
  }, [t, setError]);

  const doInstallDepFile = useCallback(async (tool: ToolWithMeta, fileName: string) => {
    setDepPicker(null);
    const toolDir = tool.executable?.split("/")[0] || tool.name;
    setBusy(tool.id);
    setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.installingDeps", { file: fileName }) }));
    setError(null);
    try {
      const proxyUrl = await getProxy();
      await invoke("pentest_install_dep_file", { toolDir, fileName, pythonVersion: tool.runtimeVersion || "", proxyUrl: proxyUrl || null });
      setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.depInstallDone") }));
      await new Promise((r) => setTimeout(r, 1500));
    } catch (e) { setError(t("toolManager.depInstallFailed", { error: e })); }
    finally { setBusy(null); setInstallProgress((p) => { const n = { ...p }; delete n[tool.id]; return n; }); }
  }, [getProxy, t, setError]);

  const handleDeleteTool = useCallback(async (tool: ToolWithMeta) => {
    setDeleteTarget(null);
    try { await invoke("pentest_delete_tool", { toolId: tool.id, category: tool.category, subcategory: tool.subcategory, toolFolder: null }); await loadData(true); }
    catch (e) { setError(t("toolManager.deleteFailed", { error: e })); }
  }, [loadData, t, setError]);

  const checkForUpdates = useCallback(async () => {
    setCheckingUpdates(true);
    try { const updates = await invoke<ToolUpdateInfo[]>("pentest_check_tool_updates"); setToolUpdates(updates); setShowUpdates(true); }
    catch { setToolUpdates([]); }
    setCheckingUpdates(false);
  }, []);

  return {
    busy, installProgress, dlProgress,
    uninstallTarget, setUninstallTarget, deleteTarget, setDeleteTarget,
    depPicker, setDepPicker, execPicker, setExecPicker,
    toolUpdates, checkingUpdates, showUpdates, setShowUpdates,
    handleCancelInstall, handleInstall, handleFixExecutablePermission,
    handleUninstall, confirmUninstall, handleInstallDeps, doInstallDepFile,
    handleDeleteTool, checkForUpdates,
  };
}
