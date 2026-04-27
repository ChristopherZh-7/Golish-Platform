import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ToolWithMeta } from "../OutputParserEditor";

interface UseGithubImportOptions {
  openEditor: (tool: ToolWithMeta) => void;
  setError: (err: string | null) => void;
}

export function useGithubImport(opts: UseGithubImportOptions) {
  const { openEditor, setError } = opts;

  const [showGithubImport, setShowGithubImport] = useState(false);
  const [githubUrl, setGithubUrl] = useState("");
  const [githubAnalyzing, setGithubAnalyzing] = useState(false);

  const handleGithubImport = useCallback(async () => {
    const url = githubUrl.trim();
    if (!url) return;
    let owner = "",
      repo = "";
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
        name: string;
        description: string;
        icon: string;
        runtime: string;
        runtime_version: string;
        launch_mode: string;
        install_method: string;
        install_source: string;
        executable: string;
        category: string;
        subcategory: string;
        readme_excerpt: string;
      }>("pentest_analyze_github_tool", { owner, repo });
      const toolData: Record<string, unknown> = {
        id:
          crypto.randomUUID?.()?.replace(/-/g, "").slice(0, 8) ??
          Math.random().toString(36).slice(2, 10),
        name: suggestion.name,
        description: suggestion.description,
        icon: suggestion.icon,
        executable: suggestion.executable,
        runtime: suggestion.runtime,
        runtimeVersion: suggestion.runtime_version,
        launchMode: suggestion.launch_mode,
        params: [],
        install: { method: suggestion.install_method, source: suggestion.install_source },
      };
      const placeholder: ToolWithMeta = {
        ...toolData,
        category: suggestion.category,
        subcategory: suggestion.subcategory,
        installed: false,
        categoryName: suggestion.category,
        subcategoryName: suggestion.subcategory,
      } as unknown as ToolWithMeta;
      setShowGithubImport(false);
      setGithubUrl("");
      openEditor(placeholder);
    } catch (e) {
      setError(String(e));
    } finally {
      setGithubAnalyzing(false);
    }
  }, [githubUrl, openEditor, setError]);

  const openImportDialog = useCallback(() => setShowGithubImport(true), []);
  const closeImportDialog = useCallback(() => {
    setShowGithubImport(false);
    setGithubUrl("");
  }, []);

  return {
    showGithubImport,
    githubUrl,
    setGithubUrl,
    githubAnalyzing,
    handleGithubImport,
    openImportDialog,
    closeImportDialog,
  };
}
