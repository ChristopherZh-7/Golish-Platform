import type { LucideIcon } from "lucide-react";
import { ChevronDown, MessageSquare, Search, Users, Zap } from "lucide-react";
import { memo, useEffect, useMemo, useState } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { type AgentMode, type ExecutionModeDescriptor, listExecutionModes } from "@/lib/ai";
import { cn } from "@/lib/utils";

const ICON_MAP: Record<string, LucideIcon> = {
  MessageSquare,
  Zap,
  Search,
  Users,
};

const BADGE_TRIGGER_CLASSES: Record<string, string> = {
  magenta:
    "bg-[var(--ansi-magenta)]/10 text-[var(--ansi-magenta)] hover:bg-[var(--ansi-magenta)]/20",
  green: "bg-[var(--ansi-green)]/10 text-[var(--ansi-green)] hover:bg-[var(--ansi-green)]/20",
  blue: "bg-[var(--ansi-blue)]/10 text-[var(--ansi-blue)] hover:bg-[var(--ansi-blue)]/20",
  muted: "bg-muted text-foreground hover:bg-[var(--bg-hover)]",
};

const BADGE_ITEM_ACTIVE_CLASSES: Record<string, string> = {
  magenta: "text-[var(--ansi-magenta)] bg-[var(--ansi-magenta)]/10",
  green: "text-[var(--ansi-green)] bg-[var(--ansi-green)]/10",
  blue: "text-[var(--ansi-blue)] bg-[var(--ansi-blue)]/10",
  muted: "text-accent bg-[var(--accent-dim)]",
};

const FALLBACK_MODES: ExecutionModeDescriptor[] = [
  {
    id: "chat",
    displayName: "Chat",
    icon: "MessageSquare",
    badgeColor: "muted",
    description: "Conversational single-agent mode with the full toolbox.",
    allowsSubAgents: false,
  },
  {
    id: "task",
    displayName: "Task",
    icon: "Zap",
    badgeColor: "magenta",
    description:
      "Auto: plan \u2192 execute \u2192 refine \u2192 report (multi-agent orchestration).",
    allowsSubAgents: true,
  },
];

function resolveIcon(name: string): LucideIcon {
  return ICON_MAP[name] ?? MessageSquare;
}

function resolveTriggerClass(badgeColor: string, isActive: boolean): string {
  if (!isActive) return BADGE_TRIGGER_CLASSES.muted;
  return BADGE_TRIGGER_CLASSES[badgeColor] ?? BADGE_TRIGGER_CLASSES.muted;
}

function resolveItemActiveClass(badgeColor: string): string {
  return BADGE_ITEM_ACTIVE_CLASSES[badgeColor] ?? BADGE_ITEM_ACTIVE_CLASSES.muted;
}

interface ExecutionModePickerProps {
  chatExecutionMode: string;
  onExecutionModeChange: (mode: string) => void;
  onAgentModeChange: (mode: AgentMode) => void;
}

export const ExecutionModePicker = memo(function ExecutionModePicker({
  chatExecutionMode,
  onExecutionModeChange,
  onAgentModeChange,
}: ExecutionModePickerProps) {
  const [modes, setModes] = useState<ExecutionModeDescriptor[]>(FALLBACK_MODES);

  useEffect(() => {
    let cancelled = false;
    listExecutionModes()
      .then((fetched) => {
        if (cancelled) return;
        if (fetched.length > 0) {
          setModes(fetched);
        }
      })
      .catch((err) => {
        console.warn(
          "[ExecutionModePicker] list_execution_modes failed; keeping fallback list",
          err
        );
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const activeMode = useMemo(
    () => modes.find((m) => m.id === chatExecutionMode) ?? modes[0],
    [modes, chatExecutionMode]
  );
  const ActiveIcon = resolveIcon(activeMode?.icon ?? "MessageSquare");

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className={cn(
            "flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium transition-colors",
            resolveTriggerClass(activeMode?.badgeColor ?? "muted", chatExecutionMode !== "chat")
          )}
        >
          <ActiveIcon className="w-3 h-3" />
          {activeMode?.displayName ?? chatExecutionMode}
          <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        side="top"
        className="bg-card border-[var(--border-medium)] min-w-[220px]"
      >
        {modes.map((mode) => {
          const Icon = resolveIcon(mode.icon);
          const isActive = mode.id === chatExecutionMode;
          return (
            <DropdownMenuItem
              key={mode.id}
              onClick={() => {
                onExecutionModeChange(mode.id);
                onAgentModeChange(mode.allowsSubAgents ? "auto-approve" : "default");
              }}
              className={cn(
                "text-xs cursor-pointer flex items-center gap-2 py-2",
                isActive
                  ? resolveItemActiveClass(mode.badgeColor)
                  : "text-foreground hover:text-accent"
              )}
            >
              <Tooltip>
                <TooltipTrigger asChild>
                  <div className="flex w-full items-center gap-2">
                    <Icon className="w-4 h-4 shrink-0" />
                    <span className="font-medium">{mode.displayName}</span>
                  </div>
                </TooltipTrigger>
                {mode.description && (
                  <TooltipContent side="right" className="max-w-[240px]">
                    {mode.description}
                  </TooltipContent>
                )}
              </Tooltip>
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
});
