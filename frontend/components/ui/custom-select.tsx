import { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

export interface CustomSelectOption {
  value: string;
  label: string;
}

interface CustomSelectProps {
  value: string;
  onChange: (value: string) => void;
  options: CustomSelectOption[];
  className?: string;
  placeholder?: string;
  size?: "xs" | "sm" | "default";
}

export function CustomSelect({ value, onChange, options, className, placeholder, size = "default" }: CustomSelectProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const selected = options.find((o) => o.value === value);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const sizeClasses = {
    xs: "h-5 px-1.5 text-[9px]",
    sm: "h-6 px-2 text-[10px]",
    default: "h-7 px-2.5 text-[10px]",
  };

  const itemSizeClasses = {
    xs: "px-1.5 py-1 text-[9px]",
    sm: "px-2 py-1 text-[10px]",
    default: "px-2.5 py-1.5 text-[10px]",
  };

  return (
    <div ref={ref} className={cn("relative", className)}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={cn(
          "flex items-center gap-1 w-full rounded-lg bg-white/[0.03] border border-white/[0.06] hover:border-white/[0.12] text-foreground/70 transition-colors cursor-pointer",
          sizeClasses[size],
        )}
      >
        <span className="flex-1 text-left truncate">
          {selected?.label ?? placeholder ?? "Select..."}
        </span>
        <ChevronDown className={cn("w-2.5 h-2.5 text-muted-foreground/30 transition-transform flex-shrink-0", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute z-50 mt-0.5 w-full min-w-[100px] rounded-lg border border-border/20 bg-popover shadow-xl py-0.5 max-h-[200px] overflow-y-auto">
          {options.map((o) => (
            <button
              key={o.value}
              type="button"
              onClick={() => { onChange(o.value); setOpen(false); }}
              className={cn(
                "w-full text-left transition-colors",
                itemSizeClasses[size],
                o.value === value
                  ? "bg-accent/15 text-accent"
                  : "text-foreground/60 hover:bg-white/[0.05] hover:text-foreground",
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
