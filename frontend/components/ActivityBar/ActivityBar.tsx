import { memo, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  Home,
  Bug,
  ClipboardList,
  Crosshair,
  BookOpen,
  GitBranch,
  Layers,
  Network,
  Wrench,
  Settings,
  Terminal,
  Globe,
  Shield,
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

export type ActivityView = "dashboard" | "wiki" | "targets" | "topology" | "methodology" | "findings" | "pipelines" | "toolManage" | "settings" | null;

type BarItemId = "dashboard" | "targets" | "topology" | "findings" | "pipelines" | "security" | "browser" | "terminal" | "wiki" | "methodology" | "toolManage";

interface BarItem {
  id: BarItemId;
  icon: LucideIcon;
  label: string;
}

const UPPER_DEFAULTS: BarItem[] = [
  { id: "dashboard", icon: Layers, label: "activity.dashboard" },
  { id: "targets", icon: Crosshair, label: "activity.targets" },
  { id: "topology", icon: Network, label: "activity.topology" },
  { id: "findings", icon: Bug, label: "activity.findings" },
  { id: "security", icon: Shield, label: "activity.security" },
  { id: "browser", icon: Globe, label: "activity.browser" },
  { id: "terminal", icon: Terminal, label: "activity.terminal" },
];

const LOWER_DEFAULTS: BarItem[] = [
  { id: "wiki", icon: BookOpen, label: "activity.wiki" },
  { id: "methodology", icon: ClipboardList, label: "activity.methodology" },
  { id: "pipelines", icon: GitBranch, label: "activity.pipelines" },
  { id: "toolManage", icon: Wrench, label: "activity.toolManage" },
];

const STORAGE_KEY = "golish-activity-bar-order";
const ITEM_HEIGHT = 44;
const DRAG_THRESHOLD = 4;

function loadOrder(): { upper: BarItemId[]; lower: BarItemId[] } | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function saveOrder(upper: BarItemId[], lower: BarItemId[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ upper, lower }));
}

function reorderItems(defaults: BarItem[], savedIds: BarItemId[] | undefined): BarItem[] {
  if (!savedIds) return defaults;
  const map = new Map(defaults.map((d) => [d.id, d]));
  const ordered: BarItem[] = [];
  for (const id of savedIds) {
    const item = map.get(id);
    if (item) ordered.push(item);
  }
  for (const d of defaults) {
    if (!ordered.find((o) => o.id === d.id)) ordered.push(d);
  }
  return ordered;
}

interface ActivityBarProps {
  activeView: ActivityView;
  onViewChange: (view: ActivityView) => void;
  terminalOpen?: boolean;
  onToggleTerminal?: () => void;
  onOpenSettings?: () => void;
  onOpenBrowser?: () => void;
  onOpenSecurity?: () => void;
  browserOpen?: boolean;
  securityOpen?: boolean;
}

export const ActivityBar = memo(function ActivityBar({
  activeView,
  onViewChange,
  terminalOpen,
  onToggleTerminal,
  onOpenBrowser,
  onOpenSecurity,
  browserOpen,
  securityOpen,
}: ActivityBarProps) {
  const { t } = useTranslation();
  const saved = loadOrder();
  const [upperItems, setUpperItems] = useState(() => reorderItems(UPPER_DEFAULTS, saved?.upper));
  const [lowerItems, setLowerItems] = useState(() => reorderItems(LOWER_DEFAULTS, saved?.lower));

  const [drag, setDrag] = useState<{
    section: "upper" | "lower";
    fromIndex: number;
    hoverIndex: number;
    ghostX: number;
    ghostY: number;
    item: BarItem;
  } | null>(null);

  const dragStartRef = useRef<{ section: "upper" | "lower"; index: number; startX: number; startY: number; moved: boolean } | null>(null);
  const hoverIndexRef = useRef(0);

  useEffect(() => {
    saveOrder(
      upperItems.map((i) => i.id),
      lowerItems.map((i) => i.id),
    );
  }, [upperItems, lowerItems]);

  const handleClick = useCallback((item: BarItem) => {
    const viewItems: BarItemId[] = ["dashboard", "targets", "topology", "findings", "pipelines", "wiki", "methodology", "toolManage"];
    if (viewItems.includes(item.id)) {
      const viewId = item.id as ActivityView;
      onViewChange(activeView === viewId ? null : viewId);
    } else if (item.id === "security") {
      onOpenSecurity?.();
    } else if (item.id === "browser") {
      onOpenBrowser?.();
    } else if (item.id === "terminal") {
      onToggleTerminal?.();
    }
  }, [activeView, onViewChange, onOpenSecurity, onOpenBrowser, onToggleTerminal]);

  const getItemActive = useCallback((item: BarItem) => {
    const viewItems: BarItemId[] = ["dashboard", "targets", "topology", "findings", "pipelines", "wiki", "methodology", "toolManage"];
    if (viewItems.includes(item.id)) return activeView === item.id;
    if (item.id === "browser") return !!browserOpen;
    if (item.id === "terminal") return !!terminalOpen;
    if (item.id === "security") return !!securityOpen;
    return false;
  }, [activeView, browserOpen, terminalOpen, securityOpen]);

  const itemRefsUpper = useRef<(HTMLButtonElement | null)[]>([]);
  const itemRefsLower = useRef<(HTMLButtonElement | null)[]>([]);

  const handlePointerDown = useCallback((
    e: React.PointerEvent,
    section: "upper" | "lower",
    index: number,
  ) => {
    dragStartRef.current = { section, index, startX: e.clientX, startY: e.clientY, moved: false };

    const items = section === "upper" ? upperItems : lowerItems;
    const refs = section === "upper" ? itemRefsUpper : itemRefsLower;
    const item = items[index];
    const el = refs.current[index];
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
        section,
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
        const newItems = [...items];
        const hoverIdx = hoverIndexRef.current;
        if (hoverIdx !== index) {
          const [moved] = newItems.splice(index, 1);
          newItems.splice(hoverIdx, 0, moved);
          if (section === "upper") setUpperItems(newItems);
          else setLowerItems(newItems);
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
  }, [upperItems, lowerItems, handleClick]);

  const getTranslateY = useCallback((section: "upper" | "lower", index: number) => {
    if (!drag || drag.section !== section) return 0;
    if (index === drag.fromIndex) return 0;
    const from = drag.fromIndex;
    const hover = drag.hoverIndex;
    if (from < hover && index > from && index <= hover) return -ITEM_HEIGHT;
    if (from > hover && index < from && index >= hover) return ITEM_HEIGHT;
    return 0;
  }, [drag]);

  const renderItem = useCallback((item: BarItem, section: "upper" | "lower", index: number) => {
    const active = getItemActive(item);
    const isDragging = drag?.section === section && drag.fromIndex === index;
    const ty = getTranslateY(section, index);
    const refs = section === "upper" ? itemRefsUpper : itemRefsLower;

    return (
      <Tooltip key={item.id}>
        <TooltipTrigger asChild>
          <button
            ref={(el) => { refs.current[index] = el; }}
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
            onPointerDown={(e) => handlePointerDown(e, section, index)}
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

  return (
    <TooltipProvider delayDuration={200}>
      <div className="w-[48px] flex-shrink-0 h-full bg-card rounded-xl flex flex-col items-center overflow-hidden panel-float">
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

          {upperItems.map((item, i) => renderItem(item, "upper", i))}
        </div>

        <div className="flex flex-col items-center gap-1 pb-3">
          {lowerItems.map((item, i) => renderItem(item, "lower", i))}

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
