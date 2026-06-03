import { memo, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

interface TaskPreparingIndicatorProps {
  /** Active execution-mode id (e.g. "assessment", "red_team", or "chat"). */
  modeId: string;
  /**
   * Epoch ms when the preparing phase began. Anchoring the elapsed counter to a
   * stable timestamp (sourced from the conversation) keeps it correct across
   * remounts — switching conversation tabs away and back no longer resets it to
   * 0. Falls back to mount time when omitted.
   */
  startedAt?: number;
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
  startedAt,
  className,
}: TaskPreparingIndicatorProps) {
  const { t } = useTranslation();
  // Initialize from the anchor (not 0) so a remount shows the real elapsed time
  // immediately, with no flash back to 0 when switching conversation tabs.
  const [seconds, setSeconds] = useState(() =>
    Math.max(0, Math.floor((Date.now() - (startedAt ?? Date.now())) / 1000))
  );

  useEffect(() => {
    const anchor = startedAt ?? Date.now();
    const update = () => setSeconds(Math.max(0, Math.floor((Date.now() - anchor) / 1000)));
    update();
    const id = setInterval(update, 1000);
    return () => clearInterval(id);
  }, [startedAt]);

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
          "agent-status-line inline-flex max-w-full items-center gap-2 select-none",
          className
        )}
        aria-live="polite"
        aria-busy
      >
        <span className="relative flex h-1.5 w-1.5 flex-shrink-0" aria-hidden="true">
          <span className="agent-status-dot absolute inline-flex h-full w-full rounded-full bg-accent/60" />
          <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-accent" />
        </span>
        <span className="agent-status-shimmer truncate text-[12px] font-medium">{text}</span>
      </div>
    </div>
  );
});
