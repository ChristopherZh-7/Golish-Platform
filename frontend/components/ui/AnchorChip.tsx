/**
 * AnchorChip
 *
 * 在 ChatPanel 的小卡和 DetailView 的大卡上同时显示同一个锚点编号（T1 / A1 / P1），
 * 让用户左右两侧能一眼对应起来。锚点本身由 `useAnchorMap` selector 计算。
 *
 * 用法：
 *   <AnchorChip anchor="T3" />
 *   <AnchorChip sessionId={sid} requestId={tc.requestId} />  // 自动查锚点
 */
import { memo } from "react";
import { cn } from "@/lib/utils";
import { useAnchorFor } from "@/store/selectors";

export type AnchorKind = "tool" | "agent" | "pipeline";

interface AnchorChipProps {
  /** 直接传锚点字符串，例如 "T3" / "A1" / "P2" */
  anchor?: string | null;
  /** 自动查询模式：传 sessionId + requestId 让 chip 自己从 store 拿锚点 */
  sessionId?: string | null;
  requestId?: string | null;
  /** 占位空间用 —— 没拿到锚点时仍然占同样宽度，避免行内布局抖动 */
  reserveSpace?: boolean;
  className?: string;
}

function classifyAnchor(anchor: string | null | undefined): AnchorKind {
  if (!anchor) return "tool";
  if (anchor.startsWith("A")) return "agent";
  if (anchor.startsWith("P")) return "pipeline";
  return "tool";
}

const KIND_CLASS: Record<AnchorKind, string> = {
  agent: "bg-[var(--accent-dim)] text-accent/90",
  pipeline: "bg-[var(--ansi-magenta)]/15 text-[var(--ansi-magenta)]/85",
  tool: "bg-muted/40 text-muted-foreground/80",
};

export const AnchorChip = memo(function AnchorChip({
  anchor,
  sessionId,
  requestId,
  reserveSpace,
  className,
}: AnchorChipProps) {
  // 自动查询模式（hook 在两种模式下都会调用，但传 null/undefined 时 selector 直接返回 null）
  const lookedUp = useAnchorFor(
    anchor ? null : sessionId ?? null,
    anchor ? null : requestId ?? null,
  );
  const value = anchor ?? lookedUp;

  if (!value) {
    if (!reserveSpace) return null;
    // 占位
    return <span aria-hidden className="inline-block w-[26px] flex-shrink-0" />;
  }

  const kind = classifyAnchor(value);
  return (
    <span
      className={cn(
        "text-[9.5px] font-mono font-medium tabular-nums px-1 py-px rounded flex-shrink-0",
        KIND_CLASS[kind],
        className,
      )}
      title={`Anchor ${value}`}
    >
      {value}
    </span>
  );
});
