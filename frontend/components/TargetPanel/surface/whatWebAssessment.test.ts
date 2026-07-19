import { describe, expect, it } from "vitest";
import type { AuditRow } from "@/lib/security-analysis";
import {
  buildWhatWebAssessments,
  parseWhatWebTransportEvidence,
} from "./whatWebAssessment";

const ORIGIN_A = "http://123.6.40.244:8000";
const ORIGIN_B = "http://123.6.40.244:8088";

function audit(
  origin: string,
  evidenceOutcome: string,
  createdAt: number,
  rawOutput: string
): AuditRow {
  return {
    id: createdAt,
    action: "whatweb_completed",
    category: "evidence",
    details: `[eas.fingerprint_web_stack] ${origin}`,
    entityType: null,
    entityId: null,
    source: "pentest_run",
    projectPath: "/workspace",
    targetId: "target-1",
    sessionId: "session-1",
    toolName: "whatweb",
    status: "completed",
    detail: {
      kind: "eas.fingerprint_web_stack",
      subject: origin,
      raw_output: rawOutput,
    },
    auditRole: "evidence",
    evidenceTechnique: "GOLISH-EAS-WEB-FINGERPRINT",
    evidenceOutcome,
    evidenceAsset: origin,
    createdAt,
  };
}

function transportFailure(origin: string, attempt: number, createdAt = attempt): AuditRow {
  return audit(
    origin,
    attempt === 3 ? "blocked" : "error",
    createdAt,
    JSON.stringify({
      failure_class: "connection_reset",
      attempt,
      producer_outcome: attempt === 3 ? "blocked" : "error",
      independently_confirmed: attempt === 3,
    })
  );
}

describe("parseWhatWebTransportEvidence", () => {
  it("parses the bounded exact-origin attempt payload", () => {
    expect(parseWhatWebTransportEvidence(transportFailure(ORIGIN_A, 2).detail)).toEqual({
      origin: ORIGIN_A,
      attempt: 2,
      failureClass: "connection_reset",
      producerOutcome: "error",
      independentlyConfirmed: false,
    });
  });

  it("rejects malformed payloads and attempts outside the producer contract", () => {
    expect(
      parseWhatWebTransportEvidence({
        kind: "eas.fingerprint_web_stack",
        subject: ORIGIN_A,
        raw_output: JSON.stringify({
          failure_class: "connection_reset",
          attempt: 4,
          producer_outcome: "blocked",
        }),
      })
    ).toBeNull();
    expect(
      parseWhatWebTransportEvidence({
        kind: "eas.fingerprint_web_stack",
        subject: "123.6.40.244",
        raw_output: "not json",
      })
    ).toBeNull();
  });
});

describe("buildWhatWebAssessments", () => {
  it.each([
    [1, "retry_pending"],
    [2, "retry_pending"],
    [3, "producer_blocked"],
  ] as const)("maps attempt %s to %s without claiming downstream exclusion", (attempt, state) => {
    const result = buildWhatWebAssessments(
      [transportFailure(ORIGIN_A, attempt)],
      new Set([ORIGIN_A])
    );
    expect(result.get(ORIGIN_A)).toMatchObject({ state, attempt });
  });

  it("uses evidence_outcome authority for found and checked-empty", () => {
    const result = buildWhatWebAssessments(
      [
        audit(ORIGIN_A, "found", 2, `${ORIGIN_A} [200 OK] HTTPServer[openresty]`),
        audit(ORIGIN_B, "checked_empty", 1, ""),
      ],
      new Set([ORIGIN_A, ORIGIN_B])
    );
    expect(result.get(ORIGIN_A)?.state).toBe("fingerprint_found");
    expect(result.get(ORIGIN_B)?.state).toBe("checked_empty");
  });

  it("renders inconsistent terminal evidence as an error, never as a pass state", () => {
    const result = buildWhatWebAssessments(
      [audit(ORIGIN_A, "blocked", 1, "malformed")],
      new Set([ORIGIN_A])
    );
    expect(result.get(ORIGIN_A)?.state).toBe("state_error");
  });

  it("keeps exact origins isolated and lets newer evidence replace older evidence", () => {
    const result = buildWhatWebAssessments(
      [
        transportFailure(ORIGIN_A, 2, 1),
        audit(ORIGIN_A, "found", 3, `${ORIGIN_A} [200 OK]`),
        transportFailure(ORIGIN_B, 3, 2),
        transportFailure("https://other.example:443", 3, 4),
      ],
      new Set([ORIGIN_A, ORIGIN_B])
    );
    expect(result.get(ORIGIN_A)?.state).toBe("fingerprint_found");
    expect(result.get(ORIGIN_B)?.state).toBe("producer_blocked");
    expect(result.has("https://other.example:443")).toBe(false);
  });

  it("ignores rows without evidence-ledger authority", () => {
    const row = transportFailure(ORIGIN_A, 3);
    row.auditRole = null;
    expect(buildWhatWebAssessments([row], new Set([ORIGIN_A])).size).toBe(0);
  });
});
