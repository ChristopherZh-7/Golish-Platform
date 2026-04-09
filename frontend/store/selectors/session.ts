/**
 * Combined Session Selectors
 *
 * Optimized selector for accessing terminal session state.
 * Returns only what the terminal-only central area needs:
 * timeline, pendingCommand, and workingDirectory.
 */

import {
  type PendingCommand,
  type UnifiedBlock,
  useStore,
} from "../index";

export interface SessionState {
  timeline: UnifiedBlock[];
  pendingCommand: PendingCommand | null;
  workingDirectory: string;
}

const EMPTY_TIMELINE: UnifiedBlock[] = [];

interface CacheEntry {
  timeline: UnifiedBlock[] | undefined;
  pendingCommand: PendingCommand | null | undefined;
  workingDirectory: string | undefined;
  result: SessionState;
}

const cache = new Map<string, CacheEntry>();

function getRawSessionInputs(state: ReturnType<typeof useStore.getState>, sessionId: string) {
  return {
    timeline: state.timelines[sessionId],
    pendingCommand: state.pendingCommand[sessionId],
    workingDirectory: state.sessions[sessionId]?.workingDirectory,
  };
}

function isCacheValid(cached: CacheEntry, inputs: ReturnType<typeof getRawSessionInputs>): boolean {
  return (
    cached.timeline === inputs.timeline &&
    cached.pendingCommand === inputs.pendingCommand &&
    cached.workingDirectory === inputs.workingDirectory
  );
}

function createSessionState(inputs: ReturnType<typeof getRawSessionInputs>): SessionState {
  return {
    timeline: inputs.timeline ?? EMPTY_TIMELINE,
    pendingCommand: inputs.pendingCommand ?? null,
    workingDirectory: inputs.workingDirectory ?? "",
  };
}

export function selectSessionState(
  state: ReturnType<typeof useStore.getState>,
  sessionId: string
): SessionState {
  const inputs = getRawSessionInputs(state, sessionId);
  const cached = cache.get(sessionId);

  if (cached && isCacheValid(cached, inputs)) {
    return cached.result;
  }

  const result = createSessionState(inputs);
  cache.set(sessionId, { ...inputs, result });
  return result;
}

export function useSessionState(sessionId: string): SessionState {
  return useStore((state) => selectSessionState(state, sessionId));
}

export function clearSessionCache(sessionId: string): void {
  cache.delete(sessionId);
}

export function clearAllSessionCaches(): void {
  cache.clear();
}
