import { AlertTriangle, CheckCircle2, Clock3, Loader2, RotateCw, ShieldAlert } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ApiError } from "@/lib/api";
import {
  type AttackCandidateRecoveryResolveResponse,
  type AttackVerificationQueueState,
  listVerificationQueue,
  resolveCandidateRecovery,
} from "@/lib/api/attack";
import { translateErrorCode } from "@/lib/api/error-codes";
import { cn } from "@/lib/utils";

interface CandidateVerificationProtocolProps {
  operationId: string;
  waveRunId: string;
  refreshVersion: number;
}

type RecoveryCase = AttackVerificationQueueState["items"][number]["recoveryCases"][number];
type RecoveryDecision =
  | "terminalize_blocked_outcome_unknown"
  | "abandon_before_side_effect"
  | "accept_external_result_with_exact_evidence";

const RECOVERY_LABELS: Record<RecoveryDecision, string> = {
  terminalize_blocked_outcome_unknown: "Record blocked outcome unknown",
  abandon_before_side_effect: "Abandon before side effect",
  accept_external_result_with_exact_evidence: "Accept external result evidence",
};

function displayError(error: unknown): string {
  if (error instanceof ApiError) return translateErrorCode(error.code, error.message);
  return error instanceof Error ? error.message : String(error);
}

function requestId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `candidate-recovery-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function evidenceIds(value: string): number[] | null {
  if (value.trim() === "") return [];
  const ids = value
    .split(/[\s,]+/)
    .filter(Boolean)
    .map(Number);
  if (ids.some((id) => !Number.isSafeInteger(id) || id <= 0)) return null;
  const unique = Array.from(new Set(ids)).sort((left, right) => left - right);
  return unique.length === ids.length ? unique : null;
}

function EvidenceLinks({
  title,
  evidence,
}: {
  title: string;
  evidence: Array<{ evidenceId: number; role: string }>;
}) {
  if (evidence.length === 0) return null;
  return (
    <div className="mt-1.5 flex flex-wrap items-center gap-1 text-[10px]">
      <span className="text-muted-foreground">{title}</span>
      {evidence.map((entry) => (
        <a
          key={`${entry.role}-${entry.evidenceId}`}
          href={`#evidence-${entry.evidenceId}`}
          className="rounded border border-border/35 px-1 py-0.5 font-mono text-accent hover:underline"
        >
          {entry.role} #{entry.evidenceId}
        </a>
      ))}
    </div>
  );
}

function ProtocolIdentity({ label, value }: { label: string; value: string | null }) {
  if (value === null) return null;
  return (
    <div>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="break-all font-mono text-foreground/80">{value}</dd>
    </div>
  );
}

export function CandidateVerificationProtocol({
  operationId,
  waveRunId,
  refreshVersion,
}: CandidateVerificationProtocolProps) {
  const [queue, setQueue] = useState<AttackVerificationQueueState | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [evidenceDrafts, setEvidenceDrafts] = useState<Record<string, string>>({});
  const [mutationRequestIds, setMutationRequestIds] = useState<Record<string, string>>({});
  const [mutatingCaseId, setMutatingCaseId] = useState<string | null>(null);
  const [mutationError, setMutationError] = useState<string | null>(null);
  const [mutationResult, setMutationResult] =
    useState<AttackCandidateRecoveryResolveResponse | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setQueue(await listVerificationQueue({ operationId, waveRunId }));
    } catch (cause) {
      setError(displayError(cause));
    } finally {
      setLoading(false);
    }
  }, [operationId, waveRunId]);

  useEffect(() => {
    void refreshVersion;
    void load();
  }, [load, refreshVersion]);

  const recoveries = useMemo(
    () => queue?.items.flatMap((item) => item.recoveryCases) ?? [],
    [queue]
  );

  const resolve = useCallback(
    async (recovery: RecoveryCase, decision: RecoveryDecision) => {
      const parsedEvidence = evidenceIds(evidenceDrafts[recovery.recoveryCaseId] ?? "");
      if (parsedEvidence === null) {
        setMutationError("Evidence IDs must be unique positive integers.");
        return;
      }
      if (
        decision === "accept_external_result_with_exact_evidence" &&
        parsedEvidence.length === 0
      ) {
        setMutationError("Accepting an external result requires exact evidence IDs.");
        return;
      }
      const stableRequestId = mutationRequestIds[recovery.recoveryCaseId] ?? requestId();
      setMutationRequestIds((current) => ({
        ...current,
        [recovery.recoveryCaseId]: stableRequestId,
      }));
      setMutationError(null);
      setMutationResult(null);
      setMutatingCaseId(recovery.recoveryCaseId);
      try {
        const result = await resolveCandidateRecovery({
          operationId,
          waveRunId,
          recoveryCaseId: recovery.recoveryCaseId,
          requestId: stableRequestId,
          expectedRowVersion: recovery.rowVersion,
          expectedAttemptRowVersion: recovery.attemptRowVersion,
          decision,
          evidenceIds:
            decision === "accept_external_result_with_exact_evidence" ? parsedEvidence : [],
        });
        setMutationResult(result);
        await load();
      } catch (cause) {
        // Preserve stableRequestId so a response-loss retry is an exact replay.
        setMutationError(displayError(cause));
      } finally {
        setMutatingCaseId(null);
      }
    },
    [evidenceDrafts, load, mutationRequestIds, operationId, waveRunId]
  );

  if (loading && queue === null) {
    return (
      <div className="flex items-center gap-2 rounded border border-border/30 p-3 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading durable Verification queue…
      </div>
    );
  }
  if (queue === null) {
    return (
      <div className="rounded border border-red-500/30 bg-red-500/5 p-3 text-xs">
        <div role="alert" className="flex items-center gap-2 text-red-300">
          <AlertTriangle className="h-3.5 w-3.5" />
          {error ?? "The durable Verification queue is unavailable."}
        </div>
        <button
          type="button"
          className="mt-2 text-accent hover:underline"
          onClick={() => void load()}
        >
          Retry queue read
        </button>
      </div>
    );
  }

  return (
    <section className="space-y-2 rounded border border-border/30 bg-card/20 p-2.5">
      <header className="flex flex-wrap items-center gap-2">
        <Clock3 className="h-3.5 w-3.5 text-accent" />
        <h4 className="text-[11px] font-semibold">Durable Verification protocol</h4>
        <span className="rounded border border-border/35 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
          wave {queue.generation} · {queue.waveStatus} · row v{queue.waveRowVersion}
        </span>
        <span className="ml-auto text-[10px] text-muted-foreground">
          {queue.items.length} queue items ·{" "}
          {recoveries.filter((item) => item.status !== "resolved").length} recovery pending ·{" "}
          {queue.pendingEnrichmentCount} FactDelta enrichment pending
        </span>
        <button type="button" aria-label="Refresh Verification queue" onClick={() => void load()}>
          <RotateCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
        </button>
      </header>

      <div className="flex flex-wrap gap-1 text-[9px] text-muted-foreground">
        <span>Legal recovery decisions:</span>
        {Object.values(RECOVERY_LABELS).map((label) => (
          <span key={label} className="rounded border border-border/30 px-1 py-0.5">
            {label}
          </span>
        ))}
      </div>

      {error !== null && (
        <div role="alert" className="rounded border border-red-500/30 p-2 text-[10px] text-red-300">
          {error}
        </div>
      )}
      {mutationError !== null && (
        <div role="alert" className="rounded border border-red-500/30 p-2 text-[10px] text-red-300">
          {mutationError}
        </div>
      )}
      {mutationResult !== null && (
        <div className="rounded border border-amber-500/30 bg-amber-500/5 p-2 text-[10px] text-amber-200">
          {mutationResult.pendingServerConvergence
            ? "Recovery decision recorded; pending server convergence."
            : "Recovery decision is resolved by the server."}
          <div className="mt-0.5 break-all font-mono">
            request {mutationResult.decisionRequestId} · row v{mutationResult.rowVersion}
            {mutationResult.replayed ? " · exact replay" : ""}
          </div>
        </div>
      )}

      {queue.waveUnits.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {queue.waveUnits.map((unit) => (
            <span
              key={unit.waveUnitId}
              className="rounded border border-border/30 px-1.5 py-0.5 text-[9px] text-muted-foreground"
              title={unit.waveUnitId}
            >
              org {unit.ordinal}: {unit.status} / {unit.consolidationStatus} / row v
              {unit.rowVersion}
            </span>
          ))}
        </div>
      )}
      {queue.pendingEnrichments.length > 0 && (
        <section className="space-y-1.5" aria-label="Pending FactDelta enrichments">
          {queue.pendingEnrichments.map((pending) => (
            <article
              key={pending.enrichmentId}
              data-testid={`verification-pending-enrichment-${pending.enrichmentId}`}
              className="rounded border border-amber-500/30 bg-amber-500/5 p-2 text-[10px] text-amber-100"
            >
              <div className="flex flex-wrap items-center gap-1.5">
                <ShieldAlert className="h-3.5 w-3.5 text-amber-300" />
                <span className="font-semibold">FactDelta enrichment pending</span>
                <span className="rounded border border-amber-500/30 px-1 py-0.5 font-mono">
                  {pending.reasonCode}
                </span>
                <span className="ml-auto font-mono text-amber-200/75">{pending.status}</span>
              </div>
              <dl className="mt-1.5 grid gap-1 rounded bg-black/10 p-1.5 sm:grid-cols-2">
                <ProtocolIdentity
                  label="subject"
                  value={`${pending.subjectKind} · ${pending.subjectId}`}
                />
                <ProtocolIdentity
                  label="frozen target"
                  value={`${pending.targetTypeAtTime} · ${pending.targetValueAtTime}`}
                />
                <ProtocolIdentity
                  label="route"
                  value={`${pending.deltaKind} → ${pending.observationKind}`}
                />
                <ProtocolIdentity label="FactDelta" value={pending.factDeltaId} />
              </dl>
              <div className="mt-1.5 flex flex-wrap items-center gap-1">
                <span className="text-amber-200/75">Allowed techniques</span>
                {pending.allowedTechniques.map((technique) => (
                  <span
                    key={technique}
                    className="rounded border border-amber-500/30 px-1 py-0.5 font-mono"
                  >
                    {technique}
                  </span>
                ))}
              </div>
              <p className="mt-1.5 text-amber-200/80">
                Source Wave remains open; no Candidate WorkItem has been created.
              </p>
            </article>
          ))}
        </section>
      )}
      {queue.consolidation !== null && (
        <div className="rounded border border-emerald-500/25 bg-emerald-500/5 p-1.5 text-[10px] text-emerald-200">
          Wave consolidation {queue.consolidation.decisionKind} ·{" "}
          {queue.consolidation.factDeltaCount} FactDelta · row v{queue.consolidation.rowVersion}
          <div className="mt-0.5 break-all font-mono">
            {queue.consolidation.consolidationId} · {queue.consolidation.reasonCode}
          </div>
        </div>
      )}

      {queue.items.length === 0 ? (
        <div className="rounded border border-border/30 p-3 text-center text-[11px] text-muted-foreground">
          No durable Candidate Verification queue items exist for this Wave.
        </div>
      ) : (
        <div className="space-y-2">
          {queue.items.map((item, index) => {
            const hasOpenRecovery = item.recoveryCases.some(
              (recovery) => recovery.status !== "resolved"
            );
            return (
              <article
                key={item.attemptId}
                data-testid={`verification-protocol-${item.attemptId}`}
                className="rounded border border-border/30 bg-muted/10 p-2 text-[10px]"
              >
                <div className="flex flex-wrap items-center gap-1.5">
                  {item.terminalReceipt !== null ? (
                    <CheckCircle2 className="h-3.5 w-3.5 text-emerald-300" />
                  ) : hasOpenRecovery ? (
                    <ShieldAlert className="h-3.5 w-3.5 text-amber-300" />
                  ) : item.status === "running" || item.status === "terminalization_pending" ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-accent" />
                  ) : (
                    <Clock3 className="h-3.5 w-3.5 text-muted-foreground" />
                  )}
                  <span className="font-semibold">
                    Queue {index + 1} · Attempt {item.ordinal}
                  </span>
                  <span className="rounded border border-border/30 px-1 py-0.5">
                    {hasOpenRecovery ? "recovery required" : item.status}
                  </span>
                  <span className="font-mono text-muted-foreground">row v{item.rowVersion}</span>
                  {item.workerStatus !== null && (
                    <span className="font-mono text-muted-foreground">
                      worker {item.workerStatus}
                    </span>
                  )}
                </div>
                <div className="mt-1 break-all font-mono text-foreground/85">{item.attemptId}</div>
                <div className="mt-0.5 text-foreground/80">{item.hypothesis}</div>
                <div className="mt-1 grid gap-1 rounded bg-muted/15 p-1.5 sm:grid-cols-3">
                  <ProtocolIdentity label="recipe" value={item.recipeVersion} />
                  <ProtocolIdentity label="executor" value={item.executorContractVersion} />
                  <ProtocolIdentity
                    label="approval start before"
                    value={item.approvalStartBefore}
                  />
                  <ProtocolIdentity label="plan hash" value={item.candidatePlanHash} />
                  <ProtocolIdentity label="worker id" value={item.workerRunId} />
                  <div>
                    <dt className="text-muted-foreground">budget</dt>
                    <dd className="font-mono text-foreground/80">
                      {item.actions.length}/{item.budgetMaxActions} actions ·{" "}
                      {item.budgetMaxRequests} requests · {item.budgetMaxRuntimeMs}ms
                    </dd>
                  </div>
                </div>
                <EvidenceLinks title="Observation" evidence={item.observationEvidence} />
                <EvidenceLinks title="Attempt" evidence={item.attemptEvidence} />

                <div className="mt-2">
                  <div className="font-semibold text-muted-foreground">Action journal</div>
                  {item.actions.length === 0 ? (
                    <div className="mt-1 rounded border border-border/25 p-1.5 text-muted-foreground">
                      No action journal rows.
                    </div>
                  ) : (
                    <div className="mt-1 space-y-1">
                      {item.actions.map((action) => (
                        <div
                          key={action.actionId}
                          className="rounded border border-border/25 p-1.5"
                        >
                          <div className="flex flex-wrap gap-1.5">
                            <span className="font-semibold">
                              #{action.actionOrdinal} {action.actionKind}
                            </span>
                            <span>{action.capabilityId}</span>
                            <span className="rounded border border-border/30 px-1">
                              {action.status}
                            </span>
                          </div>
                          <dl className="mt-1 grid gap-1 sm:grid-cols-2">
                            <ProtocolIdentity label="action id" value={action.actionId} />
                            <ProtocolIdentity
                              label="authorization receipt"
                              value={action.authorizationReceiptId}
                            />
                            <ProtocolIdentity
                              label="authorization request"
                              value={action.authorizationRequestId}
                            />
                            <ProtocolIdentity label="start before" value={action.startBefore} />
                          </dl>
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                <div className="mt-2 grid gap-1 sm:grid-cols-3">
                  <div className="rounded border border-border/25 p-1.5">
                    <div className="font-semibold">Terminal intent</div>
                    {item.terminalIntent === null ? (
                      <div className="text-muted-foreground">not persisted</div>
                    ) : (
                      <dl className="mt-1 space-y-1">
                        <ProtocolIdentity label="intent id" value={item.terminalIntent.intentId} />
                        <ProtocolIdentity
                          label="request id"
                          value={item.terminalIntent.requestId}
                        />
                        <ProtocolIdentity
                          label="disposition"
                          value={item.terminalIntent.disposition}
                        />
                      </dl>
                    )}
                  </div>
                  <div className="rounded border border-border/25 p-1.5">
                    <div className="font-semibold">Checkpoint barrier</div>
                    {item.terminalBarrier === null ? (
                      <div className="text-muted-foreground">not durable</div>
                    ) : (
                      <dl className="mt-1 space-y-1">
                        <ProtocolIdentity
                          label="barrier id"
                          value={item.terminalBarrier.barrierId}
                        />
                        <ProtocolIdentity
                          label="request id"
                          value={item.terminalBarrier.requestId}
                        />
                      </dl>
                    )}
                  </div>
                  <div className="rounded border border-border/25 p-1.5">
                    <div className="font-semibold">Terminal receipt</div>
                    {item.terminalReceipt === null ? (
                      <div className="text-muted-foreground">pending terminalizer</div>
                    ) : (
                      <dl className="mt-1 space-y-1">
                        <ProtocolIdentity
                          label="receipt id"
                          value={item.terminalReceipt.receiptId}
                        />
                        <ProtocolIdentity
                          label="request id"
                          value={item.terminalReceipt.requestId}
                        />
                        <ProtocolIdentity
                          label="finding id"
                          value={item.terminalReceipt.findingId}
                        />
                      </dl>
                    )}
                  </div>
                </div>

                {item.recoveryCases.map((recovery) => {
                  const parsedEvidence = evidenceIds(evidenceDrafts[recovery.recoveryCaseId] ?? "");
                  const busy = mutatingCaseId === recovery.recoveryCaseId;
                  const open = recovery.status === "open";
                  const stableRequestId = mutationRequestIds[recovery.recoveryCaseId];
                  return (
                    <div
                      key={recovery.recoveryCaseId}
                      className="mt-2 rounded border border-amber-500/30 bg-amber-500/5 p-2 text-amber-100"
                    >
                      <div className="flex flex-wrap items-center gap-1.5">
                        <ShieldAlert className="h-3.5 w-3.5" />
                        <span className="font-semibold">Recovery {recovery.caseKind}</span>
                        <span className="rounded border border-amber-500/30 px-1">
                          {recovery.status}
                        </span>
                        <span className="font-mono">
                          row v{recovery.rowVersion} · attempt v{recovery.attemptRowVersion}
                        </span>
                      </div>
                      <dl className="mt-1 grid gap-1 sm:grid-cols-2">
                        <ProtocolIdentity
                          label="recovery case id"
                          value={recovery.recoveryCaseId}
                        />
                        <ProtocolIdentity label="case request id" value={recovery.requestId} />
                        <ProtocolIdentity label="action id" value={recovery.actionId} />
                        <ProtocolIdentity label="intent id" value={recovery.intentId} />
                        <ProtocolIdentity label="resolution" value={recovery.resolutionKind} />
                        <ProtocolIdentity
                          label="resolution request id"
                          value={recovery.resolutionRequestId}
                        />
                      </dl>
                      <div className="mt-1 font-mono">reason {recovery.reasonCode}</div>
                      {recovery.evidenceIds.length > 0 && (
                        <div className="mt-1 flex flex-wrap gap-1">
                          {recovery.evidenceIds.map((evidenceId) => (
                            <a
                              key={evidenceId}
                              href={`#evidence-${evidenceId}`}
                              className="font-mono text-accent hover:underline"
                            >
                              evidence #{evidenceId}
                            </a>
                          ))}
                        </div>
                      )}
                      {stableRequestId !== undefined && (
                        <div className="mt-1 break-all font-mono text-[9px]">
                          mutation request {stableRequestId}
                        </div>
                      )}
                      {open && (
                        <div className="mt-2 space-y-1.5">
                          <label className="block text-[9px] text-muted-foreground">
                            Exact evidence IDs for external-result acceptance
                            <input
                              value={evidenceDrafts[recovery.recoveryCaseId] ?? ""}
                              onChange={(event) =>
                                setEvidenceDrafts((current) => ({
                                  ...current,
                                  [recovery.recoveryCaseId]: event.target.value,
                                }))
                              }
                              placeholder="e.g. 41, 42"
                              className="mt-0.5 w-full rounded border border-border/40 bg-background/70 px-2 py-1 font-mono text-[10px] text-foreground"
                            />
                          </label>
                          <div className="flex flex-wrap gap-1">
                            <button
                              type="button"
                              disabled={busy || recovery.caseKind !== "outcome_unknown"}
                              onClick={() =>
                                void resolve(recovery, "terminalize_blocked_outcome_unknown")
                              }
                              className="rounded border border-amber-500/35 px-1.5 py-1 disabled:cursor-not-allowed disabled:opacity-40"
                            >
                              Record blocked outcome unknown
                            </button>
                            <button
                              type="button"
                              disabled={busy || recovery.caseKind !== "approval_start_expired"}
                              onClick={() => void resolve(recovery, "abandon_before_side_effect")}
                              className="rounded border border-amber-500/35 px-1.5 py-1 disabled:cursor-not-allowed disabled:opacity-40"
                            >
                              Abandon before side effect
                            </button>
                            <button
                              type="button"
                              disabled={
                                busy || parsedEvidence === null || parsedEvidence.length === 0
                              }
                              onClick={() =>
                                void resolve(recovery, "accept_external_result_with_exact_evidence")
                              }
                              className="rounded border border-amber-500/35 px-1.5 py-1 disabled:cursor-not-allowed disabled:opacity-40"
                            >
                              Accept external result evidence
                            </button>
                            {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                          </div>
                        </div>
                      )}
                      {recovery.status === "decision_recorded" && (
                        <div className="mt-1 font-semibold">
                          Decision recorded; pending server convergence.
                        </div>
                      )}
                    </div>
                  );
                })}
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}
