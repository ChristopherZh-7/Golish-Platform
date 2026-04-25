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

export type RelativeTimeFallback = "days" | "localeDate";

export function formatRelativeTime(
  ts: string | number | undefined,
  fallback: RelativeTimeFallback = "days",
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
