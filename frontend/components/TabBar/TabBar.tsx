import { getCurrentWindow } from "@tauri-apps/api/window";
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
  Plus,
  Settings,
  Shield,
  Terminal,
  X,
} from "lucide-react";
import React from "react";
import { createPortal } from "react-dom";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useCreateTerminalTab } from "@/hooks/useCreateTerminalTab";
import { shutdownAiSession } from "@/lib/ai";
import { logger } from "@/lib/logger";
import { ptyDestroy } from "@/lib/tauri";
import { liveTerminalManager, TerminalInstanceManager } from "@/lib/terminal";
import { TerminalRecordingControls } from "@/components/Terminal/TerminalRecordingControls";
import { cn } from "@/lib/utils";
import { useInputMode, useStore } from "@/store";
import { type TabItemState, useTabBarState } from "@/store/selectors/tab-bar";
import { selectDisplaySettings } from "@/store/slices";

const startDrag = async (e: React.MouseEvent) => {
  e.preventDefault();
  try {
    await getCurrentWindow().startDragging();
  } catch (err) {
    logger.error("Failed to start dragging:", err);
  }
};

function buildTabNumberMap(tabs: TabItemState[]): Map<string, number> {
  const map = new Map<string, number>();
  let nextNumber = 0;

  for (const tab of tabs) {
    if (tab.tabType === "home") {
      continue;
    }

    if (nextNumber < 9) {
      map.set(tab.id, nextNumber);
    }
    nextNumber += 1;
  }

  return map;
}

interface TabBarProps {
  excludeTabIds?: string[];
  showDropHint?: boolean;
}

export const TabBar = React.memo(function TabBar({ excludeTabIds, showDropHint }: TabBarProps = {}) {
  // Use optimized selector that avoids subscribing to entire Record objects
  const { tabs: allTabs, activeSessionId } = useTabBarState();

  // Filter tabs to only show those belonging to the active conversation
  const activeConvTerminals = useStore((s) => {
    const convId = s.activeConversationId;
    if (!convId) return null;
    return s.conversationTerminals[convId] ?? null;
  });

  // Get active conversation title for the tab bar label
  const activeConvTitle = useStore((s) => {
    const convId = s.activeConversationId;
    if (!convId) return null;
    return s.conversations[convId]?.title ?? null;
  });

  const tabs = React.useMemo(() => {
    let filtered = allTabs;
    if (activeConvTerminals && activeConvTerminals.length > 0) {
      filtered = filtered.filter((tab) => {
        if (tab.tabType !== "terminal") return true;
        return activeConvTerminals.includes(tab.id);
      });
    }
    if (excludeTabIds && excludeTabIds.length > 0) {
      const excludeSet = new Set(excludeTabIds);
      filtered = filtered.filter((tab) => !excludeSet.has(tab.id));
    }
    return filtered;
  }, [allTabs, activeConvTerminals, excludeTabIds]);

  const tabNumberById = React.useMemo(() => buildTabNumberMap(tabs), [tabs]);

  // These actions don't cause re-renders - we only call them, not subscribe to changes
  const setActiveSession = useStore((state) => state.setActiveSession);
  const getTabSessionIds = useStore((state) => state.getTabSessionIds);
  const closeTab = useStore((state) => state.closeTab);
  const moveTab = useStore((state) => state.moveTab);
  const reorderTab = useStore((state) => state.reorderTab);
  const moveTabToPane = useStore((state) => state.moveTabToPane);

  const dragState = React.useRef<{
    draggedId: string | null;
    startX: number;
    startY: number;
    offsetX: number;
    offsetY: number;
    isDragging: boolean;
  }>({ draggedId: null, startX: 0, startY: 0, offsetX: 0, offsetY: 0, isDragging: false });
  const [draggedTabId, setDraggedTabId] = React.useState<string | null>(null);
  const [dragPos, setDragPos] = React.useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const [dropIndicator, setDropIndicator] = React.useState<{
    targetId: string;
    side: "left" | "right";
  } | null>(null);
  const tabRefs = React.useRef<Map<string, HTMLDivElement>>(new Map());

  // Display settings for animated show/hide of right-side buttons
  const display = useStore(selectDisplaySettings);
  const inputMode = useInputMode(activeSessionId ?? "");
  const hideAiItems = display.hideAiSettingsInShellMode && inputMode === "terminal";

  // State for convert-to-pane modal
  const [convertToPaneTab, setConvertToPaneTab] = React.useState<string | null>(null);

  const { createTerminalTab } = useCreateTerminalTab();

  // Track Cmd key press for showing tab numbers
  const [cmdKeyPressed, setCmdKeyPressed] = React.useState(false);
  React.useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Meta" && !e.repeat) {
        setCmdKeyPressed(true);
      }
    };
    const handleKeyUp = (e: KeyboardEvent) => {
      if (e.key === "Meta") {
        setCmdKeyPressed(false);
      }
    };
    const handleBlur = () => {
      setCmdKeyPressed(false);
    };

    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("keyup", handleKeyUp);
    window.addEventListener("blur", handleBlur);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("keyup", handleKeyUp);
      window.removeEventListener("blur", handleBlur);
    };
  }, []);

  // Custom scrollbar state for tab overflow
  const tabScrollRef = React.useRef<HTMLDivElement>(null);
  const [tabBarHovered, setTabBarHovered] = React.useState(false);
  const [scrollThumb, setScrollThumb] = React.useState({ left: 0, width: 0, visible: false });
  const thumbDragRef = React.useRef<{ startX: number; startScroll: number } | null>(null);

  const updateScrollThumb = React.useCallback(() => {
    const el = tabScrollRef.current;
    if (!el) return;
    const hasOverflow = el.scrollWidth > el.clientWidth + 1;
    if (!hasOverflow) {
      setScrollThumb({ left: 0, width: 0, visible: false });
      return;
    }
    const ratio = el.clientWidth / el.scrollWidth;
    const thumbWidth = Math.max(ratio * 100, 10);
    const scrollRange = el.scrollWidth - el.clientWidth;
    const thumbLeft = scrollRange > 0 ? (el.scrollLeft / scrollRange) * (100 - thumbWidth) : 0;
    setScrollThumb({ left: thumbLeft, width: thumbWidth, visible: true });
  }, []);

  React.useEffect(() => {
    const el = tabScrollRef.current;
    if (!el) return;
    updateScrollThumb();
    el.addEventListener("scroll", updateScrollThumb, { passive: true });
    const observer = new ResizeObserver(updateScrollThumb);
    observer.observe(el);
    return () => {
      el.removeEventListener("scroll", updateScrollThumb);
      observer.disconnect();
    };
  }, [updateScrollThumb, tabs.length]);

  React.useEffect(() => {
    const el = tabScrollRef.current;
    if (!el) return;
    const handler = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        e.preventDefault();
        el.scrollLeft += e.deltaY;
      }
    };
    el.addEventListener("wheel", handler, { passive: false });
    return () => el.removeEventListener("wheel", handler);
  }, []);

  const handleThumbDragStart = React.useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const el = tabScrollRef.current;
    if (!el) return;
    thumbDragRef.current = { startX: e.clientX, startScroll: el.scrollLeft };
    const onMove = (ev: MouseEvent) => {
      if (!thumbDragRef.current || !tabScrollRef.current) return;
      const trackEl = tabScrollRef.current;
      const dx = ev.clientX - thumbDragRef.current.startX;
      const trackWidth = trackEl.clientWidth;
      const scrollRange = trackEl.scrollWidth - trackEl.clientWidth;
      trackEl.scrollLeft = thumbDragRef.current.startScroll + (dx / trackWidth) * scrollRange;
    };
    const onUp = () => {
      thumbDragRef.current = null;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, []);

  const handleTabPointerDown = React.useCallback(
    (e: React.PointerEvent, tabId: string, tabType: TabItemState["tabType"]) => {
      if (tabType === "home" || e.button !== 0) return;
      const el = tabRefs.current.get(tabId);
      const rect = el?.getBoundingClientRect();
      const offsetX = rect ? e.clientX - rect.left : 0;
      const offsetY = rect ? e.clientY - rect.top : 0;
      dragState.current = { draggedId: tabId, startX: e.clientX, startY: e.clientY, offsetX, offsetY, isDragging: false };
    },
    []
  );

  React.useEffect(() => {
    const handlePointerMove = (e: PointerEvent) => {
      const ds = dragState.current;
      if (!ds.draggedId) return;

      if (!ds.isDragging && Math.abs(e.clientX - ds.startX) > 5) {
        ds.isDragging = true;
        setDraggedTabId(ds.draggedId);
        setDragPos({ x: e.clientX, y: e.clientY });
        document.documentElement.classList.add("tab-dragging");
      }

      if (!ds.isDragging) return;
      setDragPos({ x: e.clientX, y: e.clientY });

      const yDelta = e.clientY - ds.startY;
      window.dispatchEvent(new CustomEvent("tab-drag-split-hint", { detail: yDelta > 60 }));

      if (Math.abs(yDelta) < 30) {
        let closestId: string | null = null;
        let closestDist = Number.POSITIVE_INFINITY;
        let closestSide: "left" | "right" = "left";
        for (const [id, el] of tabRefs.current) {
          if (id === ds.draggedId) continue;
          const rect = el.getBoundingClientRect();
          const centerX = rect.left + rect.width / 2;
          const dist = Math.abs(e.clientX - centerX);
          if (dist < closestDist) {
            closestDist = dist;
            closestId = id;
            closestSide = e.clientX < centerX ? "left" : "right";
          }
        }
        if (closestId) {
          setDropIndicator({ targetId: closestId, side: closestSide });
        } else {
          setDropIndicator(null);
        }
      } else {
        setDropIndicator(null);
      }
    };

    const handlePointerUp = (e: PointerEvent) => {
      const ds = dragState.current;
      window.dispatchEvent(new CustomEvent("tab-drag-split-hint", { detail: false }));
      if (ds.isDragging && ds.draggedId) {
        const yDelta = e.clientY - ds.startY;
        const isOutsideWindow =
          e.clientX < 0 || e.clientY < 0 ||
          e.clientX > window.innerWidth || e.clientY > window.innerHeight;

        if (isOutsideWindow) {
          window.dispatchEvent(new CustomEvent("detach-tab", {
            detail: { tabId: ds.draggedId, screenX: e.screenX, screenY: e.screenY },
          }));
        } else if (yDelta > 60) {
          window.dispatchEvent(new CustomEvent("split-tab-right", { detail: ds.draggedId }));
        } else if (dropIndicator && Math.abs(yDelta) < 30) {
          const homeTabId = tabs[0]?.tabType === "home" ? tabs[0].id : null;
          if (dropIndicator.targetId !== homeTabId) {
            reorderTab(ds.draggedId, dropIndicator.targetId);
          }
        }
      }
      dragState.current = { draggedId: null, startX: 0, startY: 0, offsetX: 0, offsetY: 0, isDragging: false };
      setDraggedTabId(null);
      setDropIndicator(null);
      document.documentElement.classList.remove("tab-dragging");
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
    };
  }, [dropIndicator, tabs, reorderTab]);

  const handleCloseTab = React.useCallback(
    async (e: React.MouseEvent, tabId: string, tabType: TabItemState["tabType"]) => {
      e.stopPropagation();

      // Only perform PTY/AI cleanup for terminal tabs
      if (tabType === "terminal") {
        try {
          // Get all session IDs for this tab (root + all pane sessions)
          const sessionIds = getTabSessionIds(tabId);

          // If no panes found, fall back to just the tabId (backward compatibility)
          const idsToCleanup = sessionIds.length > 0 ? sessionIds : [tabId];

          // Shutdown AI and PTY for ALL sessions in this tab (in parallel)
          await Promise.all(
            idsToCleanup.map(async (sessionId) => {
              try {
                await shutdownAiSession(sessionId);
              } catch (err) {
                logger.error(`Failed to shutdown AI session ${sessionId}:`, err);
              }
              try {
                await ptyDestroy(sessionId);
              } catch (err) {
                logger.error(`Failed to destroy PTY ${sessionId}:`, err);
              }
              // Cleanup terminal instances
              TerminalInstanceManager.dispose(sessionId);
              liveTerminalManager.dispose(sessionId);
            })
          );
        } catch (err) {
          logger.error(`Error closing tab ${tabId}:`, err);
        }
      }

      // Remove terminal from its conversation
      const store = useStore.getState();
      const convId = store.getConversationForTerminal(tabId);
      if (convId) {
        store.removeTerminalFromConversation(convId, tabId);
      }

      // Remove all frontend state for the tab
      closeTab(tabId);
    },
    [getTabSessionIds, closeTab]
  );

  return (
    <TooltipProvider delayDuration={300}>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: div is used for window drag region */}
      <div
        className="relative z-[200] flex flex-col bg-transparent"
        onMouseDown={startDrag}
        onMouseEnter={() => setTabBarHovered(true)}
        onMouseLeave={() => setTabBarHovered(false)}
      >
        <div className="flex items-center h-[31px] pl-2 pr-2 gap-1">
        <div
          ref={tabScrollRef}
          className="min-w-0 overflow-x-auto scrollbar-none"
          onMouseDown={(e) => e.stopPropagation()}
        >
          <Tabs
            value={activeSessionId || undefined}
            onValueChange={setActiveSession}
          >
            <TabsList className="h-6 bg-transparent p-0 gap-1 w-max justify-start">
              {tabs.map((tab, index) => {
                const isActive = tab.id === activeSessionId;
                const isBusy = tab.tabType === "terminal" && (tab.isRunning || tab.hasPendingCommand);
                const hasNewActivity = tab.tabType === "terminal" && !isActive && tab.hasNewActivity;
                const isHomeTab = tab.tabType === "home";
                const homeVisible = display.showHomeTab;

                if (isHomeTab && !homeVisible) return null;

                return (
                  <TabItem
                    key={tab.id}
                    tab={tab}
                    index={index}
                    isActive={isActive}
                    isBusy={isBusy}
                    onClose={(e) => handleCloseTab(e, tab.id, tab.tabType)}
                    onDuplicateTab={createTerminalTab}
                    canClose={tab.tabType !== "home"}
                    canMoveLeft={index > 1}
                    canMoveRight={tab.tabType !== "home" && index < tabs.length - 1}
                    onMoveLeft={() => moveTab(tab.id, "left")}
                    onMoveRight={() => moveTab(tab.id, "right")}
                    onConvertToPane={() => {
                      logger.info("[TabBar] convert-to-pane: open", { sourceTabId: tab.id });
                      setConvertToPaneTab(tab.id);
                    }}
                    tabNumber={tabNumberById.get(tab.id)}
                    showTabNumber={cmdKeyPressed}
                    hasNewActivity={hasNewActivity}
                    isBeingDragged={draggedTabId === tab.id}
                    dropSide={
                      dropIndicator?.targetId === tab.id && draggedTabId !== tab.id
                        ? dropIndicator.side
                        : null
                    }
                    onTabPointerDown={(e) => handleTabPointerDown(e, tab.id, tab.tabType)}
                    tabRef={(el) => {
                      if (el) tabRefs.current.set(tab.id, el);
                      else tabRefs.current.delete(tab.id);
                    }}
                  />
                );
              })}
              {showDropHint && (
                <div className="h-6 px-2.5 flex items-center rounded-t-md border border-dashed border-accent/60 bg-accent/10 animate-in fade-in slide-in-from-right-2 duration-200">
                  <span className="text-[10px] font-mono text-accent/70 whitespace-nowrap animate-pulse">Drop here</span>
                </div>
              )}
            </TabsList>
          </Tabs>
        </div>

        {/* New tab button */}
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="New tab"
              title="New tab"
              onClick={() => createTerminalTab()}
              onMouseDown={(e) => e.stopPropagation()}
              className="h-5 w-5 text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]"
            >
              <Plus className="size-icon-tab-bar" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="bottom">
            <p>New tab (⌘T)</p>
          </TooltipContent>
        </Tooltip>

        {/* Recording controls for active terminal tab */}
        {(() => {
          const at = tabs.find((t) => t.id === activeSessionId);
          return at?.tabType === "terminal" ? (
            <TerminalRecordingControls sessionId={at.id} cols={80} rows={24} />
          ) : null;
        })()}

        {/* Active conversation label - shows which AI chat tab these terminals belong to */}
        {activeConvTitle && (
          <div
            className="flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[10px] text-muted-foreground/60 select-none max-w-[140px]"
            onMouseDown={(e) => e.stopPropagation()}
            title={`Terminals for: ${activeConvTitle}`}
          >
            <div className="w-1.5 h-1.5 rounded-full bg-accent/50 flex-shrink-0" />
            <span className="truncate">{activeConvTitle}</span>
          </div>
        )}

        {/* Drag region - empty space extends to fill remaining width */}
        <div className="flex-1 h-full min-w-[100px]" />

        {/* Right-side utility buttons hidden for cleaner layout */}
        </div>
        {/* Custom scrollbar track */}
        {tabBarHovered && scrollThumb.visible && (
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

      {/* Convert to Pane Modal */}
      {convertToPaneTab && (
        <ConvertToPaneModal
          sourceTabId={convertToPaneTab}
          tabs={tabs}
          onClose={() => setConvertToPaneTab(null)}
          onConfirm={(destTabId, location) => {
            logger.info("[TabBar] convert-to-pane: confirm", {
              sourceTabId: convertToPaneTab,
              destTabId,
              location,
            });
            moveTabToPane(convertToPaneTab, destTabId, location);
            setConvertToPaneTab(null);
          }}
        />
      )}
      {/* Floating ghost tab that follows cursor during drag */}
      {draggedTabId && (() => {
        const draggedTab = tabs.find((t) => t.id === draggedTabId);
        if (!draggedTab) return null;
        const IconComp = draggedTab.tabType === "home" ? Home
          : draggedTab.tabType === "settings" ? Settings
          : draggedTab.tabType === "browser" ? Globe
          : draggedTab.tabType === "security" ? Shield
          : draggedTab.mode === "agent" ? Bot : Terminal;
        const label = draggedTab.customName
          || (draggedTab.tabType === "browser" ? "Browser" : null)
          || (draggedTab.tabType === "security" ? "Security" : null)
          || (draggedTab.tabType === "settings" ? draggedTab.name || "Settings" : null)
          || draggedTab.processName
          || draggedTab.workingDirectory.split(/[/\\]/).pop()
          || "Tab";
        return createPortal(
          <div
            className="fixed z-[9999] pointer-events-none flex items-center gap-1.5 px-3 py-1 rounded-md bg-muted/90 border border-accent text-foreground text-[11px] font-mono shadow-lg backdrop-blur-sm"
            style={{
              left: dragPos.x,
              top: dragPos.y,
              transform: "translate(-50%, -50%)",
            }}
          >
            <IconComp className="w-3 h-3 text-accent flex-shrink-0" />
            <span className="truncate max-w-[120px]">{label}</span>
          </div>,
          document.body
        );
      })()}
    </TooltipProvider>
  );
});

interface TabItemProps {
  tab: TabItemState;
  index: number;
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

const TabItem = React.memo(function TabItem({
  tab,
  index,
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

  // Determine display name:
  // - home: no text label (icon only)
  // - settings: use tab.name (or custom name)
  // - terminal: custom name > process name > directory name
  const { displayName, dirName, isCustomName, isProcessName } = React.useMemo(() => {
    if (tabType === "home") {
      return {
        displayName: "", // No text for home tab - icon only
        dirName: "",
        isCustomName: false,
        isProcessName: false,
      };
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

  // Focus input when entering edit mode
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
    // Use getState() pattern to avoid subscription overhead
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

  // Generate tooltip text showing full context
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
        isBeingDragged && "opacity-60 ring-2 ring-accent bg-accent/10 rounded-md scale-[0.92] transition-all duration-150"
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
                  canClose && "pr-7" // Add padding for close button
                )}
              >
                {/* Active indicator underline */}
                {isActive && <span className="absolute bottom-0 left-0 right-0 h-px bg-accent" />}

                {/* Busy spinner - only shown when tab is busy */}
                {isBusy && (
                  <Loader2
                    className={cn(
                      "size-icon-tab-bar flex-shrink-0 animate-spin",
                      isActive ? "text-accent" : "text-muted-foreground"
                    )}
                  />
                )}

                {/* New activity indicator dot - shown when inactive tab has new activity */}
                {hasNewActivity && !isBusy && (
                  <span
                    aria-hidden="true"
                    className="activity-dot w-1.5 h-1.5 flex-shrink-0 rounded-full bg-[var(--ansi-yellow)]"
                  />
                )}

                {/* Icon for non-terminal tabs (home, settings) - these don't have text labels */}
                {tabType !== "terminal" && !isBusy && (
                  <ModeIcon
                    className={cn(
                      "size-icon-tab-bar flex-shrink-0",
                      isActive ? "text-accent" : "text-muted-foreground"
                    )}
                  />
                )}

                {/* Tab name or edit input - not rendered for home tab (icon only) */}
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

                {/* Tab number badge - shown when Cmd is held */}
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

          {/* Close button - positioned outside Tooltip to avoid event interference */}
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
        {/* Move left/right - available on all non-home tabs */}
        <ContextMenuItem onClick={onMoveLeft} disabled={!canMoveLeft}>
          <ArrowLeft className="size-icon-tab-bar" />
          Move Left
        </ContextMenuItem>
        <ContextMenuItem onClick={onMoveRight} disabled={!canMoveRight}>
          <ArrowRight className="size-icon-tab-bar" />
          Move Right
        </ContextMenuItem>
        <ContextMenuSeparator />
        {/* Split to right - available on all terminal tabs */}
        {tabType === "terminal" && (
          <ContextMenuItem onClick={() => window.dispatchEvent(new CustomEvent("split-tab-right", { detail: tab.id }))}>
            <Columns className="size-icon-tab-bar" />
            Split to Right
          </ContextMenuItem>
        )}
        {/* Convert to pane - only for terminal tabs */}
        {tabType === "terminal" && (
          <ContextMenuItem onClick={onConvertToPane}>
            <PanelLeft className="size-icon-tab-bar" />
            Convert to Pane
          </ContextMenuItem>
        )}
        {(tabType === "terminal" || tabType === "security") && (
          <ContextMenuItem onClick={() => {
            window.dispatchEvent(new CustomEvent("detach-tab", {
              detail: { tabId: tab.id, screenX: window.screenX + 100, screenY: window.screenY + 100 },
            }));
          }}>
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

interface ConvertToPaneModalProps {
  sourceTabId: string;
  tabs: TabItemState[];
  onClose: () => void;
  onConfirm: (destTabId: string, location: "left" | "right" | "top" | "bottom") => void;
}

function ConvertToPaneModal({ sourceTabId, tabs, onClose, onConfirm }: ConvertToPaneModalProps) {
  // Filter to only show terminal tabs that aren't the source, preserving their original index
  const destTabs = tabs
    .map((t, index) => ({ tab: t, index }))
    .filter(({ tab }) => tab.tabType === "terminal" && tab.id !== sourceTabId);
  const [destTabId, setDestTabId] = React.useState(destTabs[0]?.tab.id ?? "");
  const [location, setLocation] = React.useState<"left" | "right" | "top" | "bottom">("right");

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[400px]" onMouseDown={(e) => e.stopPropagation()}>
        <DialogHeader>
          <DialogTitle>Convert to Pane</DialogTitle>
          <DialogDescription>Move this tab as a pane into another tab.</DialogDescription>
        </DialogHeader>
        <div className="grid gap-4 py-2">
          <div className="grid gap-2">
            <span className="text-sm font-medium">Destination Tab</span>
            <Select value={destTabId} onValueChange={setDestTabId}>
              <SelectTrigger className="w-full">
                <SelectValue placeholder="Select a tab" />
              </SelectTrigger>
              <SelectContent>
                {destTabs.map(({ tab, index }) => (
                  <SelectItem key={tab.id} value={tab.id}>
                    <span className="text-muted-foreground mr-1.5">{index}.</span>
                    {tab.customName || tab.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-2">
            <span className="text-sm font-medium">Placement</span>
            <Select value={location} onValueChange={(v) => setLocation(v as typeof location)}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="right">Right</SelectItem>
                <SelectItem value="left">Left</SelectItem>
                <SelectItem value="bottom">Bottom</SelectItem>
                <SelectItem value="top">Top</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={() => onConfirm(destTabId, location)} disabled={!destTabId}>
            Convert
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
