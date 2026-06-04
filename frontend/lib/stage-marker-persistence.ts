/**
 * Persist task-mode stage dividers ("Stage complete" / "Step complete" bubbles)
 * across reloads.
 *
 * These are `role: "system"` messages carrying a {@link StageEvent}. They are
 * intentionally runtime-only in the conversation DB (`isPersistableMessage`
 * filters `role === "system"`), so without this they vanish on restart. We
 * snapshot them to localStorage keyed by the (stable) conversation id — the same
 * approach `contextUsagePersistence` / `stagePlanPersistence` use — and re-splice
 * them into the restored message list at load time.
 *
 * Each marker records the id of the message it followed (`anchorId`) so it can be
 * re-inserted at the right spot; markers whose anchor is gone are appended at the
 * end rather than dropped, so a milestone is never lost.
 */

import type { ChatMessage, StageEvent } from "@/store/slices/conversation";

/** localStorage key holding the `{ [conversationId]: marker[] }` map. */
export const STAGE_MARKER_STORAGE_KEY = "golish.stageMarkersByConversation";

export interface PersistedStageMarker {
  /** Id of the message this marker followed at save time (null = before all). */
  anchorId: string | null;
  marker: StageEvent;
}

type MarkerMap = Record<string, PersistedStageMarker[]>;

function isStageEvent(value: unknown): value is StageEvent {
  if (!value || typeof value !== "object") return false;
  const e = value as Record<string, unknown>;
  return typeof e.kind === "string" && typeof e.label === "string";
}

function isPersistedMarker(value: unknown): value is PersistedStageMarker {
  if (!value || typeof value !== "object") return false;
  const m = value as Record<string, unknown>;
  return (m.anchorId === null || typeof m.anchorId === "string") && isStageEvent(m.marker);
}

function readMap(): MarkerMap {
  try {
    const raw = globalThis.localStorage?.getItem(STAGE_MARKER_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return parsed as MarkerMap;
  } catch {
    return {};
  }
}

/** Read persisted stage markers for a conversation (best-effort, validated). */
export function readStageMarkers(conversationId: string): PersistedStageMarker[] {
  if (!conversationId) return [];
  const list = readMap()[conversationId];
  return Array.isArray(list) ? list.filter(isPersistedMarker) : [];
}

/**
 * Persist the ordered stage markers for a conversation. No-ops on an empty list
 * so an uninitialized view can never clobber a saved snapshot.
 */
export function writeStageMarkers(conversationId: string, markers: PersistedStageMarker[]): void {
  if (!conversationId || markers.length === 0) return;
  try {
    const map = readMap();
    map[conversationId] = markers;
    globalThis.localStorage?.setItem(STAGE_MARKER_STORAGE_KEY, JSON.stringify(map));
  } catch {
    // localStorage unavailable (privacy mode / non-browser) — best-effort only.
  }
}

/** Drop a conversation's persisted markers (e.g. on conversation delete). */
export function clearStageMarkers(conversationId: string): void {
  if (!conversationId) return;
  try {
    const map = readMap();
    if (!(conversationId in map)) return;
    delete map[conversationId];
    globalThis.localStorage?.setItem(STAGE_MARKER_STORAGE_KEY, JSON.stringify(map));
  } catch {
    // best-effort only
  }
}

/**
 * Extract the persistable stage markers from a live message list, anchoring each
 * to the id of the preceding non-system message.
 */
export function collectStageMarkers(messages: ChatMessage[]): PersistedStageMarker[] {
  const out: PersistedStageMarker[] = [];
  let anchorId: string | null = null;
  for (const m of messages) {
    if (m.role === "system") {
      if (m.stageEvent) out.push({ anchorId, marker: m.stageEvent });
    } else {
      anchorId = m.id;
    }
  }
  return out;
}

/**
 * Re-splice persisted markers into a restored message list: each marker is
 * re-inserted after its anchor message (markers sharing an anchor keep order);
 * markers whose anchor is missing are appended at the end so none are lost.
 */
export function spliceStageMarkers(
  messages: ChatMessage[],
  markers: PersistedStageMarker[]
): ChatMessage[] {
  if (markers.length === 0) return messages;

  const byAnchor = new Map<string | null, PersistedStageMarker[]>();
  for (const pm of markers) {
    const list = byAnchor.get(pm.anchorId) ?? [];
    list.push(pm);
    byAnchor.set(pm.anchorId, list);
  }

  let seq = 0;
  const toMessage = (pm: PersistedStageMarker, ts: number): ChatMessage => ({
    id: `stage-restored-${seq++}-${Math.random().toString(36).slice(2, 8)}`,
    role: "system",
    content: pm.marker.label,
    timestamp: ts,
    stageEvent: pm.marker,
  });

  const presentIds = new Set(messages.map((m) => m.id));
  const out: ChatMessage[] = [];

  for (const pm of byAnchor.get(null) ?? []) out.push(toMessage(pm, messages[0]?.timestamp ?? 0));
  for (const msg of messages) {
    out.push(msg);
    const following = byAnchor.get(msg.id);
    if (following) for (const pm of following) out.push(toMessage(pm, msg.timestamp));
  }
  // Anchors that no longer exist (e.g. their message wasn't persisted) — append
  // their markers at the end rather than silently dropping the milestone.
  const lastTs = messages[messages.length - 1]?.timestamp ?? 0;
  for (const [anchorId, list] of byAnchor) {
    if (anchorId !== null && !presentIds.has(anchorId)) {
      for (const pm of list) out.push(toMessage(pm, lastTs));
    }
  }

  return out;
}
