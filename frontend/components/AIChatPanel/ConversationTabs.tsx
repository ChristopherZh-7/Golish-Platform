import { Clock, Plus, X } from "lucide-react";
import { memo } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import type { ChatConversation } from "@/store/slices/conversation";
import { useChatTabsScrollbar } from "./useChatTabsScrollbar";

/**
 * Top-of-panel conversation tab strip.
 *
 * Owns its own horizontal-scrollbar state via [`useChatTabsScrollbar`] and
 * exposes only the high-level callbacks the parent panel needs to wire
 * (select / close / new / toggle history).  Splitting this out lets
 * `AIChatPanel` stay focused on AI-session orchestration.
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
  const { tabsRef, tabsHovered, setTabsHovered, scrollThumb, handleThumbDragStart } =
    useChatTabsScrollbar(conversations.length);

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
          {conversations.map((conv) => (
            <button
              key={conv.id}
              type="button"
              data-conv-id={conv.id}
              className={cn(
                "group flex items-center gap-1.5 h-[28px] px-3 text-[12px] whitespace-nowrap flex-shrink-0 transition-all rounded-lg",
                conv.id === activeConvId
                  ? "text-foreground bg-[var(--bg-hover)]"
                  : "text-muted-foreground hover:text-foreground/80",
              )}
              onClick={() => onSelect(conv.id)}
            >
              {conv.id === activeConvId && (
                <div className="w-1.5 h-1.5 rounded-full bg-accent/50 flex-shrink-0" />
              )}
              <span className="max-w-[120px] truncate">{conv.title}</span>
              <span
                className={cn(
                  "w-4 h-4 flex items-center justify-center rounded-full transition-opacity",
                  conv.id === activeConvId
                    ? "opacity-60 hover:opacity-100"
                    : "opacity-0 group-hover:opacity-60 hover:!opacity-100",
                )}
                onClick={(e) => onClose(conv.id, e)}
                onKeyDown={() => {}}
                role="button"
                tabIndex={-1}
              >
                <X className="w-2.5 h-2.5" />
              </span>
            </button>
          ))}
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
                : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]",
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
            {/* biome-ignore lint/a11y/noStaticElementInteractions: scrollbar thumb is drag-only */}
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
