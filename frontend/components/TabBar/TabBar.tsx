import { getCurrentWindow } from "@tauri-apps/api/window";
import { Bot, Globe, Home, Plus, Settings, Shield, Terminal } from "lucide-react";
import React from "react";
import { createPortal } from "react-dom";
import { TerminalRecordingControls } from "@/components/Terminal/TerminalRecordingControls";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useCreateTerminalTab } from "@/hooks/useCreateTerminalTab";
import { logger } from "@/lib/logger";
import { closeTabAndCleanup, useStore } from "@/store";
import { type TabItemState, useTabBarState } from "@/store/selectors/tab-bar";
import { selectDisplaySettings } from "@/store/slices";
import { ConvertToPaneModal } from "./ConvertToPaneModal";
import { TabItem } from "./TabItem";

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

export const TabBar = React.memo(function TabBar({
  excludeTabIds,
  showDropHint,
}: TabBarProps = {}) {
  const { tabs: allTabs, activeSessionId } = useTabBarState();

  const activeConvTerminals = useStore((s) => {
    const convId = s.activeConversationId;
    if (!convId) return null;
    return s.conversationTerminals[convId] ?? null;
  });

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

  const setActiveSession = useStore((state) => state.setActiveSession);
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

  const display = useStore(selectDisplaySettings);
  const [convertToPaneTab, setConvertToPaneTab] = React.useState<string | null>(null);

  const { createTerminalTab } = useCreateTerminalTab();

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
  }, [updateScrollThumb]);

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
      dragState.current = {
        draggedId: tabId,
        startX: e.clientX,
        startY: e.clientY,
        offsetX,
        offsetY,
        isDragging: false,
      };
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
          e.clientX < 0 ||
          e.clientY < 0 ||
          e.clientX > window.innerWidth ||
          e.clientY > window.innerHeight;

        if (isOutsideWindow) {
          window.dispatchEvent(
            new CustomEvent("detach-tab", {
              detail: { tabId: ds.draggedId, screenX: e.screenX, screenY: e.screenY },
            })
          );
        } else if (yDelta > 60) {
          window.dispatchEvent(new CustomEvent("split-tab-right", { detail: ds.draggedId }));
        } else if (dropIndicator && Math.abs(yDelta) < 30) {
          const homeTabId = tabs[0]?.tabType === "home" ? tabs[0].id : null;
          if (dropIndicator.targetId !== homeTabId) {
            reorderTab(ds.draggedId, dropIndicator.targetId);
          }
        }
      }
      dragState.current = {
        draggedId: null,
        startX: 0,
        startY: 0,
        offsetX: 0,
        offsetY: 0,
        isDragging: false,
      };
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
      try {
        await closeTabAndCleanup(tabId, { tabType, createTerminalTab });
      } catch (err) {
        logger.error(`Error closing tab ${tabId}:`, err);
      }
    },
    [createTerminalTab]
  );

  return (
    <TooltipProvider delayDuration={300}>
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
            <Tabs value={activeSessionId || undefined} onValueChange={setActiveSession}>
              <TabsList className="h-6 bg-transparent p-0 gap-1 w-max justify-start">
                {tabs.map((tab, index) => {
                  const isActive = tab.id === activeSessionId;
                  const isBusy =
                    tab.tabType === "terminal" && (tab.isRunning || tab.hasPendingCommand);
                  const hasNewActivity =
                    tab.tabType === "terminal" && !isActive && tab.hasNewActivity;
                  const isHomeTab = tab.tabType === "home";
                  const homeVisible = display.showHomeTab;

                  if (isHomeTab && !homeVisible) return null;

                  return (
                    <TabItem
                      key={tab.id}
                      tab={tab}
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
                    <span className="text-[10px] font-mono text-accent/70 whitespace-nowrap animate-pulse">
                      Drop here
                    </span>
                  </div>
                )}
              </TabsList>
            </Tabs>
          </div>

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

          {(() => {
            const at = tabs.find((t) => t.id === activeSessionId);
            return at?.tabType === "terminal" ? (
              <TerminalRecordingControls sessionId={at.id} cols={80} rows={24} />
            ) : null;
          })()}

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

          <div className="flex-1 h-full min-w-[100px]" />
        </div>
        {tabBarHovered && scrollThumb.visible && (
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
      {draggedTabId &&
        (() => {
          const draggedTab = tabs.find((t) => t.id === draggedTabId);
          if (!draggedTab) return null;
          const IconComp =
            draggedTab.tabType === "home"
              ? Home
              : draggedTab.tabType === "settings"
                ? Settings
                : draggedTab.tabType === "browser"
                  ? Globe
                  : draggedTab.tabType === "security"
                    ? Shield
                    : draggedTab.mode === "agent"
                      ? Bot
                      : Terminal;
          const label =
            draggedTab.customName ||
            (draggedTab.tabType === "browser" ? "Browser" : null) ||
            (draggedTab.tabType === "security" ? "Security" : null) ||
            (draggedTab.tabType === "settings" ? draggedTab.name || "Settings" : null) ||
            draggedTab.processName ||
            draggedTab.workingDirectory.split(/[/\\]/).pop() ||
            "Tab";
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
