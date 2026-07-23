import { Clock3, Loader2, OctagonX, Radio } from "lucide-react";
import { memo, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { cancelBackgroundJob } from "@/lib/ai";
import { formatDurationCompact } from "@/lib/time";
import { cn } from "@/lib/utils";
import { type BackgroundJob, type BackgroundRunMeta, useStore } from "@/store";

interface BackgroundJobPanelProps {
  sessionId: string;
  backgroundRun: BackgroundRunMeta;
  job?: BackgroundJob | null;
  terminalResult?: unknown;
  className?: string;
}

function terminalStatus(result: unknown): "completed" | "failed" | "stopped" | null {
  if (result == null || typeof result !== "object") return null;
  const status = (result as { status?: unknown }).status;
  if (status === "killed") return "stopped";
  if (status === "failed") return "failed";
  if (status === "done") return "completed";
  return null;
}

function terminalDuration(result: unknown): number | null {
  if (result == null || typeof result !== "object") return null;
  const duration = (result as { duration_ms?: unknown }).duration_ms;
  return typeof duration === "number" && Number.isFinite(duration) ? duration : null;
}

export const BackgroundJobPanel = memo(function BackgroundJobPanel({
  sessionId,
  backgroundRun,
  job: initialJob = null,
  terminalResult,
  className,
}: BackgroundJobPanelProps) {
  const { t } = useTranslation();
  const storeJob = useStore(
    (state) =>
      state.backgroundJobs[sessionId]?.find(
        (candidate) => candidate.jobId === backgroundRun.jobId
      ) ?? null
  );
  const job = storeJob ?? initialJob;
  const live = Boolean(storeJob);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [requestError, setRequestError] = useState(false);

  useEffect(() => {
    if (!live) return;
    setNowMs(Date.now());
    const interval = window.setInterval(() => setNowMs(Date.now()), 1_000);
    return () => window.clearInterval(interval);
  }, [live]);

  const requestStop = async () => {
    if (!job || job.state === "stopping") return;
    setRequestError(false);
    try {
      const accepted = await cancelBackgroundJob(job.jobId);
      if (accepted) {
        useStore.getState().setBackgroundJobState(sessionId, job.jobId, "stopping");
      } else {
        setRequestError(true);
      }
    } catch {
      setRequestError(true);
    }
  };

  const terminal = terminalStatus(terminalResult);
  const status = live
    ? job?.state === "stopping"
      ? "stopping"
      : "running"
    : (terminal ?? "completed");
  const statusLabel = t(`ai.backgroundJobs.status.${status}`);
  const elapsedBackgroundMs = Math.max(0, nowMs - backgroundRun.backgroundedAt);
  const lastOutputMs = job?.lastOutputAt != null ? Math.max(0, nowMs - job.lastOutputAt) : null;
  const finalDuration = terminalDuration(terminalResult);

  return (
    <section
      className={cn(
        "mx-4 my-3 overflow-hidden rounded-lg border border-amber-300/25 bg-amber-400/[0.07]",
        className
      )}
      aria-label={t("ai.backgroundJobs.panelLabel")}
    >
      <div className="flex items-center gap-2 border-b border-amber-300/15 px-3 py-2.5">
        {live ? (
          <Loader2 className="h-4 w-4 animate-spin text-amber-300" />
        ) : terminal === "stopped" ? (
          <OctagonX className="h-4 w-4 text-muted-foreground" />
        ) : (
          <Radio className="h-4 w-4 text-amber-300" />
        )}
        <div className="min-w-0 flex-1">
          <div className="text-xs font-medium text-foreground">{statusLabel}</div>
          <div className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground/70">
            {backgroundRun.jobId}
          </div>
        </div>
        {live && job && (
          <button
            type="button"
            onClick={requestStop}
            disabled={job.state === "stopping"}
            aria-label={t("ai.backgroundJobs.stop")}
            className="inline-flex h-7 items-center gap-1.5 rounded-md border border-destructive/25 px-2 text-[10px] text-destructive transition-colors hover:bg-destructive/10 disabled:cursor-wait disabled:opacity-60"
          >
            {job.state === "stopping" ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <OctagonX className="h-3 w-3" />
            )}
            {job.state === "stopping"
              ? t("ai.backgroundJobs.status.stopping")
              : t("ai.backgroundJobs.stopAction")}
          </button>
        )}
      </div>

      <div className="grid gap-2 px-3 py-2.5 text-[11px] text-muted-foreground sm:grid-cols-2">
        {live ? (
          <div className="flex items-center gap-1.5">
            <Clock3 className="h-3 w-3 text-amber-300/80" />
            {t("ai.backgroundJobs.backgroundFor", {
              duration: formatDurationCompact(elapsedBackgroundMs),
            })}
          </div>
        ) : finalDuration != null ? (
          <div className="flex items-center gap-1.5">
            <Clock3 className="h-3 w-3 text-amber-300/80" />
            {t("ai.backgroundJobs.totalDuration", {
              duration: formatDurationCompact(finalDuration),
            })}
          </div>
        ) : null}
        {live && <div>{t("ai.backgroundJobs.manualTermination")}</div>}
        {live && lastOutputMs != null && (
          <div>
            {t("ai.backgroundJobs.lastOutput", {
              duration: formatDurationCompact(lastOutputMs),
            })}
          </div>
        )}
      </div>

      {requestError && (
        <div className="border-t border-destructive/15 px-3 py-2 text-[10px] text-destructive">
          {t("ai.backgroundJobs.stopFailed")}
        </div>
      )}
    </section>
  );
});
