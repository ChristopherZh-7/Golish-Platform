import { Bot, Eye, MessageSquare, Shield, Users, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  setAgentMode as setAgentModeBackend,
  setUseAgents as setUseAgentsBackend,
  setExecutionMode as setExecutionModeBackend,
} from "@/lib/ai";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import { cn } from "@/lib/utils";
import {
  type AgentMode,
  type ExecutionMode,
  useAgentMode,
  useUseAgents,
  useExecutionMode,
  useStore,
} from "@/store";

interface AgentModeSelectorProps {
  sessionId: string;
  showLabel?: boolean;
}

const AGENT_MODES: {
  id: AgentMode;
  name: string;
  description: string;
  icon: React.ComponentType<{ className?: string }>;
}[] = [
  {
    id: "default",
    name: "Default",
    description: "Tool approval based on policy",
    icon: Shield,
  },
  {
    id: "auto-approve",
    name: "Auto-approve",
    description: "All tools automatically approved",
    icon: Zap,
  },
  {
    id: "planning",
    name: "Planning",
    description: "Read-only tools only",
    icon: Eye,
  },
];

export function AgentModeSelector({ sessionId, showLabel = true }: AgentModeSelectorProps) {
  const agentMode = useAgentMode(sessionId);
  const useAgents = useUseAgents(sessionId);
  const executionMode = useExecutionMode(sessionId);
  const setAgentMode = useStore((state) => state.setAgentMode);
  const setUseAgents = useStore((state) => state.setUseAgents);
  const setExecutionMode = useStore((state) => state.setExecutionMode);
  const workspace = useStore((state) => state.sessions[sessionId]?.workingDirectory);

  const currentMode = AGENT_MODES.find((m) => m.id === agentMode) ?? AGENT_MODES[0];
  const CurrentIcon = currentMode.icon;

  const handleModeSelect = async (mode: AgentMode) => {
    if (mode === agentMode) return;

    try {
      setAgentMode(sessionId, mode);
      await setAgentModeBackend(sessionId, mode, workspace);

      const modeName = AGENT_MODES.find((m) => m.id === mode)?.name ?? mode;
      notify.success(`Agent mode: ${modeName}`);
    } catch (error) {
      logger.error("Failed to set agent mode:", error);
      notify.error(`Failed to set agent mode: ${error}`);
      setAgentMode(sessionId, agentMode);
    }
  };

  const handleToggleAgents = async () => {
    const newValue = !useAgents;
    try {
      setUseAgents(sessionId, newValue);
      await setUseAgentsBackend(sessionId, newValue);
      notify.success(newValue ? "Sub-agents enabled" : "Sub-agents disabled");
    } catch (error) {
      logger.error("Failed to toggle useAgents:", error);
      notify.error(`Failed to toggle sub-agents: ${error}`);
      setUseAgents(sessionId, useAgents);
    }
  };

  const handleSetExecutionMode = async (mode: ExecutionMode) => {
    if (mode === executionMode) return;
    try {
      setExecutionMode(sessionId, mode);
      await setExecutionModeBackend(sessionId, mode);
      notify.success(mode === "task" ? "Task mode (auto)" : "Chat mode");
    } catch (error) {
      logger.error("Failed to set execution mode:", error);
      notify.error(`Failed to set execution mode: ${error}`);
      setExecutionMode(sessionId, executionMode);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className={cn(
            "h-6 px-2 text-xs font-medium rounded-lg transition-all duration-200 flex items-center",
            showLabel ? "gap-1.5" : "gap-0",
            "bg-muted/60 text-muted-foreground hover:bg-muted hover:text-foreground border border-transparent",
            agentMode === "auto-approve" &&
              "bg-[var(--ansi-yellow)]/10 text-[var(--ansi-yellow)] hover:bg-[var(--ansi-yellow)]/20 border-[var(--ansi-yellow)]/20 hover:border-[var(--ansi-yellow)]/30",
            agentMode === "planning" &&
              "bg-[var(--ansi-blue)]/10 text-[var(--ansi-blue)] hover:bg-[var(--ansi-blue)]/20 border-[var(--ansi-blue)]/20 hover:border-[var(--ansi-blue)]/30",
            executionMode === "task" &&
              "bg-[var(--ansi-magenta)]/10 text-[var(--ansi-magenta)] hover:bg-[var(--ansi-magenta)]/20 border-[var(--ansi-magenta)]/20 hover:border-[var(--ansi-magenta)]/30"
          )}
        >
          <CurrentIcon className="w-3.5 h-3.5" />
          <span
            className={cn(
              "transition-all duration-200 overflow-hidden whitespace-nowrap",
              showLabel ? "max-w-[100px] opacity-100" : "max-w-0 opacity-0"
            )}
          >
            {currentMode.name}
          </span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="bg-card border-[var(--border-medium)] min-w-[220px]"
      >
        {AGENT_MODES.map((mode) => {
          const Icon = mode.icon;
          return (
            <DropdownMenuItem
              key={mode.id}
              onClick={() => handleModeSelect(mode.id)}
              className={cn(
                "text-xs cursor-pointer flex items-start gap-2 py-2",
                agentMode === mode.id
                  ? "text-accent bg-[var(--accent-dim)]"
                  : "text-foreground hover:text-accent"
              )}
            >
              <Icon className="w-4 h-4 mt-0.5 shrink-0" />
              <div className="flex flex-col">
                <span className="font-medium">{mode.name}</span>
                <span className="text-[10px] text-muted-foreground">{mode.description}</span>
              </div>
            </DropdownMenuItem>
          );
        })}
        <DropdownMenuSeparator className="bg-[var(--border-medium)]" />
        <div className="px-2 py-1.5">
          <span className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider">
            Execution Mode
          </span>
        </div>
        <DropdownMenuItem
          onClick={() => handleSetExecutionMode("chat")}
          className={cn(
            "text-xs cursor-pointer flex items-start gap-2 py-2",
            executionMode === "chat"
              ? "text-accent bg-[var(--accent-dim)]"
              : "text-foreground hover:text-accent"
          )}
        >
          <MessageSquare className="w-4 h-4 mt-0.5 shrink-0" />
          <div className="flex flex-col">
            <span className="font-medium">Chat</span>
            <span className="text-[10px] text-muted-foreground">
              Conversational assistant with tools
            </span>
          </div>
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => handleSetExecutionMode("task")}
          className={cn(
            "text-xs cursor-pointer flex items-start gap-2 py-2",
            executionMode === "task"
              ? "text-[var(--ansi-magenta)] bg-[var(--ansi-magenta)]/10"
              : "text-foreground hover:text-accent"
          )}
        >
          <Bot className="w-4 h-4 mt-0.5 shrink-0" />
          <div className="flex flex-col">
            <span className="font-medium">Task (Auto)</span>
            <span className="text-[10px] text-muted-foreground">
              Automated: plan → execute → refine → report
            </span>
          </div>
        </DropdownMenuItem>
        <DropdownMenuSeparator className="bg-[var(--border-medium)]" />
        <DropdownMenuItem
          onClick={handleToggleAgents}
          className="text-xs cursor-pointer flex items-center gap-2 py-2"
        >
          <Users className={cn("w-4 h-4 shrink-0", useAgents ? "text-[var(--ansi-green)]" : "text-muted-foreground")} />
          <div className="flex flex-col flex-1">
            <span className="font-medium">Sub-Agents</span>
            <span className="text-[10px] text-muted-foreground">
              {useAgents ? "Enabled — AI can delegate to specialists" : "Disabled — direct tools only"}
            </span>
          </div>
          <div className={cn(
            "w-7 h-4 rounded-full transition-colors duration-200 flex items-center shrink-0",
            useAgents ? "bg-[var(--ansi-green)]/30 justify-end" : "bg-muted justify-start"
          )}>
            <div className={cn(
              "w-3 h-3 rounded-full mx-0.5 transition-colors duration-200",
              useAgents ? "bg-[var(--ansi-green)]" : "bg-muted-foreground/50"
            )} />
          </div>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
