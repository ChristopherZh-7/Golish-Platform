/**
 * Recover the owning stage_run request from the durable Agent request identity.
 * Stage Team and legacy org fan-out identities both retain the root tool id.
 */
export function stageRunRequestIdFromAgentRequestId(agentRequestId?: string | null): string | null {
  if (!agentRequestId) return null;
  const indexes = ["::org::", "::team::"]
    .map((marker) => agentRequestId.indexOf(marker))
    .filter((index) => index > 0);
  return indexes.length > 0 ? agentRequestId.slice(0, Math.min(...indexes)) : null;
}

/** Prefer an exact persisted Stage id, then recover the canonical id embedded in the Agent id. */
export function resolveOwningStageRunRequestId(
  agentRequestId: string,
  knownStageRequestIds: readonly string[]
): string | null {
  const exact = [...knownStageRequestIds]
    .filter(Boolean)
    .sort((left, right) => right.length - left.length)
    .find(
      (stageRequestId) =>
        agentRequestId === stageRequestId || agentRequestId.startsWith(`${stageRequestId}::`)
    );
  return exact ?? stageRunRequestIdFromAgentRequestId(agentRequestId);
}
