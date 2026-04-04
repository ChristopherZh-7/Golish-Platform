import { memo } from "react";
import {
  Shield,
  Search,
  FolderTree,
  Database,
  BookOpen,
  Settings,
  Terminal,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

export type ActivityView = "tools" | "search" | "explorer" | "database" | "wiki" | "settings";

interface ActivityBarItem {
  id: ActivityView;
  icon: LucideIcon;
  label: string;
}

const ACTIVITY_ITEMS: ActivityBarItem[] = [
  { id: "tools", icon: Shield, label: "渗透工具" },
  { id: "search", icon: Search, label: "搜索" },
  { id: "explorer", icon: FolderTree, label: "文件浏览" },
  { id: "database", icon: Database, label: "数据库" },
  { id: "wiki", icon: BookOpen, label: "知识库" },
];

interface ActivityBarProps {
  activeView: ActivityView;
  onViewChange: (view: ActivityView) => void;
  terminalOpen?: boolean;
  onToggleTerminal?: () => void;
  onOpenSettings?: () => void;
}

export const ActivityBar = memo(function ActivityBar({
  activeView,
  onViewChange,
  terminalOpen,
  onToggleTerminal,
  onOpenSettings,
}: ActivityBarProps) {
  return (
    <TooltipProvider delayDuration={200}>
      <div className="w-[48px] flex-shrink-0 h-full bg-card rounded-xl flex flex-col items-center overflow-hidden panel-float">
        {/* Main nav items */}
        <div className="flex flex-col items-center gap-1 pt-3 flex-1">
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
                      onClick={() => onViewChange(item.id)}
                    >
                      <item.icon className="w-[18px] h-[18px]" />
                      {isActive && (
                        <span className="absolute left-0 top-1/2 -translate-y-1/2 w-[2px] h-5 bg-accent rounded-r" />
                      )}
                    </button>
                  </TooltipTrigger>
                  <TooltipContent side="right" sideOffset={8}>
                    <p className="text-xs">{item.label}</p>
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
              <p className="text-xs">终端</p>
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
                onClick={() => onViewChange("settings")}
              >
                <Settings className="w-[18px] h-[18px]" />
                {activeView === "settings" && (
                  <span className="absolute left-0 top-1/2 -translate-y-1/2 w-[2px] h-5 bg-accent rounded-r" />
                )}
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              <p className="text-xs">设置</p>
            </TooltipContent>
          </Tooltip>
        </div>
      </div>
    </TooltipProvider>
  );
});
