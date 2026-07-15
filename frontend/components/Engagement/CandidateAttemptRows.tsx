import {
  AlertTriangle,
  CheckCircle2,
  CircleDot,
  Clock3,
  Loader2,
  RotateCw,
  ShieldAlert,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ApiError } from "@/lib/api";
import { type CandidateAttemptRow, listCandidateAttempts } from "@/lib/api/attack";
import { translateErrorCode } from "@/lib/api/error-codes";
import { cn } from "@/lib/utils";
import { CandidateVerificationProtocol } from "./CandidateVerificationProtocol";

export interface CandidateAttemptRowsProps {
  operationId: string;
  waveRunId: string;
  /** Harness traces only request a refresh; the DB API remains authoritative. */
  refreshVersion?: number;
}

type EvidenceRole = "proof" | "refutation" | "blocker";

interface FindingSummary {
  title: string;
  severity: string | null;
}

interface AttemptResultSummary {
  evidence: Array<{ role: EvidenceRole; ids: number[] }>;
  blockerReason: string | null;
  retryReason: string | null;
  finding: FindingSummary | null;
}

const STATUS_LABELS: Record<string, string> = {
  queued: "Queued",
  running: "Running",
  submitted: "Submitted",
  terminalization_pending: "Terminalization pending",
  verified: "Verified",
  refuted: "Refuted",
  blocked: "Blocked",
  retryable_failed: "Retryable failure",
  abandoned: "Abandoned",
};

const ROLE_LABELS: Record<EvidenceRole, string> = {
  proof: "Proof",
  refutation: "Refutation",
  blocker: "Blocker",
};

function displayError(error: unknown): string {
  if (error instanceof ApiError) {
    return translateErrorCode(error.code, error.message);
  }
  return error instanceof Error ? error.message : String(error);
}

function objectValue(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function positiveEvidenceIds(value: unknown): number[] {
  if (!Array.isArray(value)) return [];
  return Array.from(
    new Set(
      value.filter(
        (candidate): candidate is number =>
          typeof candidate === "number" && Number.isSafeInteger(candidate) && candidate > 0
      )
    )
  ).sort((left, right) => left - right);
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

function resultSummary(result: unknown): AttemptResultSummary {
  const value = objectValue(result);
  if (value === null) {
    return { evidence: [], blockerReason: null, retryReason: null, finding: null };
  }

  const evidence = (
    [
      ["proof", value.proof_evidence_ids ?? value.proofEvidenceIds],
      ["refutation", value.refutation_evidence_ids ?? value.refutationEvidenceIds],
      ["blocker", value.blocker_evidence_ids ?? value.blockerEvidenceIds],
    ] as const
  ).flatMap(([role, rawIds]) => {
    const ids = positiveEvidenceIds(rawIds);
    return ids.length > 0 ? [{ role, ids }] : [];
  });
  const finding = objectValue(value.finding);
  const findingTitle = finding === null ? null : optionalString(finding.title);

  return {
    evidence,
    blockerReason: optionalString(value.blocker_reason_code ?? value.blockerReasonCode),
    retryReason: optionalString(value.reason_code ?? value.reasonCode),
    finding:
      findingTitle === null
        ? null
        : {
            title: findingTitle,
            severity: optionalString(finding?.severity),
          },
  };
}

function statusIcon(status: string) {
  if (status === "verified") return <CheckCircle2 className="h-3.5 w-3.5 text-emerald-300" />;
  if (status === "refuted") return <XCircle className="h-3.5 w-3.5 text-sky-300" />;
  if (status === "blocked" || status === "retryable_failed") {
    return <ShieldAlert className="h-3.5 w-3.5 text-amber-300" />;
  }
  if (["running", "submitted", "terminalization_pending"].includes(status)) {
    return <Loader2 className="h-3.5 w-3.5 animate-spin text-accent" />;
  }
  return <Clock3 className="h-3.5 w-3.5 text-muted-foreground" />;
}

function statusLabel(status: string): string {
  return STATUS_LABELS[status] ?? status;
}

function AttemptRow({ attempt }: { attempt: CandidateAttemptRow }) {
  const result = resultSummary(attempt.result);
  const isActive = ["running", "submitted", "terminalization_pending"].includes(attempt.status);

  return (
    <article
      data-testid={`attempt-${attempt.attemptId}`}
      className="rounded border border-border/30 bg-card/40 p-2.5 text-[11px]"
    >
      <div className="flex flex-wrap items-start gap-2">
        {statusIcon(attempt.status)}
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-semibold text-foreground">Attempt {attempt.ordinal}</span>
            <span
              className={cn(
                "rounded border px-1.5 py-0.5 text-[10px]",
                attempt.status === "verified" &&
                  "border-emerald-500/30 bg-emerald-500/5 text-emerald-300",
                attempt.status === "refuted" && "border-sky-500/30 bg-sky-500/5 text-sky-300",
                ["blocked", "retryable_failed"].includes(attempt.status) &&
                  "border-amber-500/30 bg-amber-500/5 text-amber-300",
                ["queued", "abandoned"].includes(attempt.status) &&
                  "border-border/40 text-muted-foreground",
                isActive && "border-accent/40 bg-accent/5 text-accent"
              )}
            >
              {statusLabel(attempt.status)}
            </span>
            {isActive && <span className="text-[10px] text-accent">Exploit lane active</span>}
          </div>
          <div className="mt-1 break-all font-mono text-xs text-foreground/90">
            {attempt.targetValueAtTime}
          </div>
          <div className="mt-0.5 flex flex-wrap gap-x-2 text-[10px] text-muted-foreground">
            <span>{attempt.targetTypeAtTime}</span>
            <span>Candidate {attempt.candidateId}</span>
            {attempt.targetLiveId === null && <span>frozen identity · live target removed</span>}
          </div>
        </div>
        <time className="text-[10px] text-muted-foreground">
          {attempt.terminalAt ?? attempt.updatedAt}
        </time>
      </div>

      <dl className="mt-2 grid gap-1 rounded bg-muted/20 p-2 font-mono text-[10px] sm:grid-cols-2">
        <div>
          <dt className="text-muted-foreground">attempt id</dt>
          <dd className="break-all text-foreground/80">{attempt.attemptId}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">plan hash</dt>
          <dd className="break-all text-foreground/80">{attempt.candidatePlanHash}</dd>
        </div>
      </dl>

      {result.evidence.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {result.evidence.flatMap(({ role, ids }) =>
            ids.map((evidenceId) => (
              <a
                key={`${role}-${evidenceId}`}
                href={`#evidence-${evidenceId}`}
                aria-label={`${ROLE_LABELS[role]} evidence #${evidenceId}`}
                data-evidence-role={role}
                className="rounded border border-border/35 px-1.5 py-0.5 text-[10px] text-accent hover:underline"
              >
                {ROLE_LABELS[role]} evidence #{evidenceId}
              </a>
            ))
          )}
        </div>
      )}

      {attempt.status === "verified" && result.finding !== null && (
        <div className="mt-2 rounded border border-emerald-500/25 bg-emerald-500/5 p-2">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-semibold text-emerald-300">Finding lineage</span>
            {result.finding.severity !== null && (
              <span className="rounded border border-emerald-500/25 px-1.5 py-0.5 text-[10px] uppercase text-emerald-200">
                {result.finding.severity}
              </span>
            )}
          </div>
          <div className="mt-1 text-foreground/90">{result.finding.title}</div>
          <div className="mt-1 break-all font-mono text-[10px] text-muted-foreground">
            {attempt.candidateId} → {attempt.attemptId} → Finding
          </div>
        </div>
      )}

      {attempt.status === "blocked" && (
        <div className="mt-2 rounded border border-amber-500/25 bg-amber-500/5 p-2 text-amber-200">
          <div className="font-semibold">Residual risk remains</div>
          <div className="mt-0.5 font-mono text-[10px]">
            {result.blockerReason ?? "blocked_with_evidence"}
          </div>
        </div>
      )}

      {attempt.status === "retryable_failed" && result.retryReason !== null && (
        <div className="mt-2 rounded border border-amber-500/25 p-2 font-mono text-[10px] text-amber-200">
          {result.retryReason}
        </div>
      )}
    </article>
  );
}

export function CandidateAttemptRows({
  operationId,
  waveRunId,
  refreshVersion = 0,
}: CandidateAttemptRowsProps) {
  const [attempts, setAttempts] = useState<CandidateAttemptRow[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setAttempts(await listCandidateAttempts({ operationId, waveRunId }));
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

  const counts = useMemo(() => {
    const rows = attempts ?? [];
    return {
      active: rows.filter((attempt) =>
        ["running", "submitted", "terminalization_pending"].includes(attempt.status)
      ).length,
      queued: rows.filter((attempt) => attempt.status === "queued").length,
    };
  }, [attempts]);

  if (loading && attempts === null) {
    return (
      <div className="space-y-2">
        <CandidateVerificationProtocol
          operationId={operationId}
          waveRunId={waveRunId}
          refreshVersion={refreshVersion}
        />
        <div className="flex items-center gap-2 rounded border border-border/30 p-3 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading Candidate attempts…
        </div>
      </div>
    );
  }

  if (attempts === null) {
    return (
      <div className="space-y-2">
        <CandidateVerificationProtocol
          operationId={operationId}
          waveRunId={waveRunId}
          refreshVersion={refreshVersion}
        />
        <div className="rounded border border-red-500/30 bg-red-500/5 p-3 text-xs">
          <div role="alert" className="flex items-center gap-2 text-red-300">
            <AlertTriangle className="h-3.5 w-3.5" />
            {error ?? "Candidate attempts are unavailable."}
          </div>
          <button
            type="button"
            className="mt-2 text-accent hover:underline"
            onClick={() => void load()}
          >
            Retry DB read
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <CandidateVerificationProtocol
        operationId={operationId}
        waveRunId={waveRunId}
        refreshVersion={refreshVersion}
      />
      <section className="space-y-3 rounded border border-border/30 bg-muted/10 p-3">
        <header className="flex flex-wrap items-center gap-2">
          <CircleDot className="h-4 w-4 text-accent" />
          <h3 className="text-xs font-semibold">Candidate verification attempts</h3>
          <span className="ml-auto font-mono text-[10px] text-muted-foreground">
            {counts.active} active · {counts.queued} queued
          </span>
          <button type="button" aria-label="Refresh Candidate attempts" onClick={() => void load()}>
            <RotateCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
          </button>
        </header>

        {error && (
          <div
            role="alert"
            className="rounded border border-red-500/30 p-2 text-[11px] text-red-300"
          >
            {error}
          </div>
        )}

        {attempts.length === 0 ? (
          <div className="rounded border border-border/30 p-4 text-center text-xs text-muted-foreground">
            No Candidate verification attempts were recorded for this wave.
          </div>
        ) : (
          <div className="space-y-2">
            {attempts.map((attempt) => (
              <AttemptRow key={attempt.attemptId} attempt={attempt} />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
