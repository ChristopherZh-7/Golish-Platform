/**
 * Combined Selector for UnifiedInput Component
 *
 * Optimized selector for terminal-only input state.
 * AI-related state (agent responding, compacting, streaming) is no longer
 * needed here — AI interactions go through AIChatPanel.
 */

import { useStore } from "../index";

export interface UnifiedInputState {
  workingDirectory: string;
  virtualEnv: string | null;
  isSessionDead: boolean;
  gitBranch: string | null;
  gitStatus: {
    insertions: number;
    deletions: number;
    ahead: number;
    behind: number;
  } | null;
}

const EMPTY_GIT_STATUS = null;

interface CacheEntry {
  session: ReturnType<typeof useStore.getState>["sessions"][string] | undefined;
  isSessionDead: boolean | undefined;
  gitStatus: ReturnType<typeof useStore.getState>["gitStatus"][string] | undefined;
  result: UnifiedInputState;
}

const cache = new Map<string, CacheEntry>();

function getRawInputs(state: ReturnType<typeof useStore.getState>, sessionId: string) {
  return {
    session: state.sessions[sessionId],
    isSessionDead: state.isSessionDead[sessionId],
    gitStatus: state.gitStatus[sessionId],
  };
}

function isCacheValid(cached: CacheEntry, inputs: ReturnType<typeof getRawInputs>): boolean {
  return (
    cached.session === inputs.session &&
    cached.isSessionDead === inputs.isSessionDead &&
    cached.gitStatus === inputs.gitStatus
  );
}

function createInputState(inputs: ReturnType<typeof getRawInputs>): UnifiedInputState {
  const session = inputs.session;
  const gitStatus = inputs.gitStatus;

  return {
    workingDirectory: session?.workingDirectory ?? "",
    virtualEnv: session?.virtualEnv ?? null,
    isSessionDead: inputs.isSessionDead ?? false,
    gitBranch: gitStatus?.branch ?? null,
    gitStatus: gitStatus
      ? {
          insertions: gitStatus.insertions ?? 0,
          deletions: gitStatus.deletions ?? 0,
          ahead: gitStatus.ahead ?? 0,
          behind: gitStatus.behind ?? 0,
        }
      : EMPTY_GIT_STATUS,
  };
}

export function selectUnifiedInputState(
  state: ReturnType<typeof useStore.getState>,
  sessionId: string
): UnifiedInputState {
  const inputs = getRawInputs(state, sessionId);
  const cached = cache.get(sessionId);

  if (cached && isCacheValid(cached, inputs)) {
    return cached.result;
  }

  const result = createInputState(inputs);
  cache.set(sessionId, { ...inputs, result });
  return result;
}

export function useUnifiedInputState(sessionId: string): UnifiedInputState {
  return useStore((state) => selectUnifiedInputState(state, sessionId));
}

export function clearUnifiedInputCache(sessionId: string): void {
  cache.delete(sessionId);
}

export function clearAllUnifiedInputCaches(): void {
  cache.clear();
}
