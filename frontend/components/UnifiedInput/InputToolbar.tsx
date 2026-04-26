import { SendHorizontal } from "lucide-react";
import type React from "react";
import type { ToolConfig } from "@/lib/pentest/types";
import { cn } from "@/lib/utils";
import type { ToolParam } from "./useUnifiedInputState";

// ── Tool Params Panel (above the input row) ──

interface ToolParamsPanelProps {
  activeTool: ToolConfig | null;
  toolParams: ToolParam[];
  input: string;
  setInput: (v: string) => void;
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
}

export function ToolParamsPanel({
  activeTool,
  toolParams,
  input,
  setInput,
  textareaRef,
}: ToolParamsPanelProps) {
  if (!activeTool || toolParams.length === 0) return null;

  return (
    <div className="px-3 py-1.5 border-b border-[var(--border-subtle)] flex flex-wrap gap-1">
      {toolParams.map((p) => {
        const escapedFlag = p.flag.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const alreadyUsed = new RegExp(`(^|\\s)${escapedFlag}(\\s|$)`).test(input);
        return (
          <button
            key={p.flag}
            type="button"
            className={cn(
              "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] transition-colors cursor-pointer",
              alreadyUsed
                ? "bg-accent/20 text-accent border border-accent/30 hover:bg-destructive/20 hover:text-destructive hover:border-destructive/30"
                : p.required
                  ? "bg-accent/15 text-accent border border-accent/30 hover:bg-accent/25"
                  : "bg-muted/40 text-muted-foreground hover:bg-muted/60",
            )}
            onClick={() => {
              const ta = textareaRef.current;
              if (!ta) return;

              if (alreadyUsed) {
                let newInput = input;
                if (p.type === "boolean") {
                  newInput = newInput.replace(
                    new RegExp(
                      `\\s*${p.flag.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`,
                    ),
                    "",
                  );
                } else {
                  newInput = newInput.replace(
                    new RegExp(
                      `\\s*${p.flag.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s+\\S*`,
                    ),
                    "",
                  );
                }
                setInput(newInput.trim());
                requestAnimationFrame(() => {
                  ta.focus();
                });
                return;
              }

              const insert = p.type === "boolean" ? p.flag : `${p.flag} `;
              const pos = ta.selectionStart ?? input.length;
              const before = input.slice(0, pos);
              const after = input.slice(pos);
              const spaceBefore = before.length > 0 && !before.endsWith(" ") ? " " : "";
              const spaceAfter = after.length > 0 && !after.startsWith(" ") ? " " : "";
              const newInput = `${before}${spaceBefore}${insert}${spaceAfter}${after}`;
              setInput(newInput);
              requestAnimationFrame(() => {
                const newPos = (before + spaceBefore + insert).length;
                ta.focus();
                ta.setSelectionRange(newPos, newPos);
              });
            }}
            title={p.description || p.label}
          >
            <span className="font-mono font-medium">{p.flag}</span>
            <span className="opacity-70">{p.label}</span>
            {p.required && !alreadyUsed && <span className="text-accent">*</span>}
            {alreadyUsed && <span className="opacity-50">{"✓"}</span>}
          </button>
        );
      })}
    </div>
  );
}

// ── Input Badges (inline before textarea) ──

interface InputBadgesProps {
  isToolSearchMode: boolean;
  activeTool: ToolConfig | null;
  setInput: (v: string) => void;
  clearToolMode: () => void;
  toolSearchTitle: string;
}

export function InputBadges({
  isToolSearchMode,
  activeTool,
  setInput,
  clearToolMode,
  toolSearchTitle,
}: InputBadgesProps) {
  return (
    <>
      {isToolSearchMode && !activeTool && (
        <div className="flex items-center gap-1 h-[26px] px-2 rounded-md bg-orange-500/15 border border-orange-500/30 shrink-0 self-center">
          <span className="text-[12px] font-medium text-orange-400 leading-none">
            {toolSearchTitle}
          </span>
          <button
            type="button"
            className="w-3.5 h-3.5 flex items-center justify-center rounded-full hover:bg-orange-500/20 text-orange-400/60 hover:text-orange-400 transition-colors"
            onClick={() => setInput("")}
          >
            <span className="text-[10px] leading-none">{"×"}</span>
          </button>
        </div>
      )}

      {activeTool && (
        <div className="flex items-center gap-1.5 h-[26px] px-2 rounded-md bg-accent/15 border border-accent/30 shrink-0 self-center">
          <div className="w-4 h-4 rounded bg-accent/20 flex items-center justify-center">
            <span className="text-[9px] font-bold text-accent">
              {activeTool.runtime === "python"
                ? "Py"
                : activeTool.runtime === "java"
                  ? "Jv"
                  : activeTool.runtime === "node"
                    ? "Js"
                    : "\u2318"}
            </span>
          </div>
          <span className="text-[13px] font-medium text-accent leading-none">
            {activeTool.name}
          </span>
          <button
            type="button"
            className="w-3.5 h-3.5 flex items-center justify-center rounded-full hover:bg-accent/20 text-accent/60 hover:text-accent transition-colors"
            onClick={() => {
              clearToolMode();
              setInput("");
            }}
          >
            {"×"}
          </button>
        </div>
      )}
    </>
  );
}

// ── Send Button ──

interface SendButtonProps {
  onSubmit: () => void;
  disabled: boolean;
}

export function SendButton({ onSubmit, disabled }: SendButtonProps) {
  return (
    <button
      type="button"
      data-testid="send-button"
      onClick={onSubmit}
      disabled={disabled}
      className={cn(
        "h-7 w-7 flex items-center justify-center self-center rounded-md shrink-0",
        "transition-all duration-150",
        !disabled
          ? "text-foreground hover:text-foreground/70"
          : "text-muted-foreground/40 cursor-not-allowed",
      )}
    >
      <SendHorizontal className="w-3.5 h-3.5" />
    </button>
  );
}
