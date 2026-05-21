/**
 * Plain single-line text input for non-secret string fields.
 * Used for `text` / `url` / `port` field types (URL / port get
 * additional validation hints via the parent component).
 */

import { cn } from "@/lib/utils";

interface TextInputProps {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  disabled?: boolean;
  type?: "text" | "url" | "number";
  inputMode?: "numeric" | "url" | "text";
  pattern?: string;
  min?: number;
  max?: number;
  id?: string;
}

export function TextInput({
  value,
  onChange,
  placeholder,
  disabled,
  type = "text",
  inputMode,
  pattern,
  min,
  max,
  id,
}: TextInputProps) {
  return (
    <input
      id={id}
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      disabled={disabled}
      inputMode={inputMode}
      pattern={pattern}
      min={min}
      max={max}
      spellCheck={false}
      autoComplete="off"
      className={cn(
        "w-full px-2.5 py-1.5 text-[11px] rounded-md border bg-background",
        "border-border/40 focus:border-accent outline-none transition-colors",
        "font-mono disabled:opacity-50"
      )}
    />
  );
}
