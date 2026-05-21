/**
 * Multi-line secret input (cookies, certificates, multi-value tokens).
 *
 * Same reveal-with-auto-hide behaviour as `SecretInput`. The textarea
 * always renders the value, but a CSS filter blurs it when masked so
 * the user can still see line breaks / formatting hints without
 * exposing the secret to over-the-shoulder eyes.
 */

import { Eye, EyeOff } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

const AUTO_HIDE_MS = 30_000;

interface SecretTextareaProps {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  disabled?: boolean;
  rows?: number;
  placeholderForExistingSecret?: string;
  hasExistingSecret?: boolean;
  toggleLabel?: string;
  id?: string;
}

export function SecretTextarea({
  value,
  onChange,
  placeholder,
  disabled,
  rows = 4,
  placeholderForExistingSecret,
  hasExistingSecret,
  toggleLabel = "Toggle visibility",
  id,
}: SecretTextareaProps) {
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
    if (revealed) scheduleHide();
    else clearTimer();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [revealed, value]);

  const showWatermark = !revealed && value === "" && Boolean(placeholderForExistingSecret);
  const showConfiguredState = showWatermark || Boolean(hasExistingSecret && value === "");

  return (
    <div className="relative w-full">
      <textarea
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={showWatermark ? placeholderForExistingSecret : placeholder}
        disabled={disabled}
        rows={rows}
        spellCheck={false}
        autoComplete="off"
        className={cn(
          "w-full px-2.5 py-2 pr-8 text-[11px] rounded-md border bg-background",
          "border-border/40 focus:border-accent outline-none transition-colors",
          "font-mono leading-relaxed resize-y",
          "disabled:opacity-50",
          showConfiguredState &&
            "border-emerald-400/45 bg-emerald-500/10 text-emerald-100 placeholder:text-emerald-200/80",
          // Mask the rendered value when not revealed. This keeps line
          // breaks visible (helpful for cookie blobs) while keeping the
          // glyphs unreadable from a distance.
          !revealed &&
            value.length > 0 &&
            "text-transparent caret-foreground [text-shadow:0_0_8px_rgba(128,128,128,0.7)] selection:bg-accent/30"
        )}
      />
      <button
        type="button"
        onClick={() => setRevealed((v) => !v)}
        disabled={disabled}
        aria-label={toggleLabel}
        title={toggleLabel}
        className={cn(
          "absolute right-2 top-2",
          "text-muted-foreground/60 hover:text-foreground",
          "disabled:opacity-50"
        )}
      >
        {revealed ? <EyeOff className="w-3 h-3" /> : <Eye className="w-3 h-3" />}
      </button>
    </div>
  );
}
