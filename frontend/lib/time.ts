/**
 * Shared time formatting utilities.
 *
 * "Short" → ms/seconds only (e.g. "250ms", "1.5s", "")
 * "Long"  → includes minutes  (e.g. "250ms", "1.5s", "2m 30s")
 */

export function formatDurationShort(ms?: number | null): string {
  if (ms == null || ms === 0) return "";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export function formatDurationLong(ms?: number | null): string {
  if (ms == null || ms === 0) return "";
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  const minutes = Math.floor(ms / 60000);
  const seconds = Math.round((ms % 60000) / 1000);
  return `${minutes}m ${seconds}s`;
}

export function formatLogDate(ts: string | number): string {
  return new Date(ts).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

type RelativeTimeFallback = "days" | "localeDate";

export function formatRelativeTime(
  ts: string | number | undefined,
  fallback: RelativeTimeFallback = "days"
): string | null {
  if (ts == null) return null;
  const d = typeof ts === "string" ? new Date(ts) : new Date(ts);
  const diff = Date.now() - d.getTime();
  if (diff < 0) return null;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  if (fallback === "localeDate") return d.toLocaleDateString();
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

/**
 * Clock-style duration "M:SS" for media playback timers (e.g. "2:05").
 * Floors to whole seconds and zero-pads the seconds component.
 */
export function formatDurationClock(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

/**
 * Compact whole-unit elapsed span with no sub-second precision:
 * "45s", "2m 30s". Unlike `formatDurationLong` it never emits decimals
 * or a millisecond unit — use it for human-readable durations derived
 * from two timestamps.
 */
export function formatDurationCompact(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.floor((ms % 60_000) / 1000);
  return minutes > 0 ? `${minutes}m ${seconds}s` : `${seconds}s`;
}

export interface RelativeAgoOptions {
  /** Smallest unit shown. "second" → "5s ago"; "minute" → sub-minute renders "1m ago". Default "minute". */
  minUnit?: "second" | "minute";
  /** Largest unit before clamping. "hour" keeps counting hours; "day" rolls into days at 24h. Default "day". */
  maxUnit?: "hour" | "day";
  /** Returned when input can't be parsed. Defaults to echoing the original string (or "" for numbers). */
  invalidLabel?: string;
  /** Returned when the timestamp is in the future. If omitted, future is clamped to "now". */
  futureLabel?: string;
}

/**
 * Unified "<n> ago" relative formatter. Accepts an ISO string or epoch
 * millis and is policy-driven via {@link RelativeAgoOptions}, so the
 * several ad-hoc copies across the app can share one implementation
 * while keeping their exact original output.
 */
export function formatRelativeAgo(
  input: string | number,
  options: RelativeAgoOptions = {}
): string {
  const { minUnit = "minute", maxUnit = "day", invalidLabel, futureLabel } = options;
  const epoch = typeof input === "number" ? input : Date.parse(input);
  if (!Number.isFinite(epoch)) {
    return invalidLabel ?? (typeof input === "string" ? input : "");
  }
  const delta = Date.now() - epoch;
  if (delta < 0 && futureLabel !== undefined) return futureLabel;
  const elapsed = Math.max(0, delta);
  if (minUnit === "second") {
    const seconds = Math.floor(elapsed / 1000);
    if (seconds < 60) return `${seconds}s ago`;
  }
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 60) {
    const shown = minUnit === "minute" ? Math.max(1, minutes) : minutes;
    return `${shown}m ago`;
  }
  const hours = Math.floor(elapsed / 3_600_000);
  if (maxUnit === "hour" || hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}
