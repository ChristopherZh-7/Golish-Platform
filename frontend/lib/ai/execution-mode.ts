import { invoke } from "@/lib/api/client";

/**
 * Descriptor for one execution mode policy registered on the backend.
 *
 * Field names match the camelCase wire format produced by the Tauri
 * `list_execution_modes` command (see
 * `backend/crates/golish/src/ai/commands/mode.rs::ExecutionModeDescriptor`,
 * which uses `#[serde(rename_all = "camelCase")]`).
 */
export interface ExecutionModeDescriptor {
  /** Stable lookup id, e.g. "chat" / "task" / future "plan". */
  id: string;
  /** Human-readable label rendered in the picker. */
  displayName: string;
  /**
   * Lucide icon name. The picker maps known names to React components
   * and falls back to `MessageSquare` for unknown values, so adding a
   * new mode with an unmapped icon string is non-fatal.
   */
  icon: string;
  /**
   * Free-form CSS theme key (e.g. `"muted"`, `"magenta"`). The picker
   * applies a small allow-list of background classes; unknown values
   * gracefully fall back to the default neutral style.
   */
  badgeColor: string;
  /** Tooltip / help text shown under the option. */
  description: string;
  /**
   * `true` if the mode allows the LLM to dispatch sub-agents. Used by
   * the picker to enable / disable the "Sub-Agents" toggle row.
   */
  allowsSubAgents: boolean;
}

/** Default harness profile when the user has never picked one. */
export const DEFAULT_PROFILE_ID = "assessment";

/** Legacy Task engine id. UI state should normalize this to a concrete profile. */
export const LEGACY_TASK_MODE_ID = "task";

/** localStorage key remembering the last Task profile across reloads. */
export const LAST_PROFILE_STORAGE_KEY = "golish.lastHarnessProfile";

/**
 * localStorage key remembering the last execution mode id. For Task mode this
 * stores a concrete profile id (`red_team`, `assessment`, ...), not bare `task`.
 */
export const LAST_MODE_STORAGE_KEY = "golish.lastExecutionMode";

/** The two real engines. Any non-`chat` execution-mode id is a Task profile. */
export type EngineId = "chat" | "task";

/**
 * Map an execution-mode id to its engine: `"chat"` is the Chat engine; any
 * other id (a harness profile id or legacy `task`) means the Task engine is
 * active.
 */
export function resolveEngine(executionModeId: string): EngineId {
  return executionModeId === "chat" ? "chat" : "task";
}

export interface SplitModes {
  /** The Chat engine descriptor, if present in the payload. */
  chat: ExecutionModeDescriptor | null;
  /** Task harness profiles, in backend order. */
  profiles: ExecutionModeDescriptor[];
}

/**
 * Split the flat `list_execution_modes` payload into the Chat entry and the
 * Task harness profiles. The legacy bare `task` id (if ever surfaced) is
 * dropped — Task is a top-level engine here, not a selectable profile.
 */
export function splitModes(modes: ExecutionModeDescriptor[]): SplitModes {
  let chat: ExecutionModeDescriptor | null = null;
  const profiles: ExecutionModeDescriptor[] = [];
  for (const mode of modes) {
    if (mode.id === "chat") {
      chat = mode;
    } else if (mode.id !== LEGACY_TASK_MODE_ID) {
      profiles.push(mode);
    }
  }
  return { chat, profiles };
}

/** Return a concrete Task profile id, ignoring legacy engine ids. */
export function normalizeTaskProfileId(id: string | null | undefined): string | null {
  const value = id?.trim();
  if (!value || value === "chat" || value === LEGACY_TASK_MODE_ID) return null;
  return value;
}

/**
 * Choose which profile to activate when entering Task mode. Prefers the
 * remembered profile, then {@link DEFAULT_PROFILE_ID}, then the first available
 * one, guaranteeing a valid id whenever at least one profile exists.
 */
export function pickTaskProfile(
  preferredId: string | null,
  profiles: ExecutionModeDescriptor[]
): string | null {
  if (profiles.length === 0) return null;
  const isAvailable = (id: string | null): id is string => {
    const profileId = normalizeTaskProfileId(id);
    return profileId != null && profiles.some((p) => p.id === profileId);
  };
  const normalizedPreferred = normalizeTaskProfileId(preferredId);
  if (isAvailable(normalizedPreferred)) return normalizedPreferred;
  if (isAvailable(DEFAULT_PROFILE_ID)) return DEFAULT_PROFILE_ID;
  return profiles[0].id;
}

/** Read the last-used Task profile id from localStorage (best-effort). */
export function readLastProfile(): string | null {
  try {
    return normalizeTaskProfileId(globalThis.localStorage?.getItem(LAST_PROFILE_STORAGE_KEY));
  } catch {
    return null;
  }
}

/** Persist the last-used Task profile id so Task remembers it across reloads. */
export function writeLastProfile(id: string): void {
  try {
    const profileId = normalizeTaskProfileId(id);
    if (!profileId) {
      globalThis.localStorage?.removeItem(LAST_PROFILE_STORAGE_KEY);
      return;
    }
    globalThis.localStorage?.setItem(LAST_PROFILE_STORAGE_KEY, profileId);
  } catch {
    // localStorage unavailable (privacy mode / non-browser) — best-effort only.
  }
}

/**
 * Convert legacy bare `task` into a concrete profile id. This keeps restored
 * sessions, localStorage, and backend `set_execution_mode` calls aligned with
 * the picker, whose submenu only contains profiles.
 */
export function normalizeExecutionModeId(
  mode: string | null | undefined,
  preferredProfile: string | null = readLastProfile()
): string {
  const value = mode?.trim();
  if (!value) return "chat";
  if (value !== LEGACY_TASK_MODE_ID) return value;
  return normalizeTaskProfileId(preferredProfile) ?? DEFAULT_PROFILE_ID;
}

/**
 * Read the last-used execution mode id, defaulting to `"chat"`. New tabs /
 * sessions use this so they reopen in the mode the user last picked.
 */
export function readLastExecutionMode(): string {
  try {
    const stored = globalThis.localStorage?.getItem(LAST_MODE_STORAGE_KEY) ?? null;
    const normalized = normalizeExecutionModeId(stored);
    if (stored && stored !== normalized) {
      globalThis.localStorage?.setItem(LAST_MODE_STORAGE_KEY, normalized);
    }
    return normalized;
  } catch {
    return "chat";
  }
}

/** Persist the last-used execution mode id so new sessions reopen in it. */
export function writeLastExecutionMode(mode: string): void {
  try {
    globalThis.localStorage?.setItem(LAST_MODE_STORAGE_KEY, normalizeExecutionModeId(mode));
  } catch {
    // localStorage unavailable (privacy mode / non-browser) — best-effort only.
  }
}

/**
 * Fetch the list of execution modes registered on the backend.
 *
 * The call hits the cheap `list_execution_modes` Tauri command which
 * iterates `ExecutionModeRegistry::default()` synchronously, so it is
 * safe to invoke on mount without debouncing.
 */
export async function listExecutionModes(): Promise<ExecutionModeDescriptor[]> {
  return invoke<ExecutionModeDescriptor[]>("list_execution_modes");
}
