import { memo, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

interface TaskPreparingIndicatorProps {
  /** Active execution-mode id (e.g. "assessment", "red_team", or "chat"). */
  modeId: string;
  className?: string;
}

/**
 * Title-case a profile id for display: "red_team" -> "Red Team". Matches the
 * backend `display_name` for every embedded profile, so we avoid an extra
 * `list_execution_modes` round-trip just to label the indicator.
 */
function formatModeLabel(modeId: string): string {
  return modeId
    .split("_")
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

/**
 * Panel-level "preparing" indicator shown after a send while the conversation
 * is streaming but no assistant bubble exists yet. In task/profile modes the
 * orchestrator runs the planning LLM before emitting the first `started`
 * event, which otherwise leaves a blank gap between the user's message and the
 * first plan. Surfacing the active mode plus a live elapsed-seconds counter
 * makes that wait read as "working", not "stuck".
 */
export const TaskPreparingIndicator = memo(function TaskPreparingIndicator({
  modeId,
  className,
}: TaskPreparingIndicatorProps) {
  const { t } = useTranslation();
  const [seconds, setSeconds] = useState(0);

  useEffect(() => {
    const startedAt = Date.now();
    const id = setInterval(() => {
      setSeconds(Math.floor((Date.now() - startedAt) / 1000));
    }, 1000);
    return () => clearInterval(id);
  }, []);

  const isTaskMode = modeId !== "chat";
  const text = useMemo(() => {
    // Manual `%MODE%`/`%SECONDS%` placeholders (not i18next `{{}}`) so the live
    // values are substituted in the component itself — keeps the indicator
    // correct whether or not the i18next instance is initialized.
    const template = isTaskMode
      ? t("ai.preparing.planning", {
          defaultValue: "%MODE% mode · Planning task… (%SECONDS%s)",
        })
      : t("ai.preparing.thinking", { defaultValue: "Preparing… (%SECONDS%s)" });
    return template
      .replace("%MODE%", formatModeLabel(modeId))
      .replace("%SECONDS%", String(seconds));
  }, [isTaskMode, modeId, seconds, t]);

  return (
    <div className="px-4 py-3">
      <div
        className={cn(
          "agent-status-line inline-flex max-w-full items-center gap-2 rounded-md",
          "border border-[var(--border-subtle)] bg-background/55 px-2.5 py-1",
          "text-[11.5px] text-muted-foreground select-none",
          className
        )}
        aria-live="polite"
        aria-busy
      >
        <span className="relative flex h-2 w-2 flex-shrink-0" aria-hidden="true">
          <span className="agent-status-dot absolute inline-flex h-full w-full rounded-full bg-accent/55" />
          <span className="relative inline-flex h-2 w-2 rounded-full bg-accent/80" />
        </span>
        <span className="truncate text-foreground/75">{text}</span>
      </div>
    </div>
  );
});
