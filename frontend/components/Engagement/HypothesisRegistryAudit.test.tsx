import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  HypothesisRegistryAudit,
  type HypothesisRegistryAuditApi,
} from "./HypothesisRegistryAudit";

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T) => void;
};

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

const MODES = [
  "legacy_only",
  "shadow_registry",
  "dual_read_compare",
  "registry_authoritative_legacy_projection",
  "new_only",
] as const;
const SESSION_ID = "session-1";

function envelope(mode = "registry_authoritative_legacy_projection") {
  return {
    projectionSchemaVersion: 1,
    changeSeq: 42,
    readAt: "2026-07-29T12:00:00Z",
    temporalSnapshot: {
      contractVersion: 2,
      asOfTemporalCutoff: "2026-07-29T12:00:00Z",
      authorityEpochSetHash: "epoch-hash",
      earliestEffectiveValidUntil: "2026-07-29T13:00:00Z",
    },
    toolTruthContract: "checked_tool_truth_v2",
    investigationContractVersion: "hypothesis_registry_v1",
    investigationRolloutMode: mode,
    modePolicy: {
      canonicalWriter: "registry",
      gateAuthority: "registry",
      allowLegacyMutation: false,
      campaignWritePolicy: "forbidden",
      allowPreparedActionJit: false,
      comparePolicy: "none",
      legacyProjectionPolicy: "project",
    },
    nextCursor: null,
  };
}

function summary(mode = "registry_authoritative_legacy_projection") {
  return {
    envelope: envelope(mode),
    activeGenerationId: "generation-1",
    activeGenerationSealHash: "seal-123",
    currentHypothesisCount: 2,
    closedHypothesisCount: 1,
    contestedHypothesisCount: 1,
    residualCount: 2,
    generations: [{ generationId: "generation-1", generationOrdinal: 1, state: "active" }],
    waves: [{ waveId: "wave-1", waveOrdinal: 1, state: "active" }],
    openObligations: [{ obligationId: "obligation-1", obligationKind: "verification" }],
    coverageDenominator: {
      planned: 2,
      testedComplete: 0,
      testedDegraded: 0,
      untested: 2,
      blocked: 0,
    },
    coverageSufficiency: "not_assessed",
    authorityTimeMembers: [
      {
        observedAsOf: "2026-07-29T12:00:00Z",
        effectiveValidUntil: "2026-07-29T13:00:00Z",
        authorityEpochHash: "epoch-hash",
        temporalStatus: "current",
      },
    ],
    controlDecision: "continue",
    coverageGrade: "partial",
  };
}

function hypothesis(
  revisionId: string,
  overrides: Record<string, unknown> = {}
) {
  return {
    rootId: `root-${revisionId}`,
    revisionId,
    organizationId: "organization-1",
    subjectKind: "target_at_time",
    subjectIdentityHash: `subject-${revisionId}`,
    targetTypeAtTime: "url",
    targetValueAtTime: `https://${revisionId}.example.test`,
    predicateSchema: "http_service_exposure_v1",
    predicateSummary: `Predicate ${revisionId}`,
    trustBoundary: "external",
    polarity: "positive",
    epistemicState: "contested",
    lifecycleState: "current",
    planningReadiness: "blocked",
    supportCount: 3,
    contradictionCount: 2,
    gapCount: 1,
    legacyProjectionStatus: "unsupported",
    residualCodes: ["plan_c_verification_unavailable"],
    ...overrides,
  };
}

function apiWith(overrides: Partial<HypothesisRegistryAuditApi> = {}): HypothesisRegistryAuditApi {
  const item = hypothesis("revision-1");
  return {
    getSummary: vi.fn().mockResolvedValue(summary()),
    listHypotheses: vi.fn().mockResolvedValue({
      envelope: envelope(),
      hypotheses: [item],
    }),
    getHypothesis: vi.fn().mockResolvedValue({
      envelope: envelope(),
      hypothesis: item,
      predecessorRevisionId: null,
      lineageRevisionIds: [],
      supportRefIds: ["support-1"],
      contradictionRefIds: ["conflict-1"],
      applicationContextRefIds: [],
      gapRefIds: ["gap-1"],
      verificationObjectiveSummaries: ["Confirm the exact external behavior"],
      legacyUnavailableFields: ["attempt_terminal_authority"],
    }),
    ...overrides,
  };
}

describe("HypothesisRegistryAudit", () => {
  it("renders independent first-load states for summary and hypotheses", () => {
    const api = apiWith({
      getSummary: vi.fn().mockReturnValue(new Promise(() => undefined)),
      listHypotheses: vi.fn().mockReturnValue(new Promise(() => undefined)),
    });

    render(<HypothesisRegistryAudit sessionId={SESSION_ID} operationId="operation-1" api={api} />);

    expect(screen.getByText("Loading registry summary…")).toBeInTheDocument();
    expect(screen.getByText("Loading hypotheses…")).toBeInTheDocument();
    expect(api.getSummary).toHaveBeenCalledWith({
      sessionId: SESSION_ID,
      operationId: "operation-1",
    });
    expect(api.listHypotheses).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: SESSION_ID, operationId: "operation-1" })
    );
  });

  it("keeps summary and list errors independently retryable", async () => {
    const summaryCall = vi
      .fn()
      .mockRejectedValueOnce(new Error("summary unavailable"))
      .mockResolvedValueOnce(summary());
    const listCall = vi
      .fn()
      .mockRejectedValueOnce(new Error("list unavailable"))
      .mockResolvedValueOnce({ envelope: envelope(), hypotheses: [] });
    const api = apiWith({ getSummary: summaryCall, listHypotheses: listCall });

    render(<HypothesisRegistryAudit sessionId={SESSION_ID} operationId="operation-1" api={api} />);

    expect(await screen.findByText("summary unavailable")).toBeInTheDocument();
    expect(await screen.findByText("list unavailable")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry summary" }));
    fireEvent.click(screen.getByRole("button", { name: "Retry hypotheses" }));

    expect(await screen.findByText("seal-123")).toBeInTheDocument();
    expect(await screen.findByText("No hypotheses in this projection.")).toBeInTheDocument();
    expect(summaryCall).toHaveBeenCalledTimes(2);
    expect(listCall).toHaveBeenCalledTimes(2);
  });

  it("renders independent summary and hypothesis empty states", async () => {
    const api = apiWith({
      getSummary: vi.fn().mockResolvedValue({
        ...summary(),
        activeGenerationId: null,
        activeGenerationSealHash: null,
        currentHypothesisCount: 0,
        closedHypothesisCount: 0,
        contestedHypothesisCount: 0,
        residualCount: 0,
      }),
      listHypotheses: vi.fn().mockResolvedValue({ envelope: envelope(), hypotheses: [] }),
    });

    render(
      <HypothesisRegistryAudit sessionId={SESSION_ID} operationId="operation-empty" api={api} />
    );

    expect(await screen.findByText("No active hypothesis generation.")).toBeVisible();
    expect(screen.getByText("No hypotheses in this projection.")).toBeVisible();
  });

  it.each(MODES)("renders the %s rollout badge", async (mode) => {
    const api = apiWith({
      getSummary: vi.fn().mockResolvedValue(summary(mode)),
      listHypotheses: vi.fn().mockResolvedValue({ envelope: envelope(mode), hypotheses: [] }),
    });

    render(
      <HypothesisRegistryAudit
        sessionId={SESSION_ID}
        operationId={`operation-${mode}`}
        api={api}
      />
    );

    expect(await screen.findByText(mode)).toBeVisible();
  });

  it("keeps old data visible and marks it stale during refresh", async () => {
    const nextSummary = deferred<ReturnType<typeof summary>>();
    const nextList = deferred<{ envelope: ReturnType<typeof envelope>; hypotheses: never[] }>();
    const summaryCall = vi
      .fn()
      .mockResolvedValueOnce(summary())
      .mockReturnValueOnce(nextSummary.promise);
    const listCall = vi
      .fn()
      .mockResolvedValueOnce({ envelope: envelope(), hypotheses: [hypothesis("revision-1")] })
      .mockReturnValueOnce(nextList.promise);
    const api = apiWith({ getSummary: summaryCall, listHypotheses: listCall });

    render(<HypothesisRegistryAudit sessionId={SESSION_ID} operationId="operation-1" api={api} />);
    expect(await screen.findByText("Predicate revision-1")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Refresh audit" }));

    expect(screen.getByText("Predicate revision-1")).toBeInTheDocument();
    expect(screen.getByText("stale")).toBeVisible();
    await act(async () => {
      nextSummary.resolve(summary());
      nextList.resolve({ envelope: envelope(), hypotheses: [] });
    });
    expect(await screen.findByText("No hypotheses in this projection.")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("stale")).not.toBeInTheDocument());
  });

  it("shows compatibility limits, Plan C residuals, at-time subject and detail loading", async () => {
    const detail = deferred<Awaited<ReturnType<HypothesisRegistryAuditApi["getHypothesis"]>>>();
    const legacy = hypothesis("revision-legacy", {
      predicateSummary: "Legacy projection",
      legacyProjectionStatus: null,
      residualCodes: [],
    });
    const unsupported = hypothesis("revision-unsupported");
    const api = apiWith({
      listHypotheses: vi.fn().mockResolvedValue({
        envelope: envelope(),
        hypotheses: [legacy, unsupported],
      }),
      getHypothesis: vi.fn().mockReturnValue(detail.promise),
    });

    render(<HypothesisRegistryAudit sessionId={SESSION_ID} operationId="operation-1" api={api} />);

    expect(await screen.findByText("legacy_unavailable")).toBeVisible();
    expect(screen.getByText("unsupported")).toBeVisible();
    expect(screen.getByText("plan_c_verification_unavailable")).toBeVisible();
    expect(
      screen.getByRole("button", { name: /Open Predicate revision-unsupported/ })
    ).toHaveTextContent("https://revision-unsupported.example.test");
    expect(screen.queryByText(/Queue \d+/)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Open Predicate revision-unsupported/ }));
    expect(api.getHypothesis).toHaveBeenCalledWith({
      sessionId: SESSION_ID,
      operationId: "operation-1",
      revisionId: "revision-unsupported",
    });
    expect(screen.getByText("Loading hypothesis detail…")).toBeInTheDocument();
    await act(async () => {
      detail.resolve({
        envelope: envelope(),
        hypothesis: unsupported,
        predecessorRevisionId: null,
        lineageRevisionIds: ["revision-before"],
        supportRefIds: ["support-1"],
        contradictionRefIds: ["conflict-1"],
        applicationContextRefIds: [],
        gapRefIds: ["gap-1"],
        verificationObjectiveSummaries: ["Confirm the exact external behavior"],
        legacyUnavailableFields: ["attempt_terminal_authority"],
      });
    });
    expect(await screen.findByText("Confirm the exact external behavior")).toBeVisible();
    expect(screen.getByText("attempt_terminal_authority")).toBeVisible();
  });

  it("does not let a late response from the old operation overwrite the new one", async () => {
    const oldSummary = deferred<ReturnType<typeof summary>>();
    const oldList = deferred<{
      envelope: ReturnType<typeof envelope>;
      hypotheses: ReturnType<typeof hypothesis>[];
    }>();
    const api = apiWith({
      getSummary: vi.fn(({ operationId }) =>
        operationId === "operation-old"
          ? oldSummary.promise
          : Promise.resolve({ ...summary(), activeGenerationSealHash: "seal-new" })
      ),
      listHypotheses: vi.fn(({ operationId }) =>
        operationId === "operation-old"
          ? oldList.promise
          : Promise.resolve({
              envelope: envelope(),
              hypotheses: [hypothesis("revision-new")],
            })
      ),
    });
    const view = render(
      <HypothesisRegistryAudit sessionId={SESSION_ID} operationId="operation-old" api={api} />
    );

    view.rerender(
      <HypothesisRegistryAudit sessionId={SESSION_ID} operationId="operation-new" api={api} />
    );
    expect(await screen.findByText("seal-new")).toBeVisible();
    expect(await screen.findByText("Predicate revision-new")).toBeVisible();

    await act(async () => {
      oldSummary.resolve({ ...summary(), activeGenerationSealHash: "seal-old" });
      oldList.resolve({
        envelope: envelope(),
        hypotheses: [hypothesis("revision-old")],
      });
    });
    expect(screen.queryByText("seal-old")).not.toBeInTheDocument();
    expect(screen.queryByText("Predicate revision-old")).not.toBeInTheDocument();
  });

  it("does not let a late response from another session overwrite the same operation", async () => {
    const oldSummary = deferred<ReturnType<typeof summary>>();
    const oldList = deferred<{
      envelope: ReturnType<typeof envelope>;
      hypotheses: ReturnType<typeof hypothesis>[];
    }>();
    const api = apiWith({
      getSummary: vi.fn(({ sessionId }) =>
        sessionId === "session-old"
          ? oldSummary.promise
          : Promise.resolve({ ...summary(), activeGenerationSealHash: "seal-new-session" })
      ),
      listHypotheses: vi.fn(({ sessionId }) =>
        sessionId === "session-old"
          ? oldList.promise
          : Promise.resolve({
              envelope: envelope(),
              hypotheses: [hypothesis("revision-new-session")],
            })
      ),
    });
    const view = render(
      <HypothesisRegistryAudit sessionId="session-old" operationId="operation-1" api={api} />
    );

    view.rerender(
      <HypothesisRegistryAudit sessionId="session-new" operationId="operation-1" api={api} />
    );
    expect(await screen.findByText("seal-new-session")).toBeVisible();
    expect(await screen.findByText("Predicate revision-new-session")).toBeVisible();

    await act(async () => {
      oldSummary.resolve({ ...summary(), activeGenerationSealHash: "seal-old-session" });
      oldList.resolve({
        envelope: envelope(),
        hypotheses: [hypothesis("revision-old-session")],
      });
    });
    expect(screen.queryByText("seal-old-session")).not.toBeInTheDocument();
    expect(screen.queryByText("Predicate revision-old-session")).not.toBeInTheDocument();
  });
});
