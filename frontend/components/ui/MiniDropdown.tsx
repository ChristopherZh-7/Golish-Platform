import { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

type Variant = "compact" | "standard";

const btnStyles: Record<Variant, string> = {
  compact:
    "flex items-center gap-1 w-full px-1.5 py-[3px] text-[10px] rounded-md bg-white/[0.03] border border-white/[0.06] hover:border-white/[0.12] text-foreground/70 transition-colors",
  standard:
    "flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border transition-colors bg-[var(--bg-hover)]/30 border-border/30 text-foreground hover:bg-[var(--bg-hover)]/60 hover:border-border/50",
};

const menuStyles: Record<Variant, string> = {
  compact:
    "absolute z-50 mt-0.5 w-full min-w-[90px] rounded-md border border-border/20 bg-popover shadow-xl py-0.5 max-h-[180px] overflow-y-auto",
  standard:
    "absolute top-full left-0 mt-1 min-w-[120px] max-h-48 overflow-y-auto rounded-lg border border-border/30 bg-card shadow-lg z-50 py-1",
};

const chevronStyles: Record<Variant, string> = {
  compact: "w-2.5 h-2.5 text-muted-foreground/30 transition-transform",
  standard: "w-3 h-3 text-muted-foreground transition-transform",
};

interface MiniDropdownProps {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
  variant?: Variant;
  className?: string;
  buttonClassName?: string;
}

export function MiniDropdown({
  value,
  onChange,
  options,
  variant = "compact",
  className,
  buttonClassName,
}: MiniDropdownProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const selected = options.find((o) => o.value === value) ?? options[0];

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  return (
    <div ref={ref} className={cn("relative", className)}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={cn(btnStyles[variant], open && variant === "standard" && "border-accent/40 bg-[var(--bg-hover)]/50", buttonClassName)}
      >
        <span className="flex-1 text-left truncate">{selected?.label ?? value}</span>
        <ChevronDown className={cn(chevronStyles[variant], open && "rotate-180")} />
      </button>
      {open && (
        <div className={menuStyles[variant]}>
          {options.map((o) => (
            <button
              key={o.value}
              type="button"
              onClick={() => {
                onChange(o.value);
                setOpen(false);
              }}
              className={cn(
                "w-full text-left transition-colors",
                variant === "compact"
                  ? cn(
                      "px-2 py-1 text-[10px]",
                      o.value === value ? "bg-accent/15 text-accent" : "text-foreground/60 hover:bg-white/[0.05] hover:text-foreground",
                    )
                  : cn(
                      "px-3 py-1.5 text-[10px] hover:bg-[var(--bg-hover)]/60",
                      o.value === value && "text-accent bg-accent/5 font-medium",
                    ),
              )}
            >
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
