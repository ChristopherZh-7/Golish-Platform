interface TruncationResult {
  truncatedContent: string;
  isTruncated: boolean;
  totalLines: number;
  hiddenLines: number;
}

export function truncateByLines(content: string, maxLines: number = 10): TruncationResult {
  const lines = content.split("\n");
  const totalLines = lines.length;
  const isTruncated = totalLines > maxLines;

  return {
    truncatedContent: isTruncated ? lines.slice(0, maxLines).join("\n") : content,
    isTruncated,
    totalLines,
    hiddenLines: isTruncated ? totalLines - maxLines : 0,
  };
}

/**
 * Serialize an arbitrary value for display without ever materializing an
 * unbounded string. Oversized string fields and arrays are capped *during*
 * serialization, so a multi-megabyte tool result cannot trigger
 * `RangeError: Out of memory` (which a stringify-then-truncate approach cannot
 * avoid, because the OOM happens inside `JSON.stringify`).
 */
export function safeStringify(value: unknown, maxLength: number = 8000): string {
  if (typeof value === "string") {
    return value.length > maxLength ? `${value.slice(0, maxLength)}\n... (truncated)` : value;
  }

  const MAX_FIELD_CHARS = 2000;
  const MAX_ARRAY_ITEMS = 200;
  const seen = new WeakSet<object>();

  try {
    const json = JSON.stringify(
      value,
      (_key, val) => {
        if (typeof val === "string" && val.length > MAX_FIELD_CHARS) {
          return `${val.slice(0, MAX_FIELD_CHARS)}… (+${val.length - MAX_FIELD_CHARS} chars)`;
        }
        if (Array.isArray(val) && val.length > MAX_ARRAY_ITEMS) {
          return [...val.slice(0, MAX_ARRAY_ITEMS), `… (+${val.length - MAX_ARRAY_ITEMS} items)`];
        }
        if (typeof val === "object" && val !== null) {
          if (seen.has(val)) return "[Circular]";
          seen.add(val);
        }
        return val;
      },
      2
    );

    if (json === undefined) return String(value);
    return json.length > maxLength ? `${json.slice(0, maxLength)}\n... (truncated)` : json;
  } catch {
    return "[result too large or not serializable to display]";
  }
}
