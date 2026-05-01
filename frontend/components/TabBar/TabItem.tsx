import {
  ArrowLeft,
  ArrowRight,
  Bot,
  Columns,
  Copy,
  ExternalLink,
  Globe,
  Home,
  Loader2,
  PanelLeft,
  Settings,
  Shield,
  Terminal,
  X,
} from "lucide-react";
import React from "react";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import type { TabItemState } from "@/store/selectors/tab-bar";

export interface TabItemProps {
  tab: TabItemState;
  isActive: boolean;
  isBusy: boolean;
  onClose: (e: React.MouseEvent) => void;
  onDuplicateTab: (workingDirectory: string) => Promise<unknown> | undefined;
  canClose: boolean;
  canMoveLeft: boolean;
  canMoveRight: boolean;
  onMoveLeft: () => void;
  onMoveRight: () => void;
  onConvertToPane: () => void;
  tabNumber?: number;
  showTabNumber?: boolean;
  hasNewActivity: boolean;
  isBeingDragged: boolean;
  dropSide: "left" | "right" | null;
  onTabPointerDown: (e: React.PointerEvent) => void;
  tabRef: (el: HTMLDivElement | null) => void;
}

export const TabItem = React.memo(function TabItem({
  tab,
  isActive,
  isBusy,
  onClose,
  onDuplicateTab,
  canClose,
  canMoveLeft,
  canMoveRight,
  onMoveLeft,
  onMoveRight,
  onConvertToPane,
  tabNumber,
  showTabNumber,
  hasNewActivity,
  isBeingDragged,
  dropSide,
  onTabPointerDown,
  tabRef,
}: TabItemProps) {
  const [isEditing, setIsEditing] = React.useState(false);
  const [editValue, setEditValue] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);

  const tabType = tab.tabType;

  const { displayName, dirName, isCustomName, isProcessName } = React.useMemo(() => {
    if (tabType === "home") {
      return { displayName: "", dirName: "", isCustomName: false, isProcessName: false };
    }

    if (tabType === "settings") {
      const name = tab.customName || tab.name || "Settings";
      return {
        displayName: name,
        dirName: tab.name || "Settings",
        isCustomName: !!tab.customName,
        isProcessName: false,
      };
    }

    if (tabType === "browser") {
      return {
        displayName: tab.customName || "Browser",
        dirName: "Browser",
        isCustomName: !!tab.customName,
        isProcessName: false,
      };
    }

    if (tabType === "security") {
      return {
        displayName: tab.customName || "Security",
        dirName: "Security",
        isCustomName: !!tab.customName,
        isProcessName: false,
      };
    }

    const dir = tab.workingDirectory.split(/[/\\]/).pop() || "Terminal";
    const name = tab.customName || tab.processName || dir;
    return {
      displayName: name,
      dirName: dir,
      isCustomName: !!tab.customName,
      isProcessName: !tab.customName && !!tab.processName,
    };
  }, [tab.customName, tab.name, tab.processName, tab.workingDirectory, tabType]);

  React.useEffect(() => {
    if (isEditing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [isEditing]);

  const handleDoubleClick = React.useCallback(
    (e: React.MouseEvent) => {
      if (tabType !== "terminal") return;
      e.preventDefault();
      e.stopPropagation();
      setIsEditing(true);
      setEditValue(tab.customName || dirName);
    },
    [tab.customName, dirName, tabType]
  );

  const handleSave = React.useCallback(() => {
    const trimmed = editValue.trim();
    useStore.getState().setCustomTabName(tab.id, trimmed || null);
    setIsEditing(false);
  }, [editValue, tab.id]);

  const handleKeyDown = React.useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleSave();
      } else if (e.key === "Escape") {
        e.preventDefault();
        setIsEditing(false);
      }
    },
    [handleSave]
  );

  const getTabIcon = () => {
    switch (tabType) {
      case "home":
        return Home;
      case "settings":
        return Settings;
      case "browser":
        return Globe;
      case "security":
        return Shield;
      default:
        return tab.mode === "agent" ? Bot : Terminal;
    }
  };
  const ModeIcon = getTabIcon();

  const tooltipText = React.useMemo(() => {
    if (tabType === "home") return "Home";
    if (tabType === "settings") return displayName;
    if (isCustomName) return `Custom name: ${displayName}\nDirectory: ${tab.workingDirectory}`;
    if (isProcessName) return `Running: ${displayName}\nDirectory: ${tab.workingDirectory}`;
    return tab.workingDirectory;
  }, [isCustomName, isProcessName, displayName, tab.workingDirectory, tabType]);

  return (
    <div
      ref={tabRef}
      className={cn(
        "relative",
        isBeingDragged &&
          "opacity-60 ring-2 ring-accent bg-accent/10 rounded-md scale-[0.92] transition-all duration-150"
      )}
      onPointerDown={onTabPointerDown}
    >
      {dropSide === "left" && (
        <div className="absolute left-0 top-1 bottom-1 w-0.5 bg-accent rounded-full z-20" />
      )}
      {dropSide === "right" && (
        <div className="absolute right-0 top-1 bottom-1 w-0.5 bg-accent rounded-full z-20" />
      )}
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div className="group relative flex items-center">
            <Tooltip>
              <TooltipTrigger asChild>
                <TabsTrigger
                  value={tab.id}
                  className={cn(
                    "relative flex items-center gap-2 px-3 py-1 rounded-t-md min-w-0 max-w-[200px] text-[11px]",
                    tabType === "terminal" && "font-mono",
                    "data-[state=active]:bg-muted data-[state=active]:text-foreground data-[state=active]:shadow-none",
                    "data-[state=inactive]:text-muted-foreground data-[state=inactive]:hover:bg-[var(--bg-hover)] data-[state=inactive]:hover:text-foreground",
                    "border-none focus-visible:ring-0 focus-visible:ring-offset-0 transition-colors",
                    canClose && "pr-7"
                  )}
                >
                  {isActive && <span className="absolute bottom-0 left-0 right-0 h-px bg-accent" />}

                  {isBusy && (
                    <Loader2
                      className={cn(
                        "size-icon-tab-bar flex-shrink-0 animate-spin",
                        isActive ? "text-accent" : "text-muted-foreground"
                      )}
                    />
                  )}

                  {hasNewActivity && !isBusy && (
                    <span
                      aria-hidden="true"
                      className="activity-dot w-1.5 h-1.5 flex-shrink-0 rounded-full bg-[var(--ansi-yellow)]"
                    />
                  )}

                  {tabType !== "terminal" && !isBusy && (
                    <ModeIcon
                      className={cn(
                        "size-icon-tab-bar flex-shrink-0",
                        isActive ? "text-accent" : "text-muted-foreground"
                      )}
                    />
                  )}

                  {tabType !== "home" &&
                    (isEditing ? (
                      <input
                        ref={inputRef}
                        type="text"
                        value={editValue}
                        onChange={(e) => setEditValue(e.target.value)}
                        onBlur={handleSave}
                        onKeyDown={handleKeyDown}
                        onClick={(e) => e.stopPropagation()}
                        className={cn(
                          "truncate text-[11px] bg-transparent border-none outline-none",
                          tabType === "terminal" && "font-mono",
                          "focus:ring-1 focus:ring-accent rounded px-1 min-w-[60px] max-w-[140px]"
                        )}
                      />
                    ) : (
                      /* biome-ignore lint/a11y/noStaticElementInteractions: span is used for inline text with double-click rename */
                      <span
                        className={cn(
                          "truncate",
                          tabType === "terminal" && "cursor-text",
                          isProcessName && !hasNewActivity && "text-accent",
                          hasNewActivity && "text-[var(--ansi-yellow)]"
                        )}
                        onDoubleClick={handleDoubleClick}
                      >
                        {displayName}
                      </span>
                    ))}

                  {showTabNumber && tabNumber !== undefined && (
                    <span className="flex-shrink-0 ml-1 px-1 min-w-[14px] h-[14px] flex items-center justify-center bg-accent text-accent-foreground text-[9px] font-semibold rounded">
                      {tabNumber}
                    </span>
                  )}
                </TabsTrigger>
              </TooltipTrigger>
              <TooltipContent side="bottom" className="whitespace-pre-wrap">
                <p className="text-xs">{tooltipText}</p>
              </TooltipContent>
            </Tooltip>

            {canClose && (
              <button
                type="button"
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  onClose(e);
                }}
                onMouseDown={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                }}
                className={cn(
                  "absolute right-1 p-0.5 rounded opacity-0 group-hover:opacity-100 transition-opacity",
                  "hover:bg-destructive/20 text-muted-foreground hover:text-destructive",
                  "z-10"
                )}
                title="Close tab"
              >
                <X className="w-3 h-3" />
              </button>
            )}
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuItem onClick={onMoveLeft} disabled={!canMoveLeft}>
            <ArrowLeft className="size-icon-tab-bar" />
            Move Left
          </ContextMenuItem>
          <ContextMenuItem onClick={onMoveRight} disabled={!canMoveRight}>
            <ArrowRight className="size-icon-tab-bar" />
            Move Right
          </ContextMenuItem>
          <ContextMenuSeparator />
          {tabType === "terminal" && (
            <ContextMenuItem
              onClick={() =>
                window.dispatchEvent(new CustomEvent("split-tab-right", { detail: tab.id }))
              }
            >
              <Columns className="size-icon-tab-bar" />
              Split to Right
            </ContextMenuItem>
          )}
          {tabType === "terminal" && (
            <ContextMenuItem onClick={onConvertToPane}>
              <PanelLeft className="size-icon-tab-bar" />
              Convert to Pane
            </ContextMenuItem>
          )}
          {(tabType === "terminal" || tabType === "security") && (
            <ContextMenuItem
              onClick={() => {
                window.dispatchEvent(
                  new CustomEvent("detach-tab", {
                    detail: {
                      tabId: tab.id,
                      screenX: window.screenX + 100,
                      screenY: window.screenY + 100,
                    },
                  })
                );
              }}
            >
              <ExternalLink className="size-icon-tab-bar" />
              Detach to Window
            </ContextMenuItem>
          )}
          {tabType === "terminal" && (
            <ContextMenuItem onClick={() => onDuplicateTab(tab.workingDirectory)}>
              <Copy className="size-icon-tab-bar" />
              Duplicate Tab
            </ContextMenuItem>
          )}
          {tabType === "terminal" && canClose && <ContextMenuSeparator />}
          {canClose && (
            <ContextMenuItem variant="destructive" onClick={(e) => onClose(e)}>
              <X className="size-icon-tab-bar" />
              Close Tab
            </ContextMenuItem>
          )}
        </ContextMenuContent>
      </ContextMenu>
    </div>
  );
});
