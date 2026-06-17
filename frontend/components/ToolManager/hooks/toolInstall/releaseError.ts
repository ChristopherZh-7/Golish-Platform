/**
 * What the `github` install flow should do when fetching the latest
 * release fails.
 *
 * - `rate-limit`: GitHub REST quota exhausted — surface a friendly hint so
 *   the user adds a token; we can't tell whether a binary asset exists.
 * - `fall-back-to-clone`: the repo simply has no published Releases (the
 *   `/releases/latest` endpoint 404s). Source-only tools — pure-Python
 *   scripts like ctfr — legitimately ship none, so this is NOT a failure;
 *   proceed with a git clone of the source instead of aborting.
 * - `abort`: any other genuine error (network down, auth rejected, …).
 */
export type ReleaseFetchOutcome = "rate-limit" | "fall-back-to-clone" | "abort";

export function classifyReleaseFetchError(message: string): ReleaseFetchOutcome {
  if (message.includes("rate limit")) return "rate-limit";
  if (message.includes("404") || /not found/i.test(message)) return "fall-back-to-clone";
  return "abort";
}
