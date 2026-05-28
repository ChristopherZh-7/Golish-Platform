const lastSeenSeq = new Map<string, number>();
const lastSignaledAt = new Map<string, number>();

export function getLastSeenSequence(sessionId: string): number {
  return lastSeenSeq.get(sessionId) ?? -1;
}

export function setLastSeenSequence(sessionId: string, seq: number): void {
  lastSeenSeq.set(sessionId, seq);
}

export function resetSessionSequence(sessionId: string): void {
  lastSeenSeq.delete(sessionId);
}

export function resetAllSequences(): void {
  lastSeenSeq.clear();
}

export function getSessionSequenceCount(): number {
  return lastSeenSeq.size;
}

export function getLastSignaledAt(sessionId: string): number {
  return lastSignaledAt.get(sessionId) ?? 0;
}

export function setLastSignaledAt(sessionId: string, timestamp: number): void {
  lastSignaledAt.set(sessionId, timestamp);
}

export function resetLastSignaledAt(): void {
  lastSignaledAt.clear();
}
