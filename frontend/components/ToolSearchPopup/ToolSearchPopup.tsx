import { Download, Play, Terminal } from "lucide-react";
import { memo, useEffect, useRef } from "react";
import type { ToolConfig } from "@/lib/pentest/types";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";

const RUNTIME_COLORS: Record<string, string> = {
  python: "text-[#3b82f6] border-[#3b82f6]/30",
  java: "text-[#f97316] border-[#f97316]/30",
  node: "text-[#22c55e] border-[#22c55e]/30",
  system: "text-[#a78bfa] border-[#a78bfa]/30",
};

const ToolSearchItem = memo(function ToolSearchItem({
  tool,
  index,
  isSelected,
  onSelect,
}: {
  tool: ToolConfig;
  index: number;
  isSelected: boolean;
  onSelect: (tool: ToolConfig) => void;
}) {
  const { t } = useTranslation();
  const notInstalled = !tool.installed;
  const noEnv = tool.installed && !tool.envReady;
  const disabled = notInstalled || noEnv;
  const reason = notInstalled ? t("toolSearch.notInstalled") : noEnv ? t("toolSearch.noEnv") : null;

  return (
    <div
      role="option"
      aria-selected={isSelected}
      aria-disabled={disabled}
      data-index={index}
      onClick={() => { if (!disabled) onSelect(tool); }}
      className={cn(
        "flex items-center gap-3 px-3 py-2 transition-colors",
        disabled
          ? "opacity-40 cursor-not-allowed"
          : cn("cursor-pointer", isSelected ? "bg-primary/10" : "hover:bg-card"),
      )}
    >
      <div className="w-7 h-7 rounded-md bg-[var(--bg-hover)] flex items-center justify-center flex-shrink-0">
        <Terminal className="w-3.5 h-3.5 text-muted-foreground" />
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-foreground truncate">{tool.name}</span>
          <span
            className={cn(
              "text-[10px] px-1.5 py-0 border rounded-full leading-[18px]",
              RUNTIME_COLORS[tool.runtime] || RUNTIME_COLORS.system,
            )}
          >
            {tool.runtime}
          </span>
          {reason && (
            <span className="text-[10px] px-1.5 py-0 border border-yellow-500/30 text-yellow-500 rounded-full leading-[18px]">
              {reason}
            </span>
          )}
        </div>
        {tool.description && (
          <p className="text-xs text-muted-foreground truncate mt-0.5">{tool.description}</p>
        )}
      </div>
      <div className="flex-shrink-0">
        {tool.installed ? (
          <Play className="w-3.5 h-3.5 text-muted-foreground" />
        ) : (
          <Download className="w-3.5 h-3.5 text-muted-foreground" />
        )}
      </div>
    </div>
  );
});

interface ToolSearchPopupProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  tools: ToolConfig[];
  selectedIndex: number;
  onSelect: (tool: ToolConfig) => void;
  containerRef: React.RefObject<HTMLElement | null>;
}

export function ToolSearchPopup({
  open,
  onOpenChange,
  tools,
  selectedIndex,
  onSelect,
  containerRef,
}: ToolSearchPopupProps) {
  const { t } = useTranslation();
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        onOpenChange(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside, true);
    return () => document.removeEventListener("mousedown", handleClickOutside, true);
  }, [open, onOpenChange, containerRef]);

  useEffect(() => {
    if (!open) return;
    const handleBlur = () => onOpenChange(false);
    window.addEventListener("blur", handleBlur);
    return () => window.removeEventListener("blur", handleBlur);
  }, [open, onOpenChange]);

  useEffect(() => {
    if (open && listRef.current) {
      const el = listRef.current.querySelector(`[data-index="${selectedIndex}"]`);
      el?.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIndex, open]);

  if (!open || tools.length === 0) return null;

  return (
    <div
      ref={listRef}
      className="absolute bottom-full left-0 mb-2 w-[400px] z-50 bg-popover border border-border rounded-lg shadow-lg overflow-hidden"
    >
      <div className="px-3 py-1.5 border-b border-border/50">
        <span className="text-[10px] text-muted-foreground uppercase tracking-wider font-medium">
          {t("toolSearch.matchedTools", { count: tools.length })}
        </span>
      </div>
      <div className="max-h-[300px] overflow-y-auto py-1" role="listbox">
        {tools.map((tool, i) => (
          <ToolSearchItem
            key={tool.id}
            tool={tool}
            index={i}
            isSelected={i === selectedIndex}
            onSelect={onSelect}
          />
        ))}
      </div>
      <div className="px-3 py-1.5 border-t border-border/50 flex items-center gap-3 text-[10px] text-muted-foreground">
        <span>{t("toolSearch.keySelect")}</span>
        <span>{t("toolSearch.keyRun")}</span>
        <span>{t("toolSearch.keyClose")}</span>
      </div>
    </div>
  );
}
