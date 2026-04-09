import { memo, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  Home,
  Bug,
  ClipboardList,
  Crosshair,
  GitBranch,
  Layers,
  Wrench,
  Settings,
  Terminal,
  Shield,
  ScrollText,
  BookText,
  AlertTriangle,
  FolderOpen,
  Hammer,
  MoreHorizontal,
  ChevronRight,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useTranslation } from "react-i18next";

export type ActivityView = "dashboard" | "wiki" | "targets" | "methodology" | "findings" | "pipelines" | "auditLog" | "wordlists" | "vulnIntel" | "toolManage" | "settings" | "security" | null;

type BarItemId = "dashboard" | "targets" | "findings" | "pipelines" | "auditLog" | "wordlists" | "vulnIntel" | "security" | "terminal" | "wiki" | "methodology" | "toolManage";

interface BarItem {
  id: BarItemId;
  icon: LucideIcon;
  label: string;
}

interface BarGroup {
  id: string;
  icon: LucideIcon;
  label: string;
  items: BarItem[];
}

const UPPER_DEFAULTS: BarItem[] = [
  { id: "dashboard", icon: Layers, label: "activity.dashboard" },
  { id: "targets", icon: Crosshair, label: "activity.targets" },
  { id: "findings", icon: Bug, label: "activity.findings" },
  { id: "security", icon: Shield, label: "activity.security" },
  { id: "terminal", icon: Terminal, label: "activity.terminal" },
];

const LOWER_GROUPS: BarGroup[] = [
  {
    id: "knowledge",
    icon: FolderOpen,
    label: "activity.knowledge",
    items: [
      { id: "vulnIntel", icon: AlertTriangle, label: "activity.vulnKb" },
      { id: "methodology", icon: ClipboardList, label: "activity.methodology" },
      { id: "wordlists", icon: BookText, label: "activity.wordlists" },
    ],
  },
  {
    id: "tools",
    icon: Hammer,
    label: "activity.tools",
    items: [
      { id: "toolManage", icon: Wrench, label: "activity.toolManage" },
      { id: "pipelines", icon: GitBranch, label: "activity.pipelines" },
    ],
  },
  {
    id: "system",
    icon: MoreHorizontal,
    label: "activity.system",
    items: [
      { id: "auditLog", icon: ScrollText, label: "activity.auditLog" },
    ],
  },
];

const ITEM_HEIGHT = 44;
const DRAG_THRESHOLD = 4;
const ACTIVITY_ORDER_KEY = "golish-activity-bar-order";

function loadSavedOrder(): BarItem[] | null {
  try {
    const raw = localStorage.getItem(ACTIVITY_ORDER_KEY);
    if (!raw) return null;
    const ids = JSON.parse(raw) as BarItemId[];
    const lookup = new Map(UPPER_DEFAULTS.map((i) => [i.id, i]));
    const ordered = ids.map((id) => lookup.get(id)).filter(Boolean) as BarItem[];
    for (const item of UPPER_DEFAULTS) {
      if (!ordered.find((o) => o.id === item.id)) ordered.push(item);
    }
    return ordered;
  } catch {
    return null;
  }
}

interface ActivityBarProps {
  activeView: ActivityView;
  onViewChange: (view: ActivityView) => void;
  terminalOpen?: boolean;
  onToggleTerminal?: () => void;
  onOpenSettings?: () => void;
}

const VIEW_ITEMS: BarItemId[] = ["dashboard", "targets", "findings", "pipelines", "auditLog", "wordlists", "vulnIntel", "wiki", "methodology", "toolManage", "security"];

export const ActivityBar = memo(function ActivityBar({
  activeView,
  onViewChange,
  terminalOpen,
  onToggleTerminal,
}: ActivityBarProps) {
  const { t } = useTranslation();
  const [upperItems, setUpperItems] = useState(() => loadSavedOrder() ?? UPPER_DEFAULTS);
  const [expandedGroup, setExpandedGroup] = useState<string | null>(null);
  const groupRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const flyoutRef = useRef<HTMLDivElement | null>(null);

  const [drag, setDrag] = useState<{
    section: "upper";
    fromIndex: number;
    hoverIndex: number;
    ghostX: number;
    ghostY: number;
    item: BarItem;
  } | null>(null);

  const dragStartRef = useRef<{ section: "upper"; index: number; startX: number; startY: number; moved: boolean } | null>(null);
  const hoverIndexRef = useRef(0);
  const itemRefsUpper = useRef<(HTMLButtonElement | null)[]>([]);

  useEffect(() => {
    if (!expandedGroup) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      if (flyoutRef.current?.contains(target)) return;
      const btn = groupRefs.current[expandedGroup];
      if (btn?.contains(target)) return;
      setExpandedGroup(null);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [expandedGroup]);

  const handleClick = useCallback((item: BarItem) => {
    if (VIEW_ITEMS.includes(item.id)) {
      const viewId = item.id as ActivityView;
      onViewChange(activeView === viewId ? null : viewId);
      setExpandedGroup(null);
    } else if (item.id === "terminal") {
      onToggleTerminal?.();
    }
  }, [activeView, onViewChange, onToggleTerminal]);

  const getItemActive = useCallback((item: BarItem) => {
    if (VIEW_ITEMS.includes(item.id)) return activeView === item.id;
    if (item.id === "terminal") return !!terminalOpen;
    return false;
  }, [activeView, terminalOpen]);

  const isGroupActive = useCallback((group: BarGroup) => {
    return group.items.some((item) => getItemActive(item));
  }, [getItemActive]);

  const handlePointerDown = useCallback((
    e: React.PointerEvent,
    index: number,
  ) => {
    dragStartRef.current = { section: "upper", index, startX: e.clientX, startY: e.clientY, moved: false };

    const items = upperItems;
    const item = items[index];
    const el = itemRefsUpper.current[index];
    if (!el || !item) return;

    const rect = el.getBoundingClientRect();
    const offsetY = e.clientY - rect.top;

    const onMove = (ev: PointerEvent) => {
      if (!dragStartRef.current) return;
      const dx = ev.clientX - dragStartRef.current.startX;
      const dy = ev.clientY - dragStartRef.current.startY;
      if (!dragStartRef.current.moved && Math.abs(dx) + Math.abs(dy) < DRAG_THRESHOLD) return;
      dragStartRef.current.moved = true;

      const delta = ev.clientY - dragStartRef.current.startY;
      const steps = Math.round(delta / ITEM_HEIGHT);
      const hoverIdx = Math.max(0, Math.min(items.length - 1, index + steps));
      hoverIndexRef.current = hoverIdx;

      setDrag({
        section: "upper",
        fromIndex: index,
        hoverIndex: hoverIdx,
        ghostX: rect.left,
        ghostY: ev.clientY - offsetY,
        item,
      });
    };

    const onUp = () => {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);

      if (dragStartRef.current?.moved) {
        const targetIdx = hoverIndexRef.current;
        if (targetIdx !== index) {
          setUpperItems((prev) => {
            const next = [...prev];
            const [moved] = next.splice(index, 1);
            next.splice(targetIdx, 0, moved);
            try {
              localStorage.setItem(ACTIVITY_ORDER_KEY, JSON.stringify(next.map((i) => i.id)));
            } catch { /* ignore */ }
            return next;
          });
        }
      } else if (dragStartRef.current && !dragStartRef.current.moved) {
        handleClick(item);
      }

      dragStartRef.current = null;
      setDrag(null);
    };

    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
    e.preventDefault();
  }, [upperItems, handleClick]);

  const getTranslateY = useCallback((index: number) => {
    if (!drag || drag.section !== "upper") return 0;
    if (index === drag.fromIndex) return 0;
    const from = drag.fromIndex;
    const hover = drag.hoverIndex;
    if (from < hover && index > from && index <= hover) return -ITEM_HEIGHT;
    if (from > hover && index < from && index >= hover) return ITEM_HEIGHT;
    return 0;
  }, [drag]);

  const renderUpperItem = useCallback((item: BarItem, index: number) => {
    const active = getItemActive(item);
    const isDragging = drag?.section === "upper" && drag.fromIndex === index;
    const ty = getTranslateY(index);

    return (
      <Tooltip key={item.id}>
        <TooltipTrigger asChild>
          <button
            ref={(el) => { itemRefsUpper.current[index] = el; }}
            type="button"
            className={cn(
              "relative w-10 h-10 flex items-center justify-center rounded-md",
              "cursor-pointer select-none",
              active
                ? "text-foreground bg-[var(--bg-hover)]"
                : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]",
              isDragging ? "opacity-0" : "transition-transform duration-200 ease-out",
            )}
            style={{ transform: isDragging ? undefined : `translateY(${ty}px)` }}
            onPointerDown={(e) => handlePointerDown(e, index)}
          >
            <item.icon className="w-[18px] h-[18px]" />
            {active && !isDragging && (
              <span className="absolute left-0 top-1/2 -translate-y-1/2 w-[2px] h-5 bg-accent rounded-r" />
            )}
          </button>
        </TooltipTrigger>
        {!drag && (
          <TooltipContent side="right" sideOffset={8}>
            <p className="text-xs">{t(item.label)}</p>
          </TooltipContent>
        )}
      </Tooltip>
    );
  }, [getItemActive, drag, getTranslateY, handlePointerDown, t]);

  const toggleGroup = useCallback((groupId: string) => {
    setExpandedGroup((prev) => (prev === groupId ? null : groupId));
  }, []);

  const getFlyoutPosition = useCallback((groupId: string) => {
    const btn = groupRefs.current[groupId];
    if (!btn) return { top: 0, left: 0 };
    const rect = btn.getBoundingClientRect();
    return { top: rect.top, left: rect.right + 8 };
  }, []);

  return (
    <TooltipProvider delayDuration={200}>
      <div className="w-[48px] flex-shrink-0 h-full bg-card rounded-xl flex flex-col items-center overflow-hidden panel-float">
        {/* Upper section */}
        <div className="flex flex-col items-center gap-1 pt-3 flex-1">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                className={cn(
                  "relative w-10 h-10 flex items-center justify-center rounded-md",
                  "transition-colors cursor-pointer",
                  activeView === null
                    ? "text-foreground bg-[var(--bg-hover)]"
                    : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]",
                )}
                onClick={() => onViewChange(null)}
              >
                <Home className="w-[18px] h-[18px]" />
                {activeView === null && (
                  <span className="absolute left-0 top-1/2 -translate-y-1/2 w-[2px] h-5 bg-accent rounded-r" />
                )}
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              <p className="text-xs">{t("activity.home")}</p>
            </TooltipContent>
          </Tooltip>

          <div className="w-6 border-t border-border/10 my-0.5" />

          {upperItems.map((item, i) => renderUpperItem(item, i))}
        </div>

        {/* Lower section - collapsible groups */}
        <div className="flex flex-col items-center gap-1 pb-3">
          {LOWER_GROUPS.map((group) => {
            const active = isGroupActive(group);
            const expanded = expandedGroup === group.id;
            return (
              <Tooltip key={group.id}>
                <TooltipTrigger asChild>
                  <button
                    ref={(el) => { groupRefs.current[group.id] = el; }}
                    type="button"
                    className={cn(
                      "relative w-10 h-10 flex items-center justify-center rounded-md",
                      "transition-colors cursor-pointer",
                      active || expanded
                        ? "text-foreground bg-[var(--bg-hover)]"
                        : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]",
                    )}
                    onClick={() => toggleGroup(group.id)}
                  >
                    <group.icon className="w-[18px] h-[18px]" />
                    {active && !expanded && (
                      <span className="absolute left-0 top-1/2 -translate-y-1/2 w-[2px] h-5 bg-accent rounded-r" />
                    )}
                    {expanded && (
                      <span className="absolute right-0.5 top-0.5 w-1.5 h-1.5 rounded-full bg-accent" />
                    )}
                  </button>
                </TooltipTrigger>
                {!expanded && (
                  <TooltipContent side="right" sideOffset={8}>
                    <p className="text-xs">{t(group.label)}</p>
                  </TooltipContent>
                )}
              </Tooltip>
            );
          })}

          <div className="w-6 border-t border-border/10 my-0.5" />

          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                className={cn(
                  "relative w-10 h-10 flex items-center justify-center rounded-md",
                  "transition-colors cursor-pointer",
                  activeView === "settings"
                    ? "text-foreground bg-[var(--bg-hover)]"
                    : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]",
                )}
                onClick={() => onViewChange(activeView === "settings" ? null : "settings")}
              >
                <Settings className="w-[18px] h-[18px]" />
                {activeView === "settings" && (
                  <span className="absolute left-0 top-1/2 -translate-y-1/2 w-[2px] h-5 bg-accent rounded-r" />
                )}
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              <p className="text-xs">{t("activity.settings")}</p>
            </TooltipContent>
          </Tooltip>
        </div>
      </div>

      {/* Flyout panel for expanded group */}
      {expandedGroup && createPortal(
        (() => {
          const group = LOWER_GROUPS.find((g) => g.id === expandedGroup);
          if (!group) return null;
          const pos = getFlyoutPosition(expandedGroup);
          return (
            <div
              ref={flyoutRef}
              className="fixed z-[9999] animate-in fade-in-0 slide-in-from-left-2 duration-150"
              style={{ top: pos.top - 8, left: pos.left }}
            >
              <div className="rounded-xl border border-border/20 bg-[#1a1a2e] shadow-2xl py-1.5 px-1 min-w-[160px]">
                <div className="px-2 py-1 text-[9px] text-muted-foreground/40 font-medium uppercase tracking-wide">
                  {t(group.label)}
                </div>
                {group.items.map((item) => {
                  const active = getItemActive(item);
                  return (
                    <button
                      key={item.id}
                      type="button"
                      className={cn(
                        "w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg transition-colors",
                        active
                          ? "text-accent bg-accent/10"
                          : "text-foreground/70 hover:text-foreground hover:bg-muted/10",
                      )}
                      onClick={() => handleClick(item)}
                    >
                      <item.icon className="w-4 h-4 flex-shrink-0" />
                      <span className="text-[11px] font-medium">{t(item.label)}</span>
                      {active && <ChevronRight className="w-3 h-3 ml-auto text-accent/50" />}
                    </button>
                  );
                })}
              </div>
            </div>
          );
        })(),
        document.body,
      )}

      {/* Floating ghost during drag */}
      {drag && createPortal(
        <div
          className="fixed z-[9999] pointer-events-none"
          style={{
            left: drag.ghostX,
            top: drag.ghostY,
            width: 40,
            height: 40,
          }}
        >
          <div className={cn(
            "w-10 h-10 flex items-center justify-center rounded-md",
            "bg-card border border-accent/30 shadow-lg shadow-accent/10",
            "text-foreground",
          )}>
            <drag.item.icon className="w-[18px] h-[18px]" />
          </div>
        </div>,
        document.body,
      )}
    </TooltipProvider>
  );
});
