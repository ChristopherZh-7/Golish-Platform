import { AlertTriangle, CheckCircle2, Loader2, RotateCw, ShieldAlert } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ApiError } from "@/lib/api";
import {
  type AttackCandidateReviewState,
  listCandidateReviews,
  resumeCandidateReview,
  reviewCandidates,
} from "@/lib/api/attack";
import { translateErrorCode } from "@/lib/api/error-codes";
import { cn } from "@/lib/utils";

export interface AttackCandidateReviewProps {
  operationId: string;
  waveRunId: string;
  /** Harness traces only bump this refresh hint; DB remains authoritative. */
  refreshVersion?: number;
}

type ReviewDecision = "approve" | "reject";

function displayError(error: unknown): string {
  if (error instanceof ApiError) {
    return translateErrorCode(error.code, error.message);
  }
  return error instanceof Error ? error.message : String(error);
}

function titleCase(value: string): string {
  return value.length > 0 ? `${value[0].toUpperCase()}${value.slice(1)}` : value;
}

function planSummary(plan: unknown): { actions: string[]; budget: string | null } {
  const empty = { actions: [], budget: null };
  if (!plan || typeof plan !== "object" || Array.isArray(plan)) return empty;
  const record = plan as Record<string, unknown>;
  const actions = Array.isArray(record.actions)
    ? record.actions.flatMap((action) => {
        if (!action || typeof action !== "object" || Array.isArray(action)) return [];
        const value = action as Record<string, unknown>;
        const kind = typeof value.action_kind === "string" ? value.action_kind : "unknown_action";
        const capability =
          typeof value.capability_id === "string" ? ` · ${value.capability_id}` : "";
        return [`${kind}${capability}`];
      })
    : [];
  const budget =
    record.budget && typeof record.budget === "object" ? JSON.stringify(record.budget) : null;
  return { actions, budget };
}

export function AttackCandidateReview({
  operationId,
  waveRunId,
  refreshVersion = 0,
}: AttackCandidateReviewProps) {
  const [state, setState] = useState<AttackCandidateReviewState | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<"review" | "resume" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [decisions, setDecisions] = useState<Record<string, ReviewDecision>>({});
  const [startBefores, setStartBefores] = useState<Record<string, string>>({});
  const [expiries, setExpiries] = useState<Record<string, string>>({});

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await listCandidateReviews({ operationId, waveRunId });
      setState(next);
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

  const proposed = useMemo(
    () => state?.candidates.filter((candidate) => candidate.disposition === "proposed") ?? [],
    [state]
  );
  const allDecided =
    proposed.length > 0 &&
    proposed.every((candidate) => {
      const decision = decisions[candidate.candidateId];
      return (
        decision === "reject" ||
        (decision === "approve" &&
          Boolean(startBefores[candidate.candidateId]) &&
          Boolean(expiries[candidate.candidateId]) &&
          new Date(startBefores[candidate.candidateId]).getTime() <=
            new Date(expiries[candidate.candidateId]).getTime())
      );
    });

  const submitReview = async () => {
    if (!state || !allDecided) return;
    setBusy("review");
    setError(null);
    try {
      const response = await reviewCandidates({
        operationId,
        waveRunId,
        decisions: proposed.map((candidate) => {
          const decision = decisions[candidate.candidateId];
          return {
            candidateId: candidate.candidateId,
            candidatePlanHash: candidate.candidatePlanHash,
            expectedRowVersion: candidate.rowVersion,
            decision,
            startBefore:
              decision === "approve"
                ? new Date(startBefores[candidate.candidateId]).toISOString()
                : null,
            expiresAt:
              decision === "approve"
                ? new Date(expiries[candidate.candidateId]).toISOString()
                : null,
          };
        }),
      });
      setState(response.state);
    } catch (cause) {
      setError(displayError(cause));
    } finally {
      setBusy(null);
    }
  };

  const resume = async () => {
    if (!state) return;
    setBusy("resume");
    setError(null);
    try {
      const response = await resumeCandidateReview({
        operationId,
        waveRunId,
        expectedResumeVersion: state.resumeVersion,
      });
      setState(response.state);
    } catch (cause) {
      // Decisions are durable DB rows; a dispatcher failure must not erase them.
      setError(displayError(cause));
    } finally {
      setBusy(null);
    }
  };

  if (loading && !state) {
    return (
      <div className="flex items-center gap-2 rounded-md border border-border/30 px-3 py-4 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        Loading Candidate review…
      </div>
    );
  }

  if (!state) {
    return (
      <div className="rounded-md border border-red-500/30 bg-red-500/5 p-3 text-xs">
        <div role="alert" className="flex items-center gap-2 text-red-300">
          <AlertTriangle className="h-3.5 w-3.5" />
          {error ?? "Candidate review is unavailable."}
        </div>
        <button
          type="button"
          onClick={() => void load()}
          className="mt-2 text-accent hover:underline"
        >
          Retry DB read
        </button>
      </div>
    );
  }

  const canResume =
    state.reviewClosed && ["resume_pending", "resume_failed", "dispatching"].includes(state.status);

  return (
    <section className="space-y-3 rounded-md border border-border/30 bg-muted/10 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <ShieldAlert className="h-4 w-4 text-amber-300" />
        <h3 className="text-xs font-semibold text-foreground">Candidate review</h3>
        <span className="rounded border border-border/40 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
          {state.status}
        </span>
        <span className="ml-auto text-[10px] text-muted-foreground">
          {state.reviewClosedUnitCount}/{state.waveUnitCount} units closed
        </span>
      </div>

      <p className="text-[11px] leading-relaxed text-muted-foreground">
        Review immutable Candidate plans. Approval authorizes only the displayed plan hash,
        capability set, action, and budget.
      </p>

      {error && (
        <div
          role="alert"
          className="flex items-start gap-2 rounded border border-red-500/30 bg-red-500/5 p-2 text-[11px] text-red-300"
        >
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          {error}
        </div>
      )}

      {state.candidates.length === 0 ? (
        <div className="rounded border border-border/30 px-3 py-4 text-center text-xs text-muted-foreground">
          No Candidates were recorded for this wave.
        </div>
      ) : (
        <div className="space-y-2">
          {state.candidates.map((candidate) => {
            const approvalStatus = candidate.latestApproval?.status ?? candidate.disposition;
            const decided = candidate.disposition !== "proposed";
            const plan = planSummary(candidate.executionPlan);
            return (
              <article
                key={candidate.candidateId}
                data-testid={`candidate-${candidate.candidateId}`}
                className="rounded border border-border/30 bg-card/40 p-2.5"
              >
                <div className="flex flex-wrap items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-mono text-xs text-foreground">
                        {candidate.targetValueAtTime}
                      </span>
                      <span className="text-[10px] uppercase text-muted-foreground">
                        {candidate.targetTypeAtTime}
                      </span>
                      <span className="rounded border border-amber-500/30 px-1.5 py-0.5 text-[10px] text-amber-300">
                        {candidate.riskClass}
                      </span>
                    </div>
                    {!candidate.liveTargetPresent && (
                      <div className="mt-1 text-[10px] text-amber-300/90">
                        Live target removed · frozen identity
                      </div>
                    )}
                  </div>
                  {decided && (
                    <span className="flex items-center gap-1 text-[10px] text-emerald-300">
                      <CheckCircle2 className="h-3 w-3" />
                      {titleCase(approvalStatus)}
                    </span>
                  )}
                </div>

                <p className="mt-2 text-[11px] text-foreground/85">{candidate.hypothesis}</p>
                <p className="mt-1 text-[10px] text-muted-foreground">{candidate.rationale}</p>
                <dl className="mt-2 grid gap-1 rounded bg-muted/25 p-2 font-mono text-[10px] sm:grid-cols-2">
                  <div>
                    <dt className="text-muted-foreground">plan hash</dt>
                    <dd className="break-all text-foreground/80">{candidate.candidatePlanHash}</dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">actions</dt>
                    <dd className="break-all text-foreground/80">
                      {plan.actions.length > 0 ? plan.actions.join(", ") : "none"}
                    </dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">budget</dt>
                    <dd className="break-all text-foreground/80">{plan.budget ?? "none"}</dd>
                  </div>
                </dl>

                {!decided && (
                  <fieldset className="mt-2 flex gap-3 text-[11px]">
                    {(["approve", "reject"] as const).map((decision) => (
                      <label
                        key={decision}
                        className={cn(
                          "flex cursor-pointer items-center gap-1.5 rounded border px-2 py-1",
                          decisions[candidate.candidateId] === decision
                            ? "border-accent/70 bg-accent/10 text-foreground"
                            : "border-border/35 text-muted-foreground"
                        )}
                      >
                        <input
                          type="radio"
                          name={`decision-${candidate.candidateId}`}
                          aria-label={`${titleCase(decision)} ${candidate.targetValueAtTime}`}
                          checked={decisions[candidate.candidateId] === decision}
                          onChange={() => {
                            setDecisions((current) => ({
                              ...current,
                              [candidate.candidateId]: decision,
                            }));
                            if (decision === "approve" && !startBefores[candidate.candidateId]) {
                              setStartBefores((current) => ({
                                ...current,
                                [candidate.candidateId]: new Date(Date.now() + 60 * 60 * 1000)
                                  .toISOString()
                                  .slice(0, 16),
                              }));
                            }
                            if (decision === "approve" && !expiries[candidate.candidateId]) {
                              setExpiries((current) => ({
                                ...current,
                                [candidate.candidateId]: new Date(Date.now() + 2 * 60 * 60 * 1000)
                                  .toISOString()
                                  .slice(0, 16),
                              }));
                            }
                          }}
                        />
                        {titleCase(decision)}
                      </label>
                    ))}
                  </fieldset>
                )}
                {!decided && decisions[candidate.candidateId] === "approve" && (
                  <div className="mt-2 grid gap-2 sm:grid-cols-2">
                    <label className="block text-[10px] text-muted-foreground">
                      Must start by for {candidate.targetValueAtTime}
                      <input
                        type="datetime-local"
                        value={startBefores[candidate.candidateId] ?? ""}
                        onChange={(event) =>
                          setStartBefores((current) => ({
                            ...current,
                            [candidate.candidateId]: event.target.value,
                          }))
                        }
                        className="mt-1 block rounded border border-border/40 bg-background px-2 py-1 font-mono text-foreground"
                      />
                    </label>
                    <label className="block text-[10px] text-muted-foreground">
                      Approval expires for {candidate.targetValueAtTime}
                      <input
                        type="datetime-local"
                        value={expiries[candidate.candidateId] ?? ""}
                        onChange={(event) =>
                          setExpiries((current) => ({
                            ...current,
                            [candidate.candidateId]: event.target.value,
                          }))
                        }
                        className="mt-1 block rounded border border-border/40 bg-background px-2 py-1 font-mono text-foreground"
                      />
                    </label>
                  </div>
                )}
                {candidate.latestApproval && (
                  <div className="mt-2 text-[10px] text-muted-foreground">
                    Start before {new Date(candidate.latestApproval.startBefore).toLocaleString()} ·
                    Approval expires {new Date(candidate.latestApproval.expiresAt).toLocaleString()}
                  </div>
                )}
              </article>
            );
          })}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        {proposed.length > 0 && (
          <button
            type="button"
            onClick={() => void submitReview()}
            disabled={!allDecided || busy !== null}
            className="rounded bg-accent px-2.5 py-1.5 text-xs text-accent-foreground disabled:cursor-not-allowed disabled:opacity-40"
          >
            {busy === "review" ? "Submitting review…" : "Submit review"}
          </button>
        )}
        {canResume && (
          <button
            type="button"
            onClick={() => void resume()}
            disabled={busy !== null}
            className="flex items-center gap-1.5 rounded border border-emerald-500/40 bg-emerald-500/10 px-2.5 py-1.5 text-xs text-emerald-300 disabled:opacity-40"
          >
            {busy === "resume" ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <RotateCw className="h-3 w-3" />
            )}
            Resume verification
          </button>
        )}
        {loading && state && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />}
      </div>
    </section>
  );
}
