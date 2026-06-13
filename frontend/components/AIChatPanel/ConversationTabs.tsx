import { Clock, Plus, Wrench, X } from "lucide-react";
import { memo, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import type { ChatConversation } from "@/store/slices/conversation";
import { useChatTabsScrollbar } from "./useChatTabsScrollbar";

/**
 * Top-of-panel conversation tab strip.
 *
 * Owns its own horizontal-scrollbar state via [`useChatTabsScrollbar`] and
 * exposes only the high-level callbacks the parent panel needs to wire
 * (select / close / new / toggle history).  Splitting this out lets
 * `AIChatPanel` stay focused on AI-session orchestration.
 *
 * Engagement worker sessions (设计 2026-06-13-engagement-scoping-fanout §5④)
 * get a live status dot, and when many workers are open the INACTIVE ones
 * collapse into a single "⚒ N" chip (drill-in happens from the overview, so
 * 100 workers never explode the tab strip).
 */
export interface ConversationTabsProps {
  conversations: ChatConversation[];
  activeConvId: string | null;
  showHistory: boolean;
  onSelect: (convId: string) => void;
  onClose: (convId: string, e: React.MouseEvent) => void;
  onNewChat: () => void;
  onToggleHistory: () => void;
}

/** Collapse inactive worker tabs once more than this many workers exist. */
const WORKER_COLLAPSE_THRESHOLD = 6;

/** Pool status → tab dot class (mirrors the overview badge palette). */
function workerDotClass(status: string | null): string | null {
  switch (status) {
    case "running":
      return "bg-sky-400 animate-pulse";
    case "queued":
      return "bg-indigo-400";
    case "passed":
    case "skipped":
      return "bg-emerald-400";
    case "blocked":
      return "bg-amber-400";
    case "failed":
      return "bg-red-400";
    default:
      return null;
  }
}

export const ConversationTabs = memo(function ConversationTabs({
  conversations,
  activeConvId,
  showHistory,
  onSelect,
  onClose,
  onNewChat,
  onToggleHistory,
}: ConversationTabsProps) {
  const { t } = useTranslation();
  const pool = useStore((s) => s.engagementPool);

  /** Live pool status for a worker conversation (null for non-workers). */
  const workerStatus = (conv: ChatConversation): string | null => {
    const unitId = conv.workerMeta?.unitId;
    if (!unitId) return null;
    if (pool.running[unitId]) return "running";
    if (pool.queue.some((u) => u.id === unitId)) return "queued";
    return pool.outcomes[unitId]?.status ?? null;
  };

  // Collapse inactive workers into one chip when the strip would explode.
  const { visible, collapsedWorkers } = useMemo(() => {
    const workers = conversations.filter((c) => c.engagementRole === "worker");
    if (workers.length <= WORKER_COLLAPSE_THRESHOLD) {
      return { visible: conversations, collapsedWorkers: [] as ChatConversation[] };
    }
    const collapsed = workers.filter((c) => c.id !== activeConvId);
    const visibleList = conversations.filter(
      (c) => c.engagementRole !== "worker" || c.id === activeConvId
    );
    return { visible: visibleList, collapsedWorkers: collapsed };
  }, [conversations, activeConvId]);

  const overviewConv = useMemo(
    () => conversations.find((c) => c.engagementRole === "overview") ?? null,
    [conversations]
  );

  const { tabsRef, tabsHovered, setTabsHovered, scrollThumb, handleThumbDragStart } =
    useChatTabsScrollbar(visible.length, activeConvId);

  return (
    <div
      className="relative flex flex-col flex-shrink-0"
      onMouseEnter={() => setTabsHovered(true)}
      onMouseLeave={() => setTabsHovered(false)}
    >
      <div className="h-[37px] flex items-center px-2 gap-1.5">
        <div
          ref={tabsRef}
          className="flex-1 flex items-center gap-1.5 overflow-x-auto scrollbar-none min-w-0"
        >
          {visible.map((conv) => {
            const dot = workerDotClass(workerStatus(conv));
            return (
              <button
                key={conv.id}
                type="button"
                data-conv-id={conv.id}
                className={cn(
                  "group flex items-center gap-1.5 h-[28px] px-3 text-[12px] whitespace-nowrap flex-shrink-0 transition-all rounded-lg",
                  conv.id === activeConvId
                    ? "text-foreground bg-[var(--bg-hover)]"
                    : "text-muted-foreground hover:text-foreground/80"
                )}
                onClick={() => onSelect(conv.id)}
              >
                {dot ? (
                  <div className={cn("w-1.5 h-1.5 rounded-full flex-shrink-0", dot)} />
                ) : (
                  conv.id === activeConvId && (
                    <div className="w-1.5 h-1.5 rounded-full bg-accent/50 flex-shrink-0" />
                  )
                )}
                <span className="max-w-[120px] truncate">{conv.title}</span>
                <span
                  className={cn(
                    "w-4 h-4 flex items-center justify-center rounded-full transition-opacity",
                    conv.id === activeConvId
                      ? "opacity-60 hover:opacity-100"
                      : "opacity-0 group-hover:opacity-60 hover:!opacity-100"
                  )}
                  onClick={(e) => onClose(conv.id, e)}
                  onKeyDown={() => {}}
                  role="button"
                  tabIndex={-1}
                >
                  <X className="w-2.5 h-2.5" />
                </span>
              </button>
            );
          })}
          {collapsedWorkers.length > 0 && (
            <button
              type="button"
              title="Worker sessions are folded — open the engagement overview to drill in"
              className="flex items-center gap-1 h-[28px] px-2.5 text-[12px] whitespace-nowrap flex-shrink-0 rounded-lg text-muted-foreground hover:text-foreground/80 border border-dashed border-border"
              onClick={() => {
                if (overviewConv) onSelect(overviewConv.id);
                else if (collapsedWorkers[0]) onSelect(collapsedWorkers[0].id);
              }}
            >
              <Wrench className="w-3 h-3" />
              {collapsedWorkers.length} workers
            </button>
          )}
        </div>
        <div className="flex items-center gap-0.5 flex-shrink-0">
          <button
            type="button"
            title={t("ai.newChat")}
            className="h-6 w-6 flex items-center justify-center rounded-md text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
            onClick={onNewChat}
          >
            <Plus className="w-3.5 h-3.5" />
          </button>
          <button
            type="button"
            title={t("ai.history")}
            className={cn(
              "h-6 w-6 flex items-center justify-center rounded-md transition-colors",
              showHistory
                ? "text-foreground bg-[var(--bg-hover)]"
                : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]"
            )}
            onClick={onToggleHistory}
          >
            <Clock className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
      {/* Custom scrollbar track */}
      {tabsHovered && scrollThumb.visible && (
        <div className="h-[3px] mx-2">
          <div className="relative h-full w-full">
            <div
              className="absolute h-full rounded-full bg-foreground/20 hover:bg-foreground/35 cursor-pointer"
              style={{ left: `${scrollThumb.left}%`, width: `${scrollThumb.width}%` }}
              onMouseDown={handleThumbDragStart}
            />
          </div>
        </div>
      )}
    </div>
  );
});
