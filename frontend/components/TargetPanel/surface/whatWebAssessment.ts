import type { AuditRow } from "@/lib/security-analysis";
import { parseWebOrigin } from "./surfaceHierarchy";

export type WhatWebAssessmentState =
  | "fingerprint_found"
  | "checked_empty"
  | "retry_pending"
  | "producer_blocked"
  | "state_error";

export interface WhatWebTransportEvidence {
  origin: string;
  attempt: number;
  failureClass: string;
  producerOutcome: "error" | "blocked";
  independentlyConfirmed: boolean;
}

export interface WhatWebAssessment {
  origin: string;
  state: WhatWebAssessmentState;
  observedAt: string;
  attempt: number | null;
  failureClass: string | null;
}

const WHATWEB_TECHNIQUE = "GOLISH-EAS-WEB-FINGERPRINT";

function recordValue(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function canonicalOrigin(value: unknown): string | null {
  return parseWebOrigin(stringValue(value))?.id ?? null;
}

export function parseWhatWebTransportEvidence(
  detail: Record<string, unknown>
): WhatWebTransportEvidence | null {
  if (detail.kind !== "eas.fingerprint_web_stack" || typeof detail.raw_output !== "string") {
    return null;
  }
  const origin = canonicalOrigin(detail.subject);
  if (!origin) return null;

  let raw: Record<string, unknown> | null = null;
  try {
    raw = recordValue(JSON.parse(detail.raw_output));
  } catch {
    return null;
  }
  if (!raw) return null;

  const failureClass = stringValue(raw.failure_class);
  const producerOutcome = stringValue(raw.producer_outcome);
  const attempt = raw.attempt;
  if (
    !failureClass ||
    (producerOutcome !== "error" && producerOutcome !== "blocked") ||
    typeof attempt !== "number" ||
    !Number.isInteger(attempt) ||
    attempt < 1 ||
    attempt > 3
  ) {
    return null;
  }

  return {
    origin,
    attempt,
    failureClass,
    producerOutcome,
    independentlyConfirmed: raw.independently_confirmed === true,
  };
}

function originForAudit(row: AuditRow): string | null {
  return canonicalOrigin(row.evidenceAsset) ?? canonicalOrigin(row.detail.subject);
}

function assessmentForAudit(row: AuditRow, origin: string): WhatWebAssessment | null {
  const observedAt = Number.isFinite(row.createdAt) ? new Date(row.createdAt).toISOString() : "";
  if (row.evidenceOutcome === "found" || row.evidenceOutcome === "checked_empty") {
    return {
      origin,
      state: row.evidenceOutcome === "found" ? "fingerprint_found" : "checked_empty",
      observedAt,
      attempt: null,
      failureClass: null,
    };
  }

  if (row.evidenceOutcome !== "error" && row.evidenceOutcome !== "blocked") return null;
  const transport = parseWhatWebTransportEvidence(row.detail);
  if (
    !transport ||
    transport.origin !== origin ||
    transport.producerOutcome !== row.evidenceOutcome ||
    (row.evidenceOutcome === "blocked" && transport.attempt !== 3)
  ) {
    return {
      origin,
      state: "state_error",
      observedAt,
      attempt: null,
      failureClass: null,
    };
  }

  return {
    origin,
    state: row.evidenceOutcome === "blocked" ? "producer_blocked" : "retry_pending",
    observedAt,
    attempt: transport.attempt,
    failureClass: transport.failureClass,
  };
}

/**
 * Builds a display-only producer assessment from target-bound evidence rows.
 * It deliberately does not infer downstream exclusion: that decision belongs
 * to the operation-state validator when Enumeration starts.
 */
export function buildWhatWebAssessments(
  logs: AuditRow[],
  knownOriginIds: ReadonlySet<string>
): ReadonlyMap<string, WhatWebAssessment> {
  const byOrigin = new Map<string, WhatWebAssessment>();
  const ordered = [...logs].sort((left, right) => right.createdAt - left.createdAt);

  for (const row of ordered) {
    if (row.auditRole !== "evidence" || row.evidenceTechnique !== WHATWEB_TECHNIQUE) continue;
    const origin = originForAudit(row);
    if (!origin || !knownOriginIds.has(origin) || byOrigin.has(origin)) continue;
    const assessment = assessmentForAudit(row, origin);
    if (assessment) byOrigin.set(origin, assessment);
  }

  return byOrigin;
}
