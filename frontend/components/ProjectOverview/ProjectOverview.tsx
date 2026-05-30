/* ── ProjectOverview ─────────────────────────────────────────────────
 *
 * Thin shell that composes the per-project recon overview tab. The bulk of
 * the logic lives in supporting modules:
 *
 *   - ./types                — shared types & constants
 *   - ./utils                — pure formatters + icon factory
 *   - ./PipelineProgressBar  — top progress strip
 *   - ./ItemRow / ./StepRow  — feed row primitives
 *   - ./hooks/useReconFeed   — Tauri ai-event subscription + feed state
 *
 * This file is intentionally small (≪ 800 lines) so the file-size budget
 * enforced by `scripts/check_file_sizes.sh` keeps shrinking.
 */

import { AlertTriangle, Loader2, Radar, RefreshCw, Target, Terminal } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { triggerAutoRecon } from "@/lib/ai";
import { ApiError, targets as targetsApi } from "@/lib/api";
import { translateErrorCode } from "@/lib/api/error-codes";
import { onCustomEvent } from "@/lib/events";
import { logger } from "@/lib/logger";
import type { Target as PentestTarget } from "@/lib/pentest/types";
import { getProjectPath } from "@/lib/projects";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import { useReconFeed } from "./hooks/useReconFeed";
import { ItemRow } from "./ItemRow";
import { PipelineProgressBar } from "./PipelineProgressBar";
import { StepRow } from "./StepRow";

export function ProjectOverview({ sessionId }: { sessionId: string }) {
  const projectName = useStore((s) => s.currentProjectName);
  const projectPath = useStore((s) => s.currentProjectPath);
  const [targets, setTargets] = useState<PentestTarget[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reconRunning, setReconRunning] = useState(false);
  const feedEndRef = useRef<HTMLDivElement>(null);

  const fetchTargets = useCallback(async () => {
    try {
      const pp = getProjectPath();
      const data = await targetsApi.listTargets(pp);
      setTargets(data.targets);
      setError(null);
    } catch (e) {
      logger.error("[ProjectOverview] fetchTargets failed:", e);
      setError(
        translateErrorCode(
          e instanceof ApiError ? e.code : "UNKNOWN",
          e instanceof Error ? e.message : undefined
        )
      );
      setTargets([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const { feed, progress, pipelineActive } = useReconFeed(sessionId, fetchTargets);

  const handleStartRecon = useCallback(async () => {
    if (!projectName || reconRunning) return;
    const targetValues = targets.map((t) => t.value);
    if (targetValues.length === 0) return;
    setReconRunning(true);
    try {
      await triggerAutoRecon(sessionId, targetValues, projectName, projectPath ?? "");
    } catch (e) {
      logger.error("Failed to run recon:", e);
    } finally {
      setReconRunning(false);
    }
  }, [sessionId, targets, projectName, projectPath, reconRunning]);

  useEffect(() => {
    feedEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, []);

  useEffect(() => {
    let cancelled = false;
    fetchTargets();
    let unlisten: (() => void) | undefined;
    (async () => {
      const u = await onCustomEvent("targets-changed", () => {
        if (!cancelled) fetchTargets();
      });
      if (cancelled) {
        u();
        return;
      }
      unlisten = u;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [fetchTargets]);

  if (!projectName) return null;

  const hasTargets = targets.length > 0;
  const canScan = hasTargets && !pipelineActive && !reconRunning;
  const hasInterruptedScan = targets.some((t) => t.status === "recon") && !pipelineActive;
  const hasFeed = feed.length > 0;

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex-shrink-0 flex items-center justify-between px-4 py-2.5 border-b border-border/10">
        <div className="flex items-center gap-3 min-w-0">
          <Target className="w-4 h-4 text-accent/60 flex-shrink-0" />
          <h2 className="text-sm font-semibold text-foreground/90 truncate">{projectName}</h2>
          {hasTargets && (
            <span className="text-[10px] text-muted-foreground/30">
              {targets.length} target{targets.length !== 1 ? "s" : ""}
            </span>
          )}
        </div>
        {canScan && (
          <button
            type="button"
            onClick={handleStartRecon}
            className={cn(
              "flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] font-medium transition-colors",
              hasInterruptedScan
                ? "bg-yellow-500/10 text-yellow-300 hover:bg-yellow-500/20"
                : "bg-accent/10 text-accent hover:bg-accent/20"
            )}
          >
            {hasInterruptedScan ? (
              <>
                <RefreshCw className="w-3 h-3" /> Restart
              </>
            ) : (
              <>
                <Radar className="w-3 h-3" /> Start Recon
              </>
            )}
          </button>
        )}
      </div>

      {/* Pipeline progress */}
      {progress && <PipelineProgressBar progress={progress} />}

      {/* Feed */}
      <div className="flex-1 min-h-0 overflow-auto">
        {hasFeed ? (
          <div className="divide-y divide-border/5">
            {feed.map((entry) => {
              if (entry.type === "step") {
                let dynDesc: string | undefined;
                if (entry.data.stepName === "tool_install" && entry.data.status === "running") {
                  const tcStep = feed.find(
                    (e) => e.type === "step" && e.data.stepName === "tool_check" && e.data.output
                  );
                  if (tcStep && tcStep.type === "step") {
                    const out = tcStep.data.output || "";
                    const allTools = ["nmap", "whatweb", "subfinder", "httpx"];
                    const match = out.match(/Tools available:\s*(.*)/i);
                    if (match) {
                      const available = match[1]
                        .split(",")
                        .map((s: string) => s.trim().toLowerCase());
                      const missing = allTools.filter((t) => !available.includes(t));
                      if (missing.length > 0) {
                        dynDesc = `Installing: ${missing.join(", ")}`;
                      }
                    }
                  }
                }
                return (
                  <StepRow
                    key={entry.data.id}
                    step={entry.data}
                    defaultOpen={entry.data.status === "running"}
                    dynamicDesc={dynDesc}
                  />
                );
              }
              return <ItemRow key={entry.data.id} item={entry.data} />;
            })}
            <div ref={feedEndRef} />
          </div>
        ) : (
          <div className="h-full flex flex-col items-center justify-center gap-3 px-4">
            {error ? (
              <>
                <AlertTriangle className="w-6 h-6 text-red-400/70" />
                <p className="text-xs text-red-400/70 text-center">{error}</p>
              </>
            ) : loading ? (
              <Loader2 className="w-6 h-6 text-muted-foreground/20 animate-spin" />
            ) : hasTargets ? (
              <>
                <Radar className="w-8 h-8 text-muted-foreground/15" />
                <p className="text-xs text-muted-foreground/30 text-center">
                  Ready to scan. Click <span className="text-accent/60">Start Recon</span> or send a
                  message to the AI.
                </p>
              </>
            ) : (
              <>
                <Terminal className="w-8 h-8 text-muted-foreground/15" />
                <p className="text-xs text-muted-foreground/30 text-center">
                  Waiting for activity...
                </p>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
