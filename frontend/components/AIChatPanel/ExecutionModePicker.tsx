import { ChevronDown, MessageSquare, Users, Zap } from "lucide-react";
import { memo } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { AgentMode } from "@/lib/ai";
import { cn } from "@/lib/utils";

interface ExecutionModePickerProps {
  chatExecutionMode: "chat" | "task";
  chatUseSubAgents: boolean;
  onExecutionModeChange: (mode: "chat" | "task") => void;
  onAgentModeChange: (mode: AgentMode) => void;
  onToggleSubAgents: () => void;
}

export const ExecutionModePicker = memo(function ExecutionModePicker({
  chatExecutionMode,
  chatUseSubAgents,
  onExecutionModeChange,
  onAgentModeChange,
  onToggleSubAgents,
}: ExecutionModePickerProps) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className={cn(
            "flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium transition-colors",
            chatExecutionMode === "task"
              ? "bg-[var(--ansi-magenta)]/10 text-[var(--ansi-magenta)] hover:bg-[var(--ansi-magenta)]/20"
              : "bg-muted text-foreground hover:bg-[var(--bg-hover)]"
          )}
        >
          {chatExecutionMode === "task" ? (
            <Zap className="w-3 h-3" />
          ) : (
            <MessageSquare className="w-3 h-3" />
          )}
          {chatExecutionMode === "task" ? "Task" : "Chat"}
          <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        side="top"
        className="bg-card border-[var(--border-medium)] min-w-[220px]"
      >
        <DropdownMenuItem
          onClick={() => {
            onExecutionModeChange("chat");
            onAgentModeChange("default");
          }}
          className={cn(
            "text-xs cursor-pointer flex items-start gap-2 py-2.5",
            chatExecutionMode === "chat"
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
          onClick={() => {
            onExecutionModeChange("task");
            onAgentModeChange("auto-approve");
          }}
          className={cn(
            "text-xs cursor-pointer flex items-start gap-2 py-2.5",
            chatExecutionMode === "task"
              ? "text-[var(--ansi-magenta)] bg-[var(--ansi-magenta)]/10"
              : "text-foreground hover:text-accent"
          )}
        >
          <Zap className="w-4 h-4 mt-0.5 shrink-0" />
          <div className="flex flex-col">
            <span className="font-medium">Task</span>
            <span className="text-[10px] text-muted-foreground">
              Auto: plan &rarr; execute &rarr; refine &rarr; report
            </span>
          </div>
        </DropdownMenuItem>
        <DropdownMenuSeparator className="bg-[var(--border-medium)]" />
        <DropdownMenuItem
          disabled={chatExecutionMode === "chat"}
          onSelect={(e) => {
            e.preventDefault();
            if (chatExecutionMode === "chat") return;
            onToggleSubAgents();
          }}
          className={cn(
            "text-xs flex items-center gap-2 py-2",
            chatExecutionMode === "chat" ? "cursor-not-allowed opacity-60" : "cursor-pointer"
          )}
          title={
            chatExecutionMode === "chat" ? "Sub-Agents are only available in Task mode" : undefined
          }
        >
          <Users
            className={cn(
              "w-4 h-4 shrink-0",
              chatExecutionMode === "chat"
                ? "text-muted-foreground/50"
                : chatUseSubAgents
                  ? "text-[var(--ansi-green)]"
                  : "text-muted-foreground"
            )}
          />
          <div className="flex flex-col flex-1">
            <span className="font-medium">Sub-Agents</span>
            <span className="text-[10px] text-muted-foreground">
              {chatExecutionMode === "chat"
                ? "Switch to Task mode to enable"
                : chatUseSubAgents
                  ? "Enabled"
                  : "Disabled"}
            </span>
          </div>
          <div
            className={cn(
              "w-7 h-4 rounded-full transition-colors duration-200 flex items-center shrink-0",
              chatExecutionMode === "chat"
                ? "bg-muted/50 justify-start"
                : chatUseSubAgents
                  ? "bg-[var(--ansi-green)]/30 justify-end"
                  : "bg-muted justify-start"
            )}
          >
            <div
              className={cn(
                "w-3 h-3 rounded-full mx-0.5 transition-colors duration-200",
                chatExecutionMode === "chat"
                  ? "bg-muted-foreground/30"
                  : chatUseSubAgents
                    ? "bg-[var(--ansi-green)]"
                    : "bg-muted-foreground/50"
              )}
            />
          </div>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
});
