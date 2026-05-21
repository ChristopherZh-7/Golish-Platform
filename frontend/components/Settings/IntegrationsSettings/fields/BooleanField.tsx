/**
 * Checkbox-style boolean field. Stores `"true"` / `"false"` as the
 * underlying string value to stay compatible with the
 * `Record<string, string>` wire format for `integrations_set`.
 */

import { cn } from "@/lib/utils";

interface BooleanFieldProps {
  value: string;
  onChange: (next: string) => void;
  label?: string;
  disabled?: boolean;
  id?: string;
}

export function BooleanField({ value, onChange, label, disabled, id }: BooleanFieldProps) {
  const checked = value === "true" || value === "1" || value === "yes";
  return (
    <label
      htmlFor={id}
      className={cn(
        "inline-flex items-center gap-2 text-[11px] text-foreground/80",
        "select-none cursor-pointer",
        disabled && "opacity-50 cursor-not-allowed"
      )}
    >
      <input
        id={id}
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked ? "true" : "false")}
        disabled={disabled}
        className="h-3 w-3 rounded border-border/40 accent-accent"
      />
      {label && <span>{label}</span>}
    </label>
  );
}
