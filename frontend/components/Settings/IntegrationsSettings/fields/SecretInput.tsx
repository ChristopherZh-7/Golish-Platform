/**
 * Single-line secret input with reveal toggle.
 *
 * Behaviour:
 * - Default `type="password"` so the value is masked.
 * - Clicking the eye icon toggles to `type="text"` and starts a 30-second
 *   countdown after which the value is re-masked automatically.
 * - Re-typing while revealed resets the countdown.
 * - `value === ""` + `placeholderForExistingSecret` (when the backend says
 *   `has_value=true`) communicates "configured, value hidden" without
 *   ever putting the secret into the DOM tree.
 */

import { Eye, EyeOff } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

const AUTO_HIDE_MS = 30_000;

interface SecretInputProps {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  disabled?: boolean;
  /** When provided, rendered as a watermark when value is empty AND
   * the backend reports the field as already configured. */
  placeholderForExistingSecret?: string;
  hasExistingSecret?: boolean;
  /** Aria-label / accessible name for the toggle button. */
  toggleLabel?: string;
  id?: string;
}

export function SecretInput({
  value,
  onChange,
  placeholder,
  disabled,
  placeholderForExistingSecret,
  hasExistingSecret,
  toggleLabel = "Toggle visibility",
  id,
}: SecretInputProps) {
  const [revealed, setRevealed] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearTimer = () => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  };

  const scheduleHide = () => {
    clearTimer();
    timerRef.current = setTimeout(() => setRevealed(false), AUTO_HIDE_MS);
  };

  useEffect(() => () => clearTimer(), []);

  useEffect(() => {
    if (revealed) {
      scheduleHide();
    } else {
      clearTimer();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [revealed, value]);

  const showWatermark = !revealed && value === "" && Boolean(placeholderForExistingSecret);
  const showConfiguredState = showWatermark || Boolean(hasExistingSecret && value === "");

  return (
    <div className="relative w-full">
      <input
        id={id}
        type={revealed ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={showWatermark ? placeholderForExistingSecret : placeholder}
        disabled={disabled}
        className={cn(
          "w-full px-2.5 py-1.5 pr-8 text-[11px] rounded-md border bg-background",
          "border-border/40 focus:border-accent outline-none transition-colors",
          "font-mono disabled:opacity-50",
          showConfiguredState &&
            "border-emerald-400/45 bg-emerald-500/10 text-emerald-100 placeholder:text-emerald-200/80"
        )}
      />
      <button
        type="button"
        onClick={() => setRevealed((v) => !v)}
        disabled={disabled}
        aria-label={toggleLabel}
        title={toggleLabel}
        className={cn(
          "absolute right-2 top-1/2 -translate-y-1/2",
          "text-muted-foreground/60 hover:text-foreground",
          "disabled:opacity-50"
        )}
      >
        {revealed ? <EyeOff className="w-3 h-3" /> : <Eye className="w-3 h-3" />}
      </button>
    </div>
  );
}
