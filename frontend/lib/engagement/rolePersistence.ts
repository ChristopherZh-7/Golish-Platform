/**
 * Engagement-role persistence for conversations (Phase C, 设计 2026-06-13).
 *
 * `engagementRole` / `workerMeta` are runtime fields on `ChatConversation`
 * that the conversation DB's explicit column set doesn't carry. Mirror the
 * stage-marker pattern (`lib/stage-marker-persistence.ts`): a small
 * localStorage map keyed by conversation id, re-applied on restore — so the
 * overview marker and worker tags survive reloads even though the live pool
 * state (by design, spec §10) does not.
 */

import type { ChatConversation } from "@/store/slices/conversation";

const STORAGE_KEY = "golish.engagementRoles";

export interface PersistedEngagementRole {
  engagementRole: NonNullable<ChatConversation["engagementRole"]>;
  workerMeta?: ChatConversation["workerMeta"];
}

type RoleMap = Record<string, PersistedEngagementRole>;

function readMap(): RoleMap {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? (parsed as RoleMap) : {};
  } catch {
    return {};
  }
}

function writeMap(map: RoleMap): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(map));
  } catch {
    // localStorage unavailable (privacy mode / non-browser) — best-effort only.
  }
}

/** Read one conversation's persisted engagement role, if any. */
export function readEngagementRole(conversationId: string): PersistedEngagementRole | null {
  if (!conversationId) return null;
  const entry = readMap()[conversationId];
  return entry && typeof entry.engagementRole === "string" ? entry : null;
}

/** Persist (or with `null`, clear) a conversation's engagement role. */
export function writeEngagementRole(
  conversationId: string,
  role: PersistedEngagementRole | null
): void {
  if (!conversationId) return;
  const map = readMap();
  if (role == null) {
    if (!(conversationId in map)) return;
    delete map[conversationId];
  } else {
    map[conversationId] = role;
  }
  writeMap(map);
}

/** Re-apply a persisted role onto a freshly restored conversation (mutates). */
export function applyPersistedEngagementRole(conv: ChatConversation): void {
  const persisted = readEngagementRole(conv.id);
  if (!persisted) return;
  conv.engagementRole = persisted.engagementRole;
  if (persisted.workerMeta) conv.workerMeta = persisted.workerMeta;
}
