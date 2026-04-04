import { memo } from "react";
import {
  Home,
  Shield,
  Search,
  FolderTree,
  Database,
  BookOpen,
  Wrench,
  Settings,
  Terminal,
  Globe,
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

export type ActivityView = "tools" | "search" | "explorer" | "database" | "wiki" | "toolManage" | "settings" | null;

interface ActivityBarItem {
  id: ActivityView;
  icon: LucideIcon;
  label: string;
}

const ACTIVITY_ITEMS: ActivityBarItem[] = [
  { id: "search", icon: Search, label: "activity.search" },
  { id: "tools", icon: Shield, label: "activity.tools" },
  { id: "explorer", icon: FolderTree, label: "activity.explorer" },
  { id: "database", icon: Database, label: "activity.database" },
  { id: "wiki", icon: BookOpen, label: "activity.wiki" },
  { id: "toolManage", icon: Wrench, label: "activity.toolManage" },
];

interface ActivityBarProps {
  activeView: ActivityView;
  onViewChange: (view: ActivityView) => void;
  terminalOpen?: boolean;
  onToggleTerminal?: () => void;
  onOpenSettings?: () => void;
  onOpenBrowser?: () => void;
  browserOpen?: boolean;
}

export const ActivityBar = memo(function ActivityBar({
  activeView,
  onViewChange,
  terminalOpen,
  onToggleTerminal,
  onOpenSettings,
  onOpenBrowser,
  browserOpen,
}: ActivityBarProps) {
  const { t } = useTranslation();
  return (
    <TooltipProvider delayDuration={200}>
      <div className="w-[48px] flex-shrink-0 h-full bg-card rounded-xl flex flex-col items-center overflow-hidden panel-float">
        {/* Home + Main nav items */}
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

            {ACTIVITY_ITEMS.map((item) => {
              const isActive = activeView === item.id;
              return (
                <Tooltip key={item.id}>
                  <TooltipTrigger asChild>
                    <button
                      type="button"
                      className={cn(
                        "relative w-10 h-10 flex items-center justify-center rounded-md",
                        "transition-colors cursor-pointer",
                        isActive
                          ? "text-foreground bg-[var(--bg-hover)]"
                          : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]",
                      )}
                      onClick={() => onViewChange(isActive ? null : item.id)}
                    >
                      <item.icon className="w-[18px] h-[18px]" />
                      {isActive && (
                        <span className="absolute left-0 top-1/2 -translate-y-1/2 w-[2px] h-5 bg-accent rounded-r" />
                      )}
                    </button>
                  </TooltipTrigger>
                  <TooltipContent side="right" sideOffset={8}>
                    <p className="text-xs">{t(item.label)}</p>
                  </TooltipContent>
                </Tooltip>
              );
            })}
          </div>

          {/* Bottom actions */}
          <div className="flex flex-col items-center gap-1 pb-3">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                className={cn(
                  "w-10 h-10 flex items-center justify-center rounded-md",
                  "transition-colors cursor-pointer",
                  browserOpen
                    ? "text-foreground bg-[var(--bg-hover)]"
                    : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]",
                )}
                onClick={onOpenBrowser}
              >
                <Globe className="w-[18px] h-[18px]" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              <p className="text-xs">{t("activity.browser")}</p>
            </TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                className={cn(
                  "w-10 h-10 flex items-center justify-center rounded-md",
                  "transition-colors cursor-pointer",
                  terminalOpen
                    ? "text-foreground bg-[var(--bg-hover)]"
                    : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]",
                )}
                onClick={onToggleTerminal}
              >
                <Terminal className="w-[18px] h-[18px]" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              <p className="text-xs">{t("activity.terminal")}</p>
            </TooltipContent>
          </Tooltip>
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
    </TooltipProvider>
  );
});
