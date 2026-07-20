import { AlertTriangle, Check, CircleDot, LoaderCircle, Lock, RotateCcw } from "lucide-react";
import { memo, useCallback, useMemo, useState } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { IN_PLACE_RESET_STAGES, KNOWN_HARNESS_STAGES } from "@/lib/stage-reset";
import { cn } from "@/lib/utils";
import { prettyStageName } from "./StageMarker";

export interface StageResetAvailability {
  selectable: boolean;
  reason: string | null;
}

/** Mirror the backend's fail-closed in-place reset policy for immediate UX. */
export function stageResetAvailability(
  stage: string,
  currentStage: string | null,
  passedStages: string[]
): StageResetAvailability {
  if (!currentStage || !KNOWN_HARNESS_STAGES.has(currentStage)) {
    return { selectable: false, reason: "没有可原地重置的运行中阶段" };
  }
  if (!IN_PLACE_RESET_STAGES.has(currentStage)) {
    return { selectable: false, reason: "当前阶段需要新建测试任务" };
  }
  if (!IN_PLACE_RESET_STAGES.has(stage)) {
    return { selectable: false, reason: "该阶段需要新建测试任务" };
  }
  if (stage !== currentStage && !passedStages.includes(stage)) {
    return { selectable: false, reason: "该阶段尚未运行" };
  }
  return { selectable: true, reason: null };
}

interface StageResetMenuProps {
  /** All stages in DAG run order. */
  stageOrder: string[];
  /** Stages that have already passed their gate. */
  passedStages: string[];
  /** The current frontier stage (first not-yet-passed), or null when none. */
  currentStage: string | null;
  /** Disable the whole control (no conversation / streaming / no stages). */
  disabled: boolean;
  /** A reset is in flight. */
  busy: boolean;
  /** Perform a full reset (purge + rewind) to `stage` and resume from it. */
  onReset: (stage: string) => void;
}

/**
 * Dev-only stage reset picker. Lists the DAG stages and lets the user fully
 * reset a supported Company stage at or before the real running frontier.
 * Selecting a stage asks for confirmation before firing `onReset`.
 */
export const StageResetMenu = memo(function StageResetMenu({
  stageOrder,
  passedStages,
  currentStage,
  disabled,
  busy,
  onReset,
}: StageResetMenuProps) {
  const [open, setOpen] = useState(false);
  const [pendingStage, setPendingStage] = useState<string | null>(null);

  const passed = useMemo(() => new Set(passedStages), [passedStages]);

  const handleOpenChange = useCallback((next: boolean) => {
    setOpen(next);
    if (!next) setPendingStage(null);
  }, []);

  const confirmReset = useCallback(() => {
    if (!pendingStage) return;
    onReset(pendingStage);
    setPendingStage(null);
    setOpen(false);
  }, [onReset, pendingStage]);

  return (
    <DropdownMenu modal={false} open={open} onOpenChange={handleOpenChange}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          title="重置阶段重新测试（只能往回跳）"
          aria-label="重置阶段"
          disabled={disabled}
          className="h-6 w-6 flex items-center justify-center rounded text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:cursor-not-allowed disabled:opacity-40"
        >
          {busy ? (
            <LoaderCircle className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <RotateCcw className="w-3.5 h-3.5" />
          )}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        side="top"
        className="bg-card border-[var(--border-medium)] min-w-[240px]"
      >
        {pendingStage ? (
          <div className="px-3 py-2">
            <div className="flex items-start gap-2 text-[12px] text-foreground">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-[#e0af68]" />
              <div>
                清空并重测「<span className="font-semibold">{prettyStageName(pendingStage)}</span>
                」及其之后的阶段？
                <div className="mt-1 text-[11px] text-muted-foreground">
                  会删除该阶段已发现的数据（资产 / JS / 端点等，不可逆），保留日志。
                </div>
              </div>
            </div>
            <div className="mt-2.5 flex items-center justify-end gap-1.5">
              <button
                type="button"
                onClick={() => setPendingStage(null)}
                className="rounded px-2 py-1 text-[11px] text-muted-foreground hover:bg-[var(--bg-hover)] transition-colors"
              >
                取消
              </button>
              <button
                type="button"
                onClick={confirmReset}
                className="rounded bg-destructive/20 px-2 py-1 text-[11px] font-medium text-destructive hover:bg-destructive/30 transition-colors"
              >
                确认清空
              </button>
            </div>
          </div>
        ) : (
          <>
            <div className="px-3 pt-2 pb-1 text-[10.5px] text-muted-foreground">
              原地重测 Company 阶段（其他阶段需新建测试任务）
            </div>
            {stageOrder.map((stage) => {
              const availability = stageResetAvailability(stage, currentStage, passedStages);
              const selectable = availability.selectable;
              const isPassed = passed.has(stage);
              const isCurrent = stage === currentStage;
              const Icon = !selectable ? Lock : isPassed ? Check : CircleDot;
              return (
                <DropdownMenuItem
                  key={stage}
                  disabled={!selectable}
                  title={availability.reason ?? undefined}
                  onSelect={(e) => {
                    e.preventDefault();
                    if (selectable) setPendingStage(stage);
                  }}
                  className={cn(
                    "text-xs flex items-center gap-2 py-2",
                    selectable
                      ? "cursor-pointer text-foreground hover:text-accent"
                      : "cursor-not-allowed text-muted-foreground/50"
                  )}
                >
                  <Icon
                    className={cn(
                      "w-3.5 h-3.5 shrink-0",
                      !selectable
                        ? "text-muted-foreground/40"
                        : isPassed
                          ? "text-[var(--ansi-green)]"
                          : "text-accent"
                    )}
                  />
                  <span className="font-medium">{prettyStageName(stage)}</span>
                  {isCurrent && (
                    <span className="ml-auto text-[10px] text-muted-foreground">当前</span>
                  )}
                  {!isCurrent && !selectable && availability.reason && (
                    <span className="ml-auto max-w-[112px] truncate text-[9px] text-muted-foreground/60">
                      {availability.reason}
                    </span>
                  )}
                </DropdownMenuItem>
              );
            })}
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
});
