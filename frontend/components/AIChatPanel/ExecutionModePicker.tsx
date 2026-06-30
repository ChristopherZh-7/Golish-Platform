import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import type { LucideIcon } from "lucide-react";
import { ChevronDown, MessageSquare, Search, Settings, Users, Zap } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useState } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { type AgentMode, type ExecutionModeDescriptor, listExecutionModes } from "@/lib/ai";
import { cn } from "@/lib/utils";
import {
  pickTaskProfile,
  readLastProfile,
  resolveEngine,
  splitModes,
  writeLastProfile,
} from "./executionModePicker.utils";

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

const CHAT_DESCRIPTION = "Conversational single-agent mode with the full toolbox.";

/**
 * Seed list shown before (or instead of) the backend `list_execution_modes`
 * response. Mirrors the embedded profiles so the Task profile list is never
 * empty even when the IPC call is unavailable (e.g. mock / offline).
 */
const FALLBACK_MODES: ExecutionModeDescriptor[] = [
  {
    id: "chat",
    displayName: "Chat",
    icon: "MessageSquare",
    badgeColor: "muted",
    description: CHAT_DESCRIPTION,
    allowsSubAgents: false,
  },
  {
    id: "assessment",
    displayName: "Security Assessment",
    icon: "Zap",
    badgeColor: "green",
    description: "Passive + active recon",
    allowsSubAgents: true,
  },
  {
    id: "pentest",
    displayName: "Pentest",
    icon: "Zap",
    badgeColor: "blue",
    description: "Incl. controlled exploit validation",
    allowsSubAgents: true,
  },
  {
    id: "bug_bounty",
    displayName: "Bug Bounty",
    icon: "Zap",
    badgeColor: "muted",
    description: "Recon + vuln scanning",
    allowsSubAgents: true,
  },
  {
    id: "cloud_assessment",
    displayName: "Cloud Assessment",
    icon: "Zap",
    badgeColor: "muted",
    description: "Recon + vuln scanning",
    allowsSubAgents: true,
  },
  {
    id: "red_team",
    displayName: "Red Team",
    icon: "Zap",
    badgeColor: "magenta",
    description: "Full red team, incl. post-exploitation",
    allowsSubAgents: true,
  },
  {
    id: "smoke",
    displayName: "Smoke Test",
    icon: "Zap",
    badgeColor: "muted",
    description: "Passive intel only",
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

  const { chat, profiles } = useMemo(() => splitModes(modes), [modes]);

  const engine = resolveEngine(chatExecutionMode);
  const isTask = engine === "task";

  // Persistently remember the last Task profile: the header + Task row reflect
  // it, and switching into Task reuses it until the user changes it.
  const [lastProfileId, setLastProfileId] = useState<string | null>(() => readLastProfile());
  const activeProfileId = useMemo(() => {
    if (!isTask) return null;
    if (chatExecutionMode === "task") return pickTaskProfile(lastProfileId, profiles);
    return chatExecutionMode;
  }, [isTask, chatExecutionMode, lastProfileId, profiles]);
  const activeProfile = useMemo(
    () => profiles.find((p) => p.id === activeProfileId) ?? null,
    [profiles, activeProfileId]
  );

  // The gear's profile flyout opens on click only (not hover). Keeping the Radix
  // submenu controlled lets us suppress its default hover-to-open; it self-resets
  // to closed when the parent menu closes (see MenuSub effect in @radix-ui).
  const [subOpen, setSubOpen] = useState(false);
  const rememberProfile = useCallback((id: string) => {
    setLastProfileId(id);
    writeLastProfile(id);
  }, []);
  useEffect(() => {
    if (activeProfileId && activeProfile) rememberProfile(activeProfileId);
  }, [activeProfileId, activeProfile, rememberProfile]);
  useEffect(() => {
    if (chatExecutionMode !== "task" || !activeProfileId) return;
    rememberProfile(activeProfileId);
    onExecutionModeChange(activeProfileId);
    onAgentModeChange("auto-approve");
  }, [
    chatExecutionMode,
    activeProfileId,
    rememberProfile,
    onExecutionModeChange,
    onAgentModeChange,
  ]);

  const hintProfile = useMemo(
    () => activeProfile ?? profiles.find((p) => p.id === lastProfileId) ?? null,
    [activeProfile, profiles, lastProfileId]
  );

  const selectChat = useCallback(() => {
    if (chatExecutionMode === "chat") return;
    onExecutionModeChange("chat");
    onAgentModeChange("default");
  }, [chatExecutionMode, onExecutionModeChange, onAgentModeChange]);

  // Clicking the Task row itself enters Task using the remembered profile.
  const selectTask = useCallback(() => {
    const target = pickTaskProfile(lastProfileId, profiles);
    if (!target) return;
    rememberProfile(target);
    if (target === chatExecutionMode) return;
    onExecutionModeChange(target);
    onAgentModeChange("auto-approve");
  }, [
    lastProfileId,
    profiles,
    chatExecutionMode,
    onExecutionModeChange,
    onAgentModeChange,
    rememberProfile,
  ]);

  const selectProfile = useCallback(
    (id: string) => {
      rememberProfile(id);
      if (id === chatExecutionMode) return;
      onExecutionModeChange(id);
      onAgentModeChange("auto-approve");
    },
    [chatExecutionMode, onExecutionModeChange, onAgentModeChange, rememberProfile]
  );

  const TriggerIcon = isTask ? Zap : resolveIcon(chat?.icon ?? "MessageSquare");
  const triggerLabel = isTask
    ? (activeProfile?.displayName ?? "Task")
    : (chat?.displayName ?? "Chat");
  const triggerBadgeColor = isTask ? (activeProfile?.badgeColor ?? "magenta") : "muted";

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Execution mode"
          className={cn(
            "flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium transition-colors",
            resolveTriggerClass(triggerBadgeColor, isTask)
          )}
        >
          <TriggerIcon className="w-3 h-3" />
          {triggerLabel}
          <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        side="top"
        className="bg-card border-[var(--border-medium)] min-w-[220px]"
      >
        <DropdownMenuItem
          onSelect={selectChat}
          className={cn(
            "text-xs cursor-pointer flex items-center gap-2 py-2",
            !isTask ? resolveItemActiveClass("muted") : "text-foreground hover:text-accent"
          )}
        >
          <MessageSquare className="w-4 h-4 shrink-0" />
          <span className="font-medium">{chat?.displayName ?? "Chat"}</span>
        </DropdownMenuItem>

        {/* Task row: clicking the row enters Task with the remembered profile.
            The gear is a submenu trigger that flies the profile list out to the
            right; it is a separate hit target so configuring never commits Task. */}
        <div className="flex items-center gap-1">
          <DropdownMenuItem
            onSelect={selectTask}
            className={cn(
              "flex-1 text-xs cursor-pointer flex items-center gap-2 py-2",
              isTask
                ? resolveItemActiveClass(activeProfile?.badgeColor ?? "magenta")
                : "text-foreground hover:text-accent"
            )}
          >
            <Zap className="w-4 h-4 shrink-0" />
            <span className="font-medium">Task</span>
            {hintProfile && (
              <span className="text-muted-foreground font-normal">· {hintProfile.displayName}</span>
            )}
          </DropdownMenuItem>
          <DropdownMenuSub open={subOpen} onOpenChange={setSubOpen}>
            <DropdownMenuPrimitive.SubTrigger
              asChild
              // Click-only flyout: neutralize Radix's hover open/close so the
              // profile list appears on click, not on pointer hover.
              // composeEventHandlers skips the internal pointer handlers once the
              // event is default-prevented; onKeyDown is left intact for a11y.
              onPointerMove={(e) => e.preventDefault()}
              onPointerLeave={(e) => e.preventDefault()}
              onClick={(e) => {
                e.preventDefault();
                setSubOpen((open) => !open);
              }}
            >
              <button
                type="button"
                aria-label="Configure Task profile"
                className="mr-1 rounded p-1 text-muted-foreground outline-hidden transition-colors hover:bg-[var(--bg-hover)] hover:text-accent data-[state=open]:text-accent"
              >
                <Settings className="w-3.5 h-3.5" />
              </button>
            </DropdownMenuPrimitive.SubTrigger>
            <DropdownMenuSubContent className="bg-card border-[var(--border-medium)] min-w-[200px]">
              {profiles.map((profile) => (
                <DropdownMenuItem
                  key={profile.id}
                  onSelect={() => selectProfile(profile.id)}
                  className={cn(
                    "text-xs cursor-pointer flex items-center gap-2 py-2",
                    profile.id === activeProfileId
                      ? resolveItemActiveClass(profile.badgeColor)
                      : "text-foreground hover:text-accent"
                  )}
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <div className="flex w-full items-center gap-2">
                        <span className="font-medium">{profile.displayName}</span>
                      </div>
                    </TooltipTrigger>
                    {profile.description && (
                      <TooltipContent side="right" className="max-w-[240px]">
                        {profile.description}
                      </TooltipContent>
                    )}
                  </Tooltip>
                </DropdownMenuItem>
              ))}
            </DropdownMenuSubContent>
          </DropdownMenuSub>
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
});
