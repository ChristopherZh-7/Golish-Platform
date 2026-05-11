import { ChevronDown, KeyRound, ShieldAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { VaultEntrySafe } from "@/lib/security";
import { cn } from "@/lib/utils";

export function PolicyDropdown({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
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

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        className={cn(
          "flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border transition-colors",
          "bg-[var(--bg-hover)]/20 border-border/20 text-muted-foreground/60",
          "hover:bg-[var(--bg-hover)]/40 hover:border-border/40 hover:text-muted-foreground/80",
          open && "border-accent/30 bg-[var(--bg-hover)]/30"
        )}
        onClick={() => setOpen(!open)}
      >
        <ShieldAlert className="w-3 h-3 flex-shrink-0" />
        <span className="truncate max-w-[120px]">{selected?.label ?? value}</span>
        <ChevronDown
          className={cn(
            "w-2.5 h-2.5 text-muted-foreground/40 transition-transform",
            open && "rotate-180"
          )}
        />
      </button>
      {open && (
        <div className="absolute top-full left-0 mt-1 min-w-[180px] max-h-48 overflow-y-auto rounded-lg border border-border/30 bg-card shadow-lg z-50 py-1">
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              className={cn(
                "w-full text-left px-3 py-1.5 text-[10px] transition-colors",
                "hover:bg-[var(--bg-hover)]/60",
                opt.value === value && "text-accent bg-accent/5 font-medium"
              )}
              onClick={() => {
                onChange(opt.value);
                setOpen(false);
              }}
            >
              {opt.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function CredentialDropdown({
  value,
  onChange,
  open,
  onToggle,
  entries,
}: {
  value: string;
  onChange: (v: string) => void;
  open: boolean;
  onToggle: () => void;
  entries: VaultEntrySafe[];
}) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  const selected = entries.find((e) => e.id === value);
  const filtered = entries.filter((e) =>
    ["password", "token", "cookie", "api_key"].includes(e.type)
  );

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onToggle();
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open, onToggle]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        className={cn(
          "flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border transition-colors",
          "bg-[var(--bg-hover)]/20 border-border/20 text-muted-foreground/60",
          "hover:bg-[var(--bg-hover)]/40 hover:border-border/40 hover:text-muted-foreground/80",
          open && "border-accent/30 bg-[var(--bg-hover)]/30"
        )}
        onClick={onToggle}
      >
        <KeyRound className="w-3 h-3 flex-shrink-0" />
        <span className="truncate max-w-[140px]">
          {selected ? selected.name : t("security.noCredential")}
        </span>
        <ChevronDown
          className={cn(
            "w-2.5 h-2.5 text-muted-foreground/40 transition-transform",
            open && "rotate-180"
          )}
        />
      </button>
      {open && (
        <div className="absolute top-full left-0 mt-1 min-w-[200px] max-h-48 overflow-y-auto rounded-lg border border-border/30 bg-card shadow-lg z-50 py-1">
          <button
            type="button"
            className={cn(
              "w-full text-left px-3 py-1.5 text-[10px] transition-colors hover:bg-[var(--bg-hover)]/60",
              !value && "text-accent bg-accent/5 font-medium"
            )}
            onClick={() => {
              onChange("");
              onToggle();
            }}
          >
            {t("security.noCredential")}
          </button>
          {filtered.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className={cn(
                "w-full text-left px-3 py-1.5 text-[10px] transition-colors hover:bg-[var(--bg-hover)]/60 flex items-center gap-2",
                value === entry.id && "text-accent bg-accent/5 font-medium"
              )}
              onClick={() => {
                onChange(entry.id);
                onToggle();
              }}
            >
              <KeyRound className="w-2.5 h-2.5 flex-shrink-0" />
              <span className="truncate">{entry.name}</span>
              <span className="text-muted-foreground/40 text-[8px] ml-auto">{entry.type}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
