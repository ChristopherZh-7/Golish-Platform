import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { onCustomEvent } from "@/lib/events";
import {
  cancelDownload,
  cancelRuntimeInstall,
  checkRequirements,
  checkToolUpdates,
  createPythonEnv,
  deleteTool,
  downloadAndExtract,
  fetchGitHubRelease,
  findToolExecutables,
  fixToolExecutablePermission,
  getConfig,
  gitCloneTool,
  installDepFile,
  installRequirements,
  installRuntime,
  listDepFiles,
  listInstalledJava,
  listPythonEnvs,
  listToolDirFiles,
  pipInstall,
  pipUninstall,
  renameToolDir,
  uninstallBrewPackage,
  uninstallGemPackage,
  uninstallToolFiles,
  updateToolExecutable,
} from "@/lib/pentest/api";
import {
  ensureJavaInstalled,
  mapJavaInstallError,
  mapJavaProgressStage,
} from "@/lib/pentest/javaInstaller";
import { getSettings } from "@/lib/settings";
import type { ExecPickerState, ToolUpdateInfo } from "../Dialogs";
import type { ToolWithMeta } from "../OutputParserEditor";
import { filterCandidateExecutables } from "./toolInstall/executableFilter";
import { selectGitHubAsset } from "./toolInstall/githubAsset";
import { isAutoInstallMethod, resolveInstallForPlatform } from "./toolInstall/installPlatform";

export {
  detectInstallPlatform,
  isAutoInstallMethod,
  resolveInstallForPlatform,
} from "./toolInstall/installPlatform";

interface InstallOptions {
  interactive?: boolean;
  refresh?: boolean;
  force?: boolean;
  reportError?: boolean;
}

interface InstallResult {
  success: boolean;
  error?: string;
  cancelled?: boolean;
}

export function useToolInstall(
  loadData: (silent?: boolean) => Promise<void>,
  setError: (err: string | null) => void
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
    const flush = () => {
      if (pending) {
        setDlProgress(pending);
        pending = null;
      }
      rafId = null;
      lastUpdate = Date.now();
    };
    onCustomEvent<{ downloaded: number; total: number }>("download-progress", (payload) => {
      pending = payload;
      const now = Date.now();
      if (now - lastUpdate >= 250) flush();
      else if (!rafId) rafId = window.setTimeout(flush, 250 - (now - lastUpdate));
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
      if (rafId) clearTimeout(rafId);
    };
  }, []);

  const getProxy = useCallback(async () => {
    const s = await getSettings().catch(() => null);
    return s?.network?.proxy_url || undefined;
  }, []);

  const handleCancelInstall = useCallback(() => {
    cancelRef.current = true;
    cancelDownload().catch(() => {});
    cancelRuntimeInstall().catch(() => {});
    setBusy(null);
    setDlProgress(null);
    setInstallProgress({});
  }, []);

  const handleInstall = useCallback(
    async (tool: ToolWithMeta, options: InstallOptions = {}): Promise<InstallResult> => {
      const interactive = options.interactive ?? true;
      const refresh = options.refresh ?? true;
      const reportError = options.reportError ?? true;
      if (busy && !options.force) return { success: false, cancelled: false };
      cancelRef.current = false;
      const resolved = resolveInstallForPlatform(tool.install ?? null);
      if (!resolved) {
        const error = t("toolManager.windowsManualInstall", {
          name: tool.name,
          defaultValue:
            "{{name}} 在 Windows 上需要手动安装，请参考 docs/windows-support.md（可用 winget / scoop 等价方案）",
        });
        if (reportError) setError(error);
        return { success: false, error };
      }
      const method = resolved.method;
      const resolvedSource = resolved.source;
      if (!isAutoInstallMethod(method)) {
        const error = t("toolManager.noInstallMethod", { name: tool.name });
        if (reportError) setError(error);
        return { success: false, error };
      }
      const proxyUrl = await getProxy();
      let installedToolDir: string | null = null;

      // Python env setup
      if (tool.runtime === "python" && tool.runtimeVersion) {
        const ver = tool.runtimeVersion.replace(/\+$/, "");
        const envName = `python${ver}_env`;
        let envExists = false;
        try {
          const r = await listPythonEnvs();
          if (r.success) envExists = r.versions.some((v) => v.vendor === envName);
        } catch {}
        if (!envExists) {
          setError(null);
          setBusy(tool.id);
          setInstallProgress((p) => ({ ...p, [tool.id]: t("install.missingPythonEnv", { ver }) }));
          try {
            const r = await createPythonEnv(envName, ver, proxyUrl);
            if (!r.success) {
              const error = t("install.pythonEnvFailed", { ver, error: r.message });
              if (reportError) setError(error);
              setBusy(null);
              setInstallProgress((p) => {
                const n = { ...p };
                delete n[tool.id];
                return n;
              });
              return { success: false, error };
            }
          } catch (e) {
            const error = t("install.pythonEnvFailed", { ver, error: e });
            if (reportError) setError(error);
            setBusy(null);
            setInstallProgress((p) => {
              const n = { ...p };
              delete n[tool.id];
              return n;
            });
            return { success: false, error };
          }
        }
      }

      // Java setup
      if (tool.runtime === "java") {
        const requiredMajor = tool.runtimeVersion || "17";
        let javaReady = false;
        try {
          const r = await listInstalledJava();
          if (r.success && r.versions.length > 0)
            javaReady = r.versions.some(
              (v) => v.version.startsWith(`${requiredMajor}.`) || v.version === requiredMajor
            );
        } catch {}
        if (!javaReady) {
          setError(null);
          setBusy(tool.id);
          setInstallProgress((p) => ({
            ...p,
            [tool.id]: t("install.missingJava", { ver: requiredMajor }),
          }));
          try {
            // Delegate to the shared installer which, on Windows, runs the
            // Temurin → Microsoft.OpenJDK → direct-MSI fallback chain when
            // winget can't reach GitHub. Ported from grid-terminal-recover@d055e2ce.
            await ensureJavaInstalled(requiredMajor, {
              proxyUrl,
              onProgress: (stage) =>
                setInstallProgress((p) => ({
                  ...p,
                  [tool.id]: mapJavaProgressStage(stage, t),
                })),
            });
          } catch (e) {
            const error = mapJavaInstallError(String(e), requiredMajor, t);
            if (reportError) setError(error);
            setBusy(null);
            setInstallProgress((p) => {
              const n = { ...p };
              delete n[tool.id];
              return n;
            });
            return { success: false, error };
          }
        }
      }

      setBusy(tool.id);
      setInstallProgress((p) => ({ ...p, [tool.id]: t("common.preparing") }));
      setDlProgress(null);
      setError(null);
      try {
        if (method === "github") {
          const source = resolvedSource;
          if (!source) throw new Error(t("install.missingGithubSource"));
          const [owner, repo] = source.split("/");
          if (!owner || !repo) throw new Error(t("install.githubSourceFormat"));
          setInstallProgress((p) => ({ ...p, [tool.id]: t("install.detectMethod") }));

          let binaryAsset: { browser_download_url: string; name: string } | null = null;
          let releaseVersion: string | null = null;
          try {
            const release = await fetchGitHubRelease(owner, repo);
            releaseVersion = release.tag_name;
            binaryAsset = selectGitHubAsset(release.assets);
          } catch (releaseErr) {
            const message = String(releaseErr);
            if (message.includes("rate limit")) {
              throw new Error(t("install.githubRateLimit"));
            }
            throw releaseErr;
          }

          if (binaryAsset) {
            setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.downloadRelease") }));
            const result = await downloadAndExtract({
              url: binaryAsset.browser_download_url,
              fileName: binaryAsset.name,
              useProxy: !!proxyUrl,
            });
            if (cancelRef.current) return { success: false, cancelled: true };
            if (!result.success) throw new Error(result.error || t("install.downloadFailed"));
            setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.installing") }));
            const stableDirName = tool.name;
            installedToolDir = stableDirName;
            if (result.extract_path) {
              const actualDir = result.extract_path.split("/").pop() || "";
              if (actualDir && actualDir !== stableDirName) {
                try {
                  await renameToolDir(result.extract_path, stableDirName);
                } catch {}
              }
            }
            setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.detectExecutable") }));
            try {
              let execs: string[] = await findToolExecutables(stableDirName, tool.runtime || null);
              if (execs.length === 0) {
                try {
                  const allFiles: string[] = await listToolDirFiles(stableDirName);
                  execs = filterCandidateExecutables(allFiles);
                } catch {}
              }
              const cfg = await getConfig().catch(() => null);
              const toolDirAbs = cfg?.tools_dir
                ? `${cfg.tools_dir.replace(/[/\\]+$/, "")}/${stableDirName}`
                : stableDirName;
              const selectedExec = interactive
                ? await new Promise<string | null>((resolve) => {
                    setExecPicker({
                      tool,
                      dirName: stableDirName,
                      toolDirAbs,
                      candidates: execs,
                      resolve,
                    });
                  })
                : execs[0] || null;
              if (selectedExec) {
                const newExecutable = `${stableDirName}/${selectedExec.replace(/^[/\\]+/, "")}`;
                await updateToolExecutable({
                  toolId: tool.id,
                  newExecutable,
                  version: releaseVersion || undefined,
                  lastUpdated: new Date().toISOString().slice(0, 10),
                });
              } else if (releaseVersion) {
                await updateToolExecutable({
                  toolId: tool.id,
                  newExecutable: tool.executable,
                  version: releaseVersion,
                  lastUpdated: new Date().toISOString().slice(0, 10),
                });
              }
            } catch {}
          } else {
            setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.gitCloning") }));
            const toolDir = tool.executable?.split("/")[0] || tool.name;
            installedToolDir = toolDir;
            await gitCloneTool({
              source: `https://github.com/${source}.git`,
              toolDir,
              proxyUrl: proxyUrl || null,
              runtime: tool.runtime || null,
            });
          }
        } else if (method === "homebrew") {
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.brewInstalling") }));
          const pkg = resolvedSource || tool.name;
          const r = await installRuntime(`brew:${pkg}`, proxyUrl);
          if (!r.success) throw new Error(r.message || `brew install ${pkg} failed`);
          const m = r.message?.match(/BREW_VERSION=(.+)/);
          if (m)
            await updateToolExecutable({
              toolId: tool.id,
              newExecutable: tool.executable,
              version: m[1],
              lastUpdated: new Date().toISOString().slice(0, 10),
            });
        } else if (method === "homebrew-cask") {
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.brewInstalling") }));
          const pkg = resolvedSource || tool.name;
          const r = await installRuntime(`brew-cask:${pkg}`, proxyUrl);
          if (!r.success) throw new Error(r.message || `brew install --cask ${pkg} failed`);
        } else if (method === "gem") {
          const pkg = resolvedSource || tool.name;
          setInstallProgress((p) => ({ ...p, [tool.id]: `Installing ${pkg} via gem...` }));
          const r = await installRuntime(`gem:${pkg}`, proxyUrl);
          if (!r.success) throw new Error(r.message || `gem install ${pkg} failed`);
        } else if (method === "pip") {
          const pkg = resolvedSource || tool.name;
          const ver = (tool.runtimeVersion || "").replace(/\+$/, "");
          const envName = ver ? `python${ver}_env` : "base";
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.pipInstalling", { pkg }) }));
          const r = await pipInstall(envName, pkg);
          if (!r.success) throw new Error(r.message || `pip install ${pkg} failed`);
        }

        if (tool.runtime === "python" && tool.runtimeVersion) {
          const toolDir = installedToolDir || tool.executable?.split("/")[0] || tool.name;
          try {
            const hasReqs = await checkRequirements(toolDir);
            if (hasReqs) {
              setInstallProgress((p) => ({
                ...p,
                [tool.id]: t("toolManager.installingPythonDeps"),
              }));
              await installRequirements(toolDir, tool.runtimeVersion || null);
            }
          } catch {}
        }
        if (refresh) await loadData(true);
        return { success: true };
      } catch (e) {
        const error = t("toolManager.installFailed", { error: e });
        if (reportError) setError(error);
        return { success: false, error };
      } finally {
        setBusy(null);
        setDlProgress(null);
        setInstallProgress((p) => {
          const n = { ...p };
          delete n[tool.id];
          return n;
        });
      }
    },
    [busy, getProxy, loadData, setError, t]
  );

  const handleFixExecutablePermission = useCallback(
    async (tool: ToolWithMeta) => {
      if (busy) return;
      setBusy(tool.id);
      setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.fixingPermission") }));
      setError(null);
      try {
        await fixToolExecutablePermission(tool.executable);
        await loadData(true);
      } catch (e) {
        setError(t("toolManager.fixPermissionFailed", { error: e }));
      } finally {
        setBusy(null);
        setInstallProgress((p) => {
          const n = { ...p };
          delete n[tool.id];
          return n;
        });
      }
    },
    [busy, loadData, t, setError]
  );

  const doUninstall = useCallback(
    async (tool: ToolWithMeta) => {
      if (busy) return;
      setBusy(tool.id);
      try {
        const via = tool.installedVia;
        const method = tool.install?.method;
        const pkg = tool.install?.source?.trim() || tool.name;
        if (via === "homebrew" || method === "homebrew") await uninstallBrewPackage(pkg);
        else if (via === "gem" || method === "gem") await uninstallGemPackage(pkg);
        else if (via === "pip" || method === "pip") {
          const ver = (tool.runtimeVersion || "").replace(/\+$/, "");
          const envName = ver ? `python${ver}_env` : "base";
          const r = await pipUninstall(envName, pkg);
          if (!r.success) throw new Error(r.message || `pip uninstall ${pkg} failed`);
        } else {
          const execHead = (tool.executable || "").split("/")[0];
          const isAbs = tool.executable.startsWith("/") || /^[A-Za-z]:[\\/]/.test(tool.executable);
          if (!execHead || isAbs)
            throw new Error(
              t("toolManager.uninstallNotManaged", { executable: tool.executable || tool.name })
            );
          await uninstallToolFiles(execHead);
        }
        await loadData(true);
      } catch (e) {
        setError(t("toolManager.uninstallFailed", { error: e }));
      } finally {
        setBusy(null);
      }
    },
    [busy, loadData, t, setError]
  );

  const handleUninstall = useCallback((tool: ToolWithMeta) => setUninstallTarget(tool), []);
  const confirmUninstall = useCallback(async () => {
    if (!uninstallTarget) return;
    setUninstallTarget(null);
    await doUninstall(uninstallTarget);
  }, [uninstallTarget, doUninstall]);

  const doInstallDepFile = useCallback(
    async (tool: ToolWithMeta, fileName: string) => {
      setDepPicker(null);
      const toolDir = tool.executable?.split("/")[0] || tool.name;
      setBusy(tool.id);
      setInstallProgress((p) => ({
        ...p,
        [tool.id]: t("toolManager.installingDeps", { file: fileName }),
      }));
      setError(null);
      try {
        await getProxy();
        await installDepFile(toolDir, fileName);
        setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.depInstallDone") }));
        await new Promise((r) => setTimeout(r, 1500));
      } catch (e) {
        setError(t("toolManager.depInstallFailed", { error: e }));
      } finally {
        setBusy(null);
        setInstallProgress((p) => {
          const n = { ...p };
          delete n[tool.id];
          return n;
        });
      }
    },
    [getProxy, t, setError]
  );

  const handleInstallDeps = useCallback(
    async (tool: ToolWithMeta) => {
      if (tool.runtime !== "python" || !tool.runtimeVersion) return;
      const toolDir = tool.executable?.split("/")[0] || tool.name;
      try {
        const files = await listDepFiles(toolDir);
        if (files.length === 0) {
          setInstallProgress((p) => ({ ...p, [tool.id]: t("toolManager.noDepFiles") }));
          setTimeout(
            () =>
              setInstallProgress((p) => {
                const n = { ...p };
                delete n[tool.id];
                return n;
              }),
            2000
          );
          return;
        }
        if (files.length === 1 && files[0].toLowerCase() === "requirements.txt")
          await doInstallDepFile(tool, files[0]);
        else setDepPicker({ tool, files });
      } catch (e) {
        setError(t("toolManager.scanFailed", { error: e }));
      }
    },
    [doInstallDepFile, t, setError]
  );

  const handleDeleteTool = useCallback(
    async (tool: ToolWithMeta) => {
      setDeleteTarget(null);
      try {
        await deleteTool({ toolId: tool.id, toolFolder: null });
        await loadData(true);
      } catch (e) {
        setError(t("toolManager.deleteFailed", { error: e }));
      }
    },
    [loadData, t, setError]
  );

  const checkForUpdates = useCallback(async () => {
    setCheckingUpdates(true);
    try {
      const updates = await checkToolUpdates<ToolUpdateInfo>();
      setToolUpdates(updates);
      setShowUpdates(true);
    } catch {
      setToolUpdates([]);
    }
    setCheckingUpdates(false);
  }, []);

  const [batchInstalling, setBatchInstalling] = useState(false);
  const batchCancelRef = useRef(false);

  const handleInstallAllRequired = useCallback(
    async (tools: ToolWithMeta[]) => {
      if (busy || batchInstalling) return;
      const toInstall = tools.filter(
        (t) =>
          (t.tier === "essential" || t.tier === "recommended") &&
          !t.installed &&
          isAutoInstallMethod(t.install?.method)
      );
      if (toInstall.length === 0) return;
      setBatchInstalling(true);
      batchCancelRef.current = false;
      setError(null);
      const failures: string[] = [];
      try {
        for (const tool of toInstall) {
          if (batchCancelRef.current) break;
          const result = await handleInstall(tool, {
            interactive: false,
            refresh: false,
            force: true,
            reportError: false,
          });
          if (!result.success && !result.cancelled) {
            failures.push(`${tool.name}: ${result.error || "failed"}`);
          }
          await new Promise((r) => setTimeout(r, 300));
        }
        if (failures.length > 0 && !batchCancelRef.current) {
          setError(failures.slice(0, 3).join("; "));
        }
      } finally {
        setBatchInstalling(false);
        await loadData(true);
      }
    },
    [batchInstalling, busy, handleInstall, loadData, setError]
  );

  const cancelBatchInstall = useCallback(() => {
    batchCancelRef.current = true;
    cancelRef.current = true;
    cancelDownload().catch(() => {});
    cancelRuntimeInstall().catch(() => {});
  }, []);

  return {
    busy,
    installProgress,
    dlProgress,
    uninstallTarget,
    setUninstallTarget,
    deleteTarget,
    setDeleteTarget,
    depPicker,
    setDepPicker,
    execPicker,
    setExecPicker,
    toolUpdates,
    checkingUpdates,
    showUpdates,
    setShowUpdates,
    batchInstalling,
    handleInstallAllRequired,
    cancelBatchInstall,
    handleCancelInstall,
    handleInstall,
    handleFixExecutablePermission,
    handleUninstall,
    confirmUninstall,
    handleInstallDeps,
    doInstallDepFile,
    handleDeleteTool,
    checkForUpdates,
  };
}
