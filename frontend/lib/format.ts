export function formatBytes(
  bytes: number,
  opts?: { compact?: boolean; zeroLabel?: string },
): string {
  const compact = opts?.compact ?? true;
  const zeroLabel = opts?.zeroLabel ?? "-";
  if (bytes === 0) return zeroLabel;
  const sep = compact ? "" : " ";
  if (bytes < 1024) return `${bytes}${sep}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}${sep}${compact ? "K" : "KB"}`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}${sep}${compact ? "M" : "MB"}`;
}
