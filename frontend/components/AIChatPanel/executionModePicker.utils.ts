/**
 * Pure helpers behind {@link ExecutionModePicker}.
 *
 * Kept as a component-local import path for existing tests/callers; the shared
 * implementation lives in `frontend/lib/ai/execution-mode.ts` because restore
 * paths outside AIChatPanel also need to normalize legacy bare `task`.
 */
export {
  DEFAULT_PROFILE_ID,
  type EngineId,
  LAST_MODE_STORAGE_KEY,
  LAST_PROFILE_STORAGE_KEY,
  normalizeExecutionModeId,
  normalizeTaskProfileId,
  pickTaskProfile,
  readLastExecutionMode,
  readLastProfile,
  resolveEngine,
  type SplitModes,
  splitModes,
  writeLastExecutionMode,
  writeLastProfile,
} from "@/lib/ai/execution-mode";
