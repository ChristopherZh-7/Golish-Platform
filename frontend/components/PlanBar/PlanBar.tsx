/**
 * PlanBar
 *
 * 浮在面板顶部的"任务计划"概览条。ChatPanel 和 ToolDetailView 共用同一份组件，
 * 让用户在两边看见的是同一个 plan 视图。
 *
 * - 折叠态：一行进度条 + 版本号 + 完成进度
 * - 展开态：每个步骤一行（含步骤标号、状态图标、文字）
 *
 * 数据来源：useStore(s => s.sessions[sessionId]?.plan)
 *
 * 点击某步可以触发 onStepClick，宿主决定怎么"跳"过去（ChatPanel 滚动到对应工具，
 * DetailView 滚动到对应 PlanStepGroup）。如果不传 onStepClick，步骤行依然渲染但没点击效果。
 */
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  ClipboardList,
  Eye,
  Loader2,
  XCircle,
} from "lucide-react";
import { memo, useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";

interface PlanBarProps {
  sessionId: string | null | undefined;
  /** 默认展开 / 折叠（DetailView 默认展开，ChatPanel 默认折叠） */
  defaultOpen?: boolean;
  /**
   * 点击某一步（按 step.id 优先，没有则按 index 字符串）。
   * 宿主决定如何把"跳到那一步"映射到自己的滚动 / 高亮行为。
   */
  onStepClick?: (stepKey: string, stepIdx: number) => void;
  /**
   * 是否显示"查看"按钮 —— 点击后把中央面板切到 tool-detail 模式。
   * 默认为 true（ChatPanel 用），DetailView 自身已经在显示 tool-detail 所以传 false。
   */
  showViewButton?: boolean;
}

function StatusGlyph({ status }: { status: string }) {
  switch (status) {
    case "completed":
      return <CheckCircle2 className="w-3 h-3 text-green-500 flex-shrink-0" />;
    case "in_progress":
      return <Loader2 className="w-3 h-3 text-accent animate-spin flex-shrink-0" />;
    case "failed":
    case "cancelled":
      return <XCircle className="w-3 h-3 text-red-400/70 flex-shrink-0" />;
    default:
      return (
        <div className="w-3 h-3 rounded-full border-[1.5px] border-muted-foreground/25 flex-shrink-0" />
      );
  }
}

export const PlanBar = memo(function PlanBar({
  sessionId,
  defaultOpen = false,
  onStepClick,
  showViewButton = true,
}: PlanBarProps) {
  const { t } = useTranslation();
  // Plan 可能写在 ai-session 或者 conversation 关联的 terminal-session。
  // 这里先按 sessionId 取，没有则在与之关联的 conversation 的所有 sessions 中找一个有 plan 的回退。
  const { plan, planSessionId } = useStore(
    useShallow((s) => {
      const direct = sessionId ? s.sessions[sessionId]?.plan ?? null : null;
      if (direct) return { plan: direct, planSessionId: sessionId ?? null };
      if (sessionId) {
        for (const conv of Object.values(s.conversations)) {
          if (!conv) continue;
          const isMatch =
            conv.aiSessionId === sessionId ||
            (s.conversationTerminals[conv.id] ?? []).includes(sessionId);
          if (!isMatch) continue;
          const candidates = [conv.aiSessionId, ...(s.conversationTerminals[conv.id] ?? [])];
          for (const sid of candidates) {
            if (!sid) continue;
            const p = s.sessions[sid]?.plan;
            if (p) return { plan: p, planSessionId: sid };
          }
        }
      }
      return { plan: null as null, planSessionId: null as string | null };
    }),
  );
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);
  const [expanded, setExpanded] = useState(defaultOpen);
  const toggle = useCallback(() => setExpanded((v) => !v), []);

  const handleViewDetail = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      const target = planSessionId ?? sessionId;
      if (target) setDetailViewMode(target, "tool-detail");
    },
    [planSessionId, sessionId, setDetailViewMode],
  );

  if (!plan || plan.steps.length === 0) return null;

  const summary = plan.summary;
  const total = summary.total ?? plan.steps.length;
  const completed = summary.completed ?? 0;
  const progress = total > 0 ? (completed / total) * 100 : 0;
  const isDone = total > 0 && completed === total;

  return (
    <div className="border-b border-[var(--border-subtle)] bg-background/40 flex-shrink-0">
      <div className="w-full flex items-center gap-2 px-3 py-2 hover:bg-[var(--bg-hover)] transition-colors">
        <button
          type="button"
          onClick={toggle}
          className="flex flex-1 items-center gap-2 text-left min-w-0"
          title={expanded ? t("ai.planBar.collapse") : t("ai.planBar.expand")}
        >
          <ClipboardList className="w-3.5 h-3.5 text-accent flex-shrink-0" />
          <span className="text-[12px] font-semibold text-foreground">Plan</span>
          <span className="text-[9.5px] px-1.5 py-px rounded-full bg-[var(--accent-dim)] text-accent/80 font-medium tabular-nums flex-shrink-0">
            v{plan.version}
          </span>
          {isDone && (
            <span className="text-[9.5px] px-1.5 py-px rounded-full bg-green-500/10 text-green-500/80 font-medium flex-shrink-0">
              Done
            </span>
          )}
          <div className="flex-1 mx-2 h-1 rounded-full bg-muted/40 overflow-hidden min-w-[40px]">
            <div
              className="h-full bg-accent transition-all duration-300"
              style={{ width: `${progress}%` }}
            />
          </div>
          <span className="text-[10px] text-muted-foreground/70 tabular-nums flex-shrink-0">
            {completed}/{total}
          </span>
          {expanded ? (
            <ChevronDown className="w-3 h-3 text-muted-foreground/60 flex-shrink-0" />
          ) : (
            <ChevronRight className="w-3 h-3 text-muted-foreground/60 flex-shrink-0" />
          )}
        </button>
        {showViewButton && (
          <button
            type="button"
            onClick={handleViewDetail}
            title={t("ai.planBar.viewDetail")}
            className="flex items-center gap-1 text-[10px] text-muted-foreground/60 hover:text-accent transition-colors px-1.5 py-0.5 rounded hover:bg-accent/10 flex-shrink-0"
          >
            <Eye className="w-3 h-3" />
            <span>{t("ai.planBar.view")}</span>
          </button>
        )}
      </div>

      {expanded && (
        <div className="px-3 pb-2 space-y-0.5">
          {plan.steps.map((step, i) => {
            const stepKey = step.id ?? `idx-${i}`;
            const isClickable = !!onStepClick;
            const Tag = isClickable ? "button" : "div";
            return (
              <Tag
                key={stepKey}
                type={isClickable ? "button" : undefined}
                onClick={isClickable ? () => onStepClick?.(stepKey, i) : undefined}
                className={cn(
                  "w-full flex items-center gap-2 py-1 px-2 rounded text-[11px] text-left transition-colors",
                  isClickable && "hover:bg-accent/10 cursor-pointer",
                  step.status === "in_progress" && "bg-accent/[0.04]",
                )}
              >
                <StatusGlyph status={step.status} />
                <span
                  className={cn(
                    "flex-1 truncate",
                    step.status === "completed" && "text-muted-foreground/65",
                    step.status === "in_progress" && "text-accent",
                    step.status === "cancelled" && "text-red-400/80 line-through",
                    step.status === "failed" && "text-red-400/80",
                    step.status === "pending" && "text-muted-foreground/50",
                  )}
                  title={step.step}
                >
                  {step.step}
                </span>
                <span className="text-[9.5px] px-1 py-px rounded bg-muted/40 text-muted-foreground/65 tabular-nums flex-shrink-0">
                  Step {i + 1}
                </span>
              </Tag>
            );
          })}
        </div>
      )}
    </div>
  );
});
