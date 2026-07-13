import { AlertTriangle, CheckCircle2, Loader2, RotateCw, ShieldAlert } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import {
  type CleanupCloseoutGateView,
  type CleanupObligationView,
  type CleanupWaiverSubmitRequest,
  getCleanupCloseoutGate,
  listCleanupObligations,
  waiveCleanupObligation,
} from "@/lib/api/cleanup";

export interface CleanupObligationListProps {
  operationId: string;
  organizationIdAtTime: string;
  refreshVersion?: number;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

interface WaiverDraft {
  reason: string;
  summary: string;
  severity: string;
  evidenceIds: string;
}

interface WaiverConfirmation {
  obligationId: string;
  request: CleanupWaiverSubmitRequest;
}

const EMPTY_DRAFT: WaiverDraft = {
  reason: "",
  summary: "",
  severity: "medium",
  evidenceIds: "",
};

function parseEvidenceIds(raw: string): number[] | null {
  const parts = raw.split(",").map((value) => value.trim());
  if (
    parts.length === 0 ||
    parts.some((value) => value.length === 0) ||
    parts.some((value) => !Number.isSafeInteger(Number(value)) || Number(value) <= 0)
  ) {
    return null;
  }
  const evidenceIds = parts.map(Number);
  return new Set(evidenceIds).size === evidenceIds.length ? evidenceIds : null;
}

export function CleanupObligationList({
  operationId,
  organizationIdAtTime,
  refreshVersion = 0,
}: CleanupObligationListProps) {
  const [obligations, setObligations] = useState<CleanupObligationView[] | null>(null);
  const [gate, setGate] = useState<CleanupCloseoutGateView | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [waiving, setWaiving] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, WaiverDraft>>({});
  const [confirmation, setConfirmation] = useState<WaiverConfirmation | null>(null);

  const request = { operationId, organizationIdAtTime };
  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [rows, closeout] = await Promise.all([
        listCleanupObligations(request),
        getCleanupCloseoutGate(request),
      ]);
      setObligations(rows);
      setGate(closeout);
      setConfirmation((current) => {
        if (current === null) return null;
        const live = rows.find((row) => row.obligationId === current.obligationId);
        const frozen = current.request;
        return live !== undefined &&
          live.operationId === frozen.operationId &&
          live.projectScopeId === frozen.projectScopeId &&
          live.scopeSnapshotId === frozen.scopeSnapshotId &&
          live.organizationIdAtTime === frozen.organizationIdAtTime &&
          live.rowVersion === frozen.expectedRowVersion
          ? current
          : null;
      });
    } catch (cause) {
      setError(message(cause));
    } finally {
      setLoading(false);
    }
  }, [operationId, organizationIdAtTime]);

  useEffect(() => {
    void refreshVersion;
    setDrafts({});
    setConfirmation(null);
    void load();
  }, [load, refreshVersion]);

  const updateDraft = (obligationId: string, patch: Partial<WaiverDraft>) => {
    setDrafts((current) => ({
      ...current,
      [obligationId]: { ...EMPTY_DRAFT, ...current[obligationId], ...patch },
    }));
  };

  const reviewWaiver = (obligation: CleanupObligationView) => {
    const draft = drafts[obligation.obligationId] ?? EMPTY_DRAFT;
    const evidenceIds = parseEvidenceIds(draft.evidenceIds);
    if (
      obligation.operationId !== operationId ||
      obligation.organizationIdAtTime !== organizationIdAtTime
    ) {
      setError("Cleanup obligation scope changed; refresh before waiving.");
      setConfirmation(null);
      return;
    }
    if (!draft.reason.trim() || !draft.summary.trim() || evidenceIds === null) {
      setError(
        "Waiver review requires a reason, residual summary, and unique positive evidence ids."
      );
      setConfirmation(null);
      return;
    }
    setError(null);
    setConfirmation({
      obligationId: obligation.obligationId,
      request: {
        waiverId: crypto.randomUUID(),
        obligationId: obligation.obligationId,
        operationId: obligation.operationId,
        projectScopeId: obligation.projectScopeId,
        scopeSnapshotId: obligation.scopeSnapshotId,
        organizationIdAtTime: obligation.organizationIdAtTime,
        expectedRowVersion: obligation.rowVersion,
        reason: draft.reason.trim(),
        residualSummary: draft.summary.trim(),
        residualSeverity: draft.severity,
        evidenceIds,
      },
    });
  };

  const submitWaiver = async (frozen: WaiverConfirmation) => {
    setWaiving(frozen.obligationId);
    setError(null);
    try {
      await waiveCleanupObligation(frozen.request);
      setDrafts((current) => {
        const next = { ...current };
        delete next[frozen.obligationId];
        return next;
      });
      setConfirmation(null);
      await load();
    } catch (cause) {
      setError(message(cause));
    } finally {
      setWaiving(null);
    }
  };

  if (loading && obligations === null) {
    return (
      <div className="flex items-center gap-2 rounded border border-border/30 p-3 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading cleanup obligations…
      </div>
    );
  }

  if (obligations === null || gate === null) {
    return (
      <div className="rounded border border-red-500/30 bg-red-500/5 p-3 text-xs">
        <div role="alert" className="flex items-center gap-2 text-red-300">
          <AlertTriangle className="h-3.5 w-3.5" /> {error ?? "Cleanup state is unavailable."}
        </div>
        <button
          type="button"
          className="mt-2 text-accent hover:underline"
          onClick={() => void load()}
        >
          Retry DB read
        </button>
      </div>
    );
  }

  return (
    <section className="space-y-3 rounded border border-border/30 bg-muted/10 p-3">
      <header className="flex flex-wrap items-center gap-2">
        {gate.allowsCloseout ? (
          <CheckCircle2 className="h-4 w-4 text-emerald-300" />
        ) : (
          <ShieldAlert className="h-4 w-4 text-amber-300" />
        )}
        <h3 className="text-xs font-semibold">Cleanup obligations</h3>
        <span className="ml-auto font-mono text-[10px] text-muted-foreground">
          missing {gate.missingObligationCount} · open {gate.nonterminalObligationCount} · residual{" "}
          {gate.undisclosedResidualCount} · invalid {gate.invalidTerminalTruthCount}
        </span>
        <button type="button" aria-label="Refresh cleanup obligations" onClick={() => void load()}>
          <RotateCw className="h-3.5 w-3.5" />
        </button>
      </header>

      {error && (
        <div role="alert" className="rounded border border-red-500/30 p-2 text-[11px] text-red-300">
          {error}
        </div>
      )}

      {obligations.length === 0 ? (
        <div className="rounded border border-border/30 p-4 text-center text-xs text-muted-foreground">
          No cleanup obligations were created for this operation and organization.
        </div>
      ) : (
        <div className="space-y-2">
          {obligations.map((obligation) => {
            const draft = drafts[obligation.obligationId] ?? EMPTY_DRAFT;
            const frozen =
              confirmation?.obligationId === obligation.obligationId ? confirmation : null;
            return (
              <article
                key={obligation.obligationId}
                className="rounded border border-border/30 p-2.5 text-[11px]"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono">{obligation.obligationId}</span>
                  <span className="rounded border border-border/30 px-1.5 py-0.5">
                    {obligation.status}
                  </span>
                  <span className="ml-auto text-muted-foreground">
                    deadline {obligation.deadline}
                  </span>
                </div>
                <pre className="mt-2 overflow-auto rounded bg-muted/20 p-2 text-[10px]">
                  {JSON.stringify(obligation.cleanupStrategy, null, 2)}
                </pre>
                {obligation.residualRisk !== null && (
                  <pre className="mt-2 overflow-auto rounded border border-amber-500/20 p-2 text-[10px] text-amber-200">
                    {JSON.stringify(obligation.residualRisk, null, 2)}
                  </pre>
                )}
                {["open", "in_progress"].includes(obligation.status) && (
                  <div className="mt-2 grid gap-2 sm:grid-cols-2">
                    <input
                      aria-label={`Waiver reason ${obligation.obligationId}`}
                      value={draft.reason}
                      onChange={(event) =>
                        updateDraft(obligation.obligationId, { reason: event.target.value })
                      }
                      placeholder="Operator waiver reason"
                    />
                    <input
                      aria-label={`Residual summary ${obligation.obligationId}`}
                      value={draft.summary}
                      onChange={(event) =>
                        updateDraft(obligation.obligationId, { summary: event.target.value })
                      }
                      placeholder="Residual risk summary"
                    />
                    <select
                      aria-label={`Residual severity ${obligation.obligationId}`}
                      value={draft.severity}
                      onChange={(event) =>
                        updateDraft(obligation.obligationId, { severity: event.target.value })
                      }
                    >
                      <option value="low">low</option>
                      <option value="medium">medium</option>
                      <option value="high">high</option>
                      <option value="critical">critical</option>
                    </select>
                    <input
                      aria-label={`Waiver evidence ${obligation.obligationId}`}
                      value={draft.evidenceIds}
                      onChange={(event) =>
                        updateDraft(obligation.obligationId, { evidenceIds: event.target.value })
                      }
                      placeholder="Evidence ids, comma separated"
                    />
                    <button
                      type="button"
                      aria-label={`Review waiver ${obligation.obligationId}`}
                      disabled={
                        waiving !== null ||
                        !draft.reason.trim() ||
                        !draft.summary.trim() ||
                        !draft.evidenceIds.trim()
                      }
                      onClick={() => reviewWaiver(obligation)}
                      className="rounded border border-amber-500/30 px-2 py-1 text-amber-200 disabled:opacity-50"
                    >
                      Review trusted waiver
                    </button>
                    {frozen !== null && (
                      <div className="space-y-2 rounded border border-amber-500/30 bg-amber-500/5 p-2 sm:col-span-2">
                        <div className="font-semibold text-amber-200">Confirm frozen waiver</div>
                        <p className="break-all font-mono text-[10px] text-muted-foreground">
                          operation {frozen.request.operationId} · project{" "}
                          {frozen.request.projectScopeId}
                          {" · snapshot "}
                          {frozen.request.scopeSnapshotId} · organization{" "}
                          {frozen.request.organizationIdAtTime} · row{" "}
                          {frozen.request.expectedRowVersion}
                        </p>
                        <p>
                          {frozen.request.reason} · {frozen.request.residualSummary} ·{" "}
                          {frozen.request.residualSeverity} · evidence{" "}
                          {frozen.request.evidenceIds.join(", ")}
                        </p>
                        <div className="flex gap-2">
                          <button
                            type="button"
                            aria-label={`Cancel waiver ${obligation.obligationId}`}
                            disabled={waiving !== null}
                            onClick={() => setConfirmation(null)}
                            className="rounded border border-border/40 px-2 py-1"
                          >
                            Cancel
                          </button>
                          <button
                            type="button"
                            aria-label={`Confirm waiver ${obligation.obligationId}`}
                            disabled={waiving !== null}
                            onClick={() => void submitWaiver(frozen)}
                            className="rounded border border-amber-500/40 px-2 py-1 text-amber-200 disabled:opacity-50"
                          >
                            {waiving === obligation.obligationId ? "Waiving…" : "Confirm waiver"}
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                )}
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}
