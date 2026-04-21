import { useCallback, useEffect, useState } from "react";
import {
  Check, FileSearch, Loader2, Play, Square, Trash2, Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useStore } from "@/store";
import { StyledSelect } from "./shared";

interface WordlistOption { id: string; name: string; category: string; line_count: number }
interface SensitiveResult {
  id: string; baseUrl: string; probePath: string; fullUrl: string;
  statusCode: number; contentLength: number; contentType: string;
  isConfirmed: boolean; aiVerdict: string | null; createdAt: number;
}
interface SensitiveProgress {
  scanId: string; total: number; completed: number; hits: number;
  currentUrl: string; running: boolean; dirsFound: number;
}

export function SensitiveScanPanel() {
  const projectPath = useStore((s) => s.currentProjectPath);
  const [wordlists, setWordlists] = useState<WordlistOption[]>([]);
  const [selectedWordlist, setSelectedWordlist] = useState<string>("");
  const [ratePerSecond, setRatePerSecond] = useState(20);
  const [useSitemapDirs, setUseSitemapDirs] = useState(true);
  const [targetUrl, setTargetUrl] = useState("");
  const [results, setResults] = useState<SensitiveResult[]>([]);
  const [progress, setProgress] = useState<SensitiveProgress | null>(null);
  const [running, setRunning] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  useEffect(() => {
    invoke<WordlistOption[]>("wordlist_list").then((wl) => {
      setWordlists(Array.isArray(wl) ? wl : []);
    }).catch(() => {});
    invoke<SensitiveResult[]>("sensitive_scan_results", { projectPath, confirmedOnly: false }).then((r) => {
      setResults(Array.isArray(r) ? r : []);
    }).catch(() => {});
    invoke<boolean>("sensitive_scan_status").then((r) => setRunning(r)).catch(() => {});
  }, [projectPath]);

  useEffect(() => {
    const unlisten = listen<SensitiveProgress>("sensitive-scan-progress", (event) => {
      setProgress(event.payload);
      setRunning(event.payload.running);
      if (!event.payload.running) {
        invoke<SensitiveResult[]>("sensitive_scan_results", { projectPath, confirmedOnly: false }).then((r) => {
          setResults(Array.isArray(r) ? r : []);
        }).catch(() => {});
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [projectPath]);

  const handleStart = useCallback(async () => {
    try {
      await invoke("sensitive_scan_start", {
        config: {
          targetUrl: targetUrl || "http://localhost",
          wordlistId: selectedWordlist || null,
          ratePerSecond,
          useSitemapDirs,
        },
        projectPath,
      });
      setRunning(true);
    } catch (e) {
      console.error("Sensitive scan start failed:", e);
    }
  }, [targetUrl, selectedWordlist, ratePerSecond, useSitemapDirs, projectPath]);

  const handleStop = useCallback(async () => {
    await invoke("sensitive_scan_stop").catch(() => {});
  }, []);

  const handleClear = useCallback(async () => {
    await invoke("sensitive_scan_clear", { projectPath }).catch(() => {});
    setResults([]);
    setSelectedIds(new Set());
  }, [projectPath]);

  const toggleSelect = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  const selectAll = useCallback(() => {
    if (selectedIds.size === results.length) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(results.map((r) => r.id)));
    }
  }, [results, selectedIds.size]);

  const handleConfirm = useCallback(async () => {
    const ids = Array.from(selectedIds);
    if (ids.length === 0) return;
    await invoke("sensitive_scan_confirm", { ids, confirmed: true }).catch(() => {});
    setResults((prev) => prev.map((r) => selectedIds.has(r.id) ? { ...r, isConfirmed: true } : r));
    setSelectedIds(new Set());
  }, [selectedIds]);

  const [analyzing, setAnalyzing] = useState(false);
  const handleAiAnalyze = useCallback(async (onlyNew: boolean) => {
    const toAnalyze = onlyNew ? results.filter((r) => !r.aiVerdict) : results;
    if (toAnalyze.length === 0) return;
    setAnalyzing(true);
    try {
      const { initAiSession, sendPromptSession, shutdownAiSession } = await import("@/lib/ai");
      const { getSettings } = await import("@/lib/settings");
      const settings = await getSettings();
      const tmpId = `sensitive-ai-${Date.now()}`;

      const apiKey = settings.ai?.anthropic?.api_key || settings.ai?.openai?.api_key || settings.ai?.openrouter?.api_key || "";
      const provider = settings.ai?.anthropic?.api_key ? "anthropic"
        : settings.ai?.openai?.api_key ? "openai"
        : settings.ai?.openrouter?.api_key ? "openrouter" : "";
      const model = provider === "anthropic" ? "claude-sonnet-4-20250514"
        : provider === "openai" ? "gpt-4o"
        : provider === "openrouter" ? "anthropic/claude-sonnet-4-20250514" : "";

      if (!provider || !apiKey) {
        console.warn("[SensitiveScan] No AI provider configured");
        setAnalyzing(false);
        return;
      }

      await initAiSession(tmpId, { provider, workspace: ".", model, api_key: apiKey } as any);

      const items = toAnalyze.map((r) =>
        `- ${r.probePath} [${r.fullUrl}] status=${r.statusCode} size=${r.contentLength} type=${r.contentType}`
      ).join("\n");

      const prompt = `You are a security analyst. Classify each sensitive file scan result as true_positive, false_positive, or needs_review.
Respond ONLY with a raw JSON array: [{"path": "...", "verdict": "...", "reason": "..."}]
No markdown, no explanation outside the JSON.

Results:
${items}`;

      const response = await sendPromptSession(tmpId, prompt);
      shutdownAiSession(tmpId).catch(() => {});

      const jsonStart = response.indexOf("[");
      const jsonEnd = response.lastIndexOf("]");
      if (jsonStart >= 0 && jsonEnd > jsonStart) {
        const verdicts = JSON.parse(response.substring(jsonStart, jsonEnd + 1));
        if (Array.isArray(verdicts)) {
          const applied = await invoke<{ analyzed: number; true_positives: number }>(
            "sensitive_scan_apply_verdicts", { verdicts, projectPath }
          );
          const freshResults = await invoke<SensitiveResult[]>("sensitive_scan_results", { projectPath, confirmedOnly: false });
          setResults(Array.isArray(freshResults) ? freshResults : []);
          console.info("[SensitiveScan] AI analyzed:", applied);
        }
      }
    } catch (e) {
      console.error("[SensitiveScan] AI analysis failed:", e);
    }
    setAnalyzing(false);
  }, [results, projectPath]);

  const pct = progress && progress.total > 0 ? Math.round((progress.completed / progress.total) * 100) : 0;
  const dirWordlists = wordlists.filter((w) => w.category === "directories" || w.category === "fuzz" || w.category === "custom");

  return (
    <div className="h-full flex flex-col">
      <div className="px-4 py-3 border-b border-border/10 space-y-2 flex-shrink-0">
        <div className="flex items-center gap-2">
          <FileSearch className="w-3.5 h-3.5 text-amber-400" />
          <span className="text-[11px] font-semibold text-foreground/80">Sensitive File Scanner</span>
          {results.length > 0 && (
            <span className="text-[9px] text-muted-foreground/30 ml-auto">{results.length} hit{results.length !== 1 ? "s" : ""}</span>
          )}
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          <label className="flex items-center gap-1.5 text-[10px] text-muted-foreground/50">
            <input
              type="checkbox"
              checked={useSitemapDirs}
              onChange={(e) => setUseSitemapDirs(e.target.checked)}
              className="w-3 h-3 rounded accent-accent"
            />
            Sitemap dirs
          </label>

          {!useSitemapDirs && (
            <input
              value={targetUrl}
              onChange={(e) => setTargetUrl(e.target.value)}
              placeholder="https://target.com"
              className="flex-1 min-w-[140px] px-2 py-1 rounded-md text-[10px] bg-[var(--bg-input)] border border-border/10 text-foreground/80 placeholder:text-muted-foreground/20"
            />
          )}

          <StyledSelect
            value={selectedWordlist}
            onChange={setSelectedWordlist}
            options={[
              { value: "", label: `Built-in (${DEFAULT_SENSITIVE_PATH_COUNT} paths)` },
              ...dirWordlists.map((w) => ({ value: w.id, label: `${w.name} (${w.line_count})` })),
            ]}
            className="min-w-[140px]"
          />

          <div className="flex items-center gap-1 text-[10px] text-muted-foreground/40">
            <span>Rate:</span>
            <input
              type="number" min={1} max={200} value={ratePerSecond}
              onChange={(e) => setRatePerSecond(Math.max(1, Math.min(200, Number(e.target.value))))}
              className="w-12 px-1 py-0.5 rounded text-[10px] bg-[var(--bg-input)] border border-border/10 text-foreground/70 text-center"
            />
            <span>/s</span>
          </div>

          {!running ? (
            <button type="button" onClick={handleStart}
              className="flex items-center gap-1 px-2.5 py-1 rounded-md text-[10px] font-medium bg-amber-500/15 text-amber-400 hover:bg-amber-500/25 transition-colors">
              <Play className="w-3 h-3" /> Scan
            </button>
          ) : (
            <button type="button" onClick={handleStop}
              className="flex items-center gap-1 px-2.5 py-1 rounded-md text-[10px] font-medium bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">
              <Square className="w-3 h-3" /> Stop
            </button>
          )}

          {results.length > 0 && !running && (
            <>
              {analyzing ? (
                <span className="flex items-center gap-1 text-[9px] text-blue-400">
                  <Loader2 className="w-3 h-3 animate-spin" /> Analyzing...
                </span>
              ) : (
                <>
                  <button type="button" onClick={() => handleAiAnalyze(false)}
                    className="flex items-center gap-1 px-2 py-1 rounded-md text-[9px] font-medium bg-blue-500/10 text-blue-400 hover:bg-blue-500/20 transition-colors">
                    <Zap className="w-2.5 h-2.5" /> AI Analyze All
                  </button>
                  {results.some((r) => !r.aiVerdict) && (
                    <button type="button" onClick={() => handleAiAnalyze(true)}
                      className="flex items-center gap-1 px-2 py-1 rounded-md text-[9px] font-medium bg-blue-500/10 text-blue-300 hover:bg-blue-500/20 transition-colors">
                      <Zap className="w-2.5 h-2.5" /> Analyze New
                    </button>
                  )}
                </>
              )}
              <button type="button" onClick={handleClear}
                className="flex items-center gap-1 px-2 py-1 rounded-md text-[9px] text-muted-foreground/30 hover:text-destructive transition-colors">
                <Trash2 className="w-2.5 h-2.5" /> Clear
              </button>
            </>
          )}
        </div>

        {running && progress && (
          <div className="space-y-1">
            <div className="flex items-center gap-2 text-[9px] text-muted-foreground/40">
              <Loader2 className="w-3 h-3 animate-spin text-amber-400" />
              <span>{progress.completed}/{progress.total}</span>
              <span>{pct}%</span>
              <span className="text-amber-300">{progress.hits} hits</span>
              <span className="text-muted-foreground/20">{progress.dirsFound} dirs</span>
            </div>
            <div className="h-1 rounded-full bg-muted/20 overflow-hidden">
              <div className="h-full rounded-full bg-amber-500 transition-all duration-300" style={{ width: `${pct}%` }} />
            </div>
            {progress.currentUrl && (
              <div className="text-[8px] text-muted-foreground/20 font-mono truncate">{progress.currentUrl}</div>
            )}
          </div>
        )}
      </div>

      {/* Results */}
      <div className="flex-1 overflow-y-auto">
        {results.length === 0 && !running ? (
          <div className="h-full flex flex-col items-center justify-center gap-2 text-muted-foreground/20">
            <FileSearch className="w-10 h-10" />
            <p className="text-[11px]">No results yet</p>
            <p className="text-[9px] text-muted-foreground/15 max-w-[260px] text-center">
              Probes each directory level from sitemap for sensitive files (.env, .git, backups, configs, etc.)
            </p>
          </div>
        ) : (
          <div>
            {results.length > 0 && (
              <div className="flex items-center gap-2 px-4 py-1.5 border-b border-border/5 text-[9px] text-muted-foreground/30">
                <button type="button" onClick={selectAll} className="hover:text-foreground/60 transition-colors">
                  {selectedIds.size === results.length ? "Deselect all" : "Select all"}
                </button>
                {selectedIds.size > 0 && (
                  <>
                    <span>{selectedIds.size} selected</span>
                    <button type="button" onClick={handleConfirm}
                      className="flex items-center gap-1 px-2 py-0.5 rounded bg-emerald-500/10 text-emerald-400 hover:bg-emerald-500/20 transition-colors">
                      <Check className="w-2.5 h-2.5" /> Confirm
                    </button>
                  </>
                )}
              </div>
            )}

            <div className="divide-y divide-border/5">
              {results.map((r) => {
                const statusColor = r.statusCode >= 200 && r.statusCode < 300 ? "text-emerald-400"
                  : r.statusCode >= 300 && r.statusCode < 400 ? "text-yellow-400"
                  : "text-red-400";
                return (
                  <div
                    key={r.id}
                    className={cn("flex items-center gap-2 px-4 py-2 hover:bg-white/[0.02] transition-colors cursor-pointer",
                      selectedIds.has(r.id) && "bg-accent/5")}
                    onClick={() => toggleSelect(r.id)}
                  >
                    <input type="checkbox" checked={selectedIds.has(r.id)} readOnly
                      className="w-3 h-3 rounded accent-accent flex-shrink-0" />
                    <span className={cn("text-[10px] font-mono w-8 text-center", statusColor)}>{r.statusCode}</span>
                    <div className="flex-1 min-w-0">
                      <div className="text-[10px] font-mono text-foreground/70 truncate">{r.probePath}</div>
                      <div className="text-[8px] font-mono text-muted-foreground/25 truncate">{r.baseUrl}</div>
                    </div>
                    <span className="text-[8px] text-muted-foreground/20">{r.contentLength > 0 ? `${r.contentLength}b` : ""}</span>
                    {r.isConfirmed && (
                      <span className="text-[8px] px-1.5 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400">confirmed</span>
                    )}
                    {r.aiVerdict && (
                      <span className={cn("text-[8px] px-1.5 py-0.5 rounded-full",
                        r.aiVerdict === "true_positive" ? "bg-red-500/15 text-red-400"
                        : r.aiVerdict === "false_positive" ? "bg-zinc-500/10 text-zinc-400"
                        : "bg-yellow-500/10 text-yellow-400"
                      )}>{r.aiVerdict.replace("_", " ")}</span>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

const DEFAULT_SENSITIVE_PATH_COUNT = 60;

