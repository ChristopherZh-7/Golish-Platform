import { ChevronDown, ShieldCheck, Zap } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

type ApprovalMode = "ask" | "run-all";

const OPTIONS: { id: ApprovalMode; label: string; hint: string }[] = [
  { id: "ask", label: "Ask Every Time", hint: "Approve each tool call before it runs" },
  { id: "run-all", label: "Run Everything", hint: "Auto-run every tool without asking" },
];

/**
 * Persistent tool-approval mode switch for the chat toolbar.
 *
 * Unlike the per-tool-call dropdown (which only exists while a tool is pending
 * approval), this is ALWAYS visible — so choosing "Run Everything" is never a
 * one-way trap: in auto mode no approval card appears, yet the user can still
 * switch back to "Ask Every Time" here at any time (Cursor-style). The amber
 * styling in "Run Everything" keeps the auto-run state obvious at a glance.
 */
export function ApprovalModeSelector({
  approvalMode,
  onApprovalModeChange,
}: {
  approvalMode: string;
  onApprovalModeChange: (mode: ApprovalMode) => void;
}) {
  const isRunAll = approvalMode === "run-all";
  const Icon = isRunAll ? Zap : ShieldCheck;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Tool approval mode"
          className={cn(
            "flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium transition-colors",
            isRunAll
              ? "bg-[var(--ansi-yellow)]/10 text-[var(--ansi-yellow)] hover:bg-[var(--ansi-yellow)]/20"
              : "bg-muted text-foreground hover:bg-[var(--bg-hover)]"
          )}
        >
          <Icon className="w-3 h-3" />
          {isRunAll ? "Run Everything" : "Ask Every Time"}
          <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        side="top"
        className="bg-card border-[var(--border-medium)] min-w-[200px]"
      >
        {OPTIONS.map((opt) => (
          <DropdownMenuItem
            key={opt.id}
            onClick={() => onApprovalModeChange(opt.id)}
            className={cn(
              "flex-col items-start gap-0.5 text-xs cursor-pointer",
              approvalMode === opt.id && "bg-accent/10 text-accent"
            )}
          >
            <span className="flex w-full items-center">
              {opt.label}
              {approvalMode === opt.id && <span className="ml-auto text-accent">✓</span>}
            </span>
            <span className="text-[10px] text-muted-foreground/70">{opt.hint}</span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
