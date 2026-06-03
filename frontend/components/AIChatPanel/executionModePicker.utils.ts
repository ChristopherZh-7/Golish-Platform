import type { ExecutionModeDescriptor } from "@/lib/ai";

/**
 * Pure helpers behind {@link ExecutionModePicker}.
 *
 * The backend only has two real engines — `Chat` and `Task` — while every
 * harness profile (assessment / pentest / red_team / …) is just configuration
 * layered onto the Task engine (`ExecutionMode::Task` + `harness_profile`). The
 * flat `list_execution_modes` payload mixes the Chat entry with one entry per
 * profile; these helpers re-separate the two concepts so the picker can present
 * a top-level engine choice plus a Task-only profile selector.
 */

/** Default harness profile when the user has never picked one. */
export const DEFAULT_PROFILE_ID = "assessment";

/** localStorage key remembering the last Task profile across reloads. */
export const LAST_PROFILE_STORAGE_KEY = "golish.lastHarnessProfile";

/**
 * localStorage key remembering the last *engine* selection (the full execution
 * mode id — `"chat"` or a profile id). Used to reopen new tabs / sessions in the
 * mode the user last chose instead of always resetting to Chat.
 */
export const LAST_MODE_STORAGE_KEY = "golish.lastExecutionMode";

/** The two real engines. Any non-`chat` execution-mode id is a Task profile. */
export type EngineId = "chat" | "task";

/**
 * Map an execution-mode id to its engine: `"chat"` is the Chat engine; any
 * other id (a harness profile id) means the Task engine is active.
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
    } else if (mode.id !== "task") {
      profiles.push(mode);
    }
  }
  return { chat, profiles };
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
  const isAvailable = (id: string | null): id is string =>
    id != null && profiles.some((p) => p.id === id);
  if (isAvailable(preferredId)) return preferredId;
  if (isAvailable(DEFAULT_PROFILE_ID)) return DEFAULT_PROFILE_ID;
  return profiles[0].id;
}

/** Read the last-used Task profile id from localStorage (best-effort). */
export function readLastProfile(): string | null {
  try {
    return globalThis.localStorage?.getItem(LAST_PROFILE_STORAGE_KEY) ?? null;
  } catch {
    return null;
  }
}

/** Persist the last-used Task profile id so Task remembers it across reloads. */
export function writeLastProfile(id: string): void {
  try {
    globalThis.localStorage?.setItem(LAST_PROFILE_STORAGE_KEY, id);
  } catch {
    // localStorage unavailable (privacy mode / non-browser) — best-effort only.
  }
}

/**
 * Read the last-used execution mode id (engine), defaulting to `"chat"`. New
 * tabs / sessions use this so they reopen in the mode the user last picked.
 */
export function readLastExecutionMode(): string {
  try {
    return globalThis.localStorage?.getItem(LAST_MODE_STORAGE_KEY) ?? "chat";
  } catch {
    return "chat";
  }
}

/** Persist the last-used execution mode id so new sessions reopen in it. */
export function writeLastExecutionMode(mode: string): void {
  try {
    globalThis.localStorage?.setItem(LAST_MODE_STORAGE_KEY, mode);
  } catch {
    // localStorage unavailable (privacy mode / non-browser) — best-effort only.
  }
}
