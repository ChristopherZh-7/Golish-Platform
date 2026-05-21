/**
 * Composite proxy field.
 *
 * The wire-format stores a single string in the URL form
 * `<scheme>://[user:pass@]host[:port]` (matching the existing
 * `network.proxy_url` setting). We render it as a single input for
 * now — full host / port / auth split-form is a Phase 5+ upgrade
 * once we have a real schema that uses it.
 *
 * Until then this behaves like a non-secret URL input plus a
 * "validate looks like a URL" hint in the placeholder. Auth secrets
 * inside the URL are not masked; the schema should mark the field as
 * `secret_text` if those need protection.
 */

import { cn } from "@/lib/utils";

interface ProxyInputProps {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  disabled?: boolean;
  id?: string;
}

export function ProxyInput({
  value,
  onChange,
  placeholder = "http://user:pass@host:port",
  disabled,
  id,
}: ProxyInputProps) {
  return (
    <input
      id={id}
      type="text"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      disabled={disabled}
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
