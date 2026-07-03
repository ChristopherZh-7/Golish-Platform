const LIVE_TOOL_OUTPUT_TAIL_LIMIT = 64_000;
const LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX =
  "... (showing latest live output; earlier chunks are in the transcript/result)\n";

export function appendLiveToolOutput(current: string | undefined, chunk: string): string {
  if (!chunk) return current ?? "";

  const existing = current ?? "";
  const wasTruncated = existing.startsWith(LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX);
  const currentBody = wasTruncated
    ? existing.slice(LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX.length)
    : existing;
  const combined = currentBody + chunk;

  if (!wasTruncated && combined.length <= LIVE_TOOL_OUTPUT_TAIL_LIMIT) {
    return combined;
  }

  const tailLimit = Math.max(
    0,
    LIVE_TOOL_OUTPUT_TAIL_LIMIT - LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX.length
  );
  return LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX + combined.slice(-tailLimit);
}

export { LIVE_TOOL_OUTPUT_TAIL_LIMIT, LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX };
