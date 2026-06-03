/**
 * Persist the most recent context-window usage per conversation so the
 * {@link ContextUsageRing} can show the last-known utilization immediately
 * after a refresh / app restart, instead of falling back to "unavailable"
 * until the backend emits the next `context_warning`.
 *
 * Conversation ids are stable across reloads (persisted to the conversation DB
 * and restored by id), so they make a safe localStorage key. Storage is a
 * single JSON map and every access is best-effort — a missing or corrupt entry
 * just yields `null` and the ring renders its neutral "no data" state.
 */

/** localStorage key holding the `{ [conversationId]: snapshot }` map. */
export const CONTEXT_USAGE_STORAGE_KEY = "golish.contextUsageByConversation";

export interface ContextUsageSnapshot {
  utilization: number;
  totalTokens: number;
  maxTokens: number;
}

type UsageMap = Record<string, ContextUsageSnapshot>;

function isSnapshot(value: unknown): value is ContextUsageSnapshot {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.utilization === "number" &&
    typeof v.totalTokens === "number" &&
    typeof v.maxTokens === "number"
  );
}

function readMap(): UsageMap {
  try {
    const raw = globalThis.localStorage?.getItem(CONTEXT_USAGE_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return parsed as UsageMap;
  } catch {
    return {};
  }
}

/** Read the last-known usage for a conversation (best-effort, validated). */
export function readContextUsage(conversationId: string): ContextUsageSnapshot | null {
  if (!conversationId) return null;
  const snapshot = readMap()[conversationId];
  return isSnapshot(snapshot) ? snapshot : null;
}

/** Persist the latest usage for a conversation so it survives reloads. */
export function writeContextUsage(conversationId: string, usage: ContextUsageSnapshot): void {
  if (!conversationId) return;
  try {
    const map = readMap();
    map[conversationId] = usage;
    globalThis.localStorage?.setItem(CONTEXT_USAGE_STORAGE_KEY, JSON.stringify(map));
  } catch {
    // localStorage unavailable (privacy mode / non-browser) — best-effort only.
  }
}
