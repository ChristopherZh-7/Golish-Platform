/**
 * Dropdown for `select`-type fields. Options come from the schema.
 */

import type { SelectOption } from "@/lib/api/integrations";
import { cn } from "@/lib/utils";

interface SelectFieldProps {
  value: string;
  onChange: (next: string) => void;
  options: SelectOption[];
  disabled?: boolean;
  placeholder?: string;
  id?: string;
}

export function SelectField({
  value,
  onChange,
  options,
  disabled,
  placeholder,
  id,
}: SelectFieldProps) {
  return (
    <select
      id={id}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      disabled={disabled}
      className={cn(
        "w-full px-2.5 py-1.5 text-[11px] rounded-md border bg-background",
        "border-border/40 focus:border-accent outline-none transition-colors",
        "disabled:opacity-50"
      )}
    >
      {placeholder && (
        <option value="" disabled>
          {placeholder}
        </option>
      )}
      {options.map((opt) => (
        <option key={opt.value} value={opt.value}>
          {opt.label}
        </option>
      ))}
    </select>
  );
}
