import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { type ReportingViewApi, ReportReadModelView } from "./ReportReadModelView";

function validatedModel() {
  return {
    reportId: "report-1",
    operationId: "operation-1",
    projectScopeId: "project-1",
    scopeSnapshotId: "snapshot-1",
    scopeSnapshotHash: "a".repeat(64),
    current: {
      revisionId: "revision-2",
      revisionNumber: 2,
      rowVersion: 3,
      sourceSetHash: "b".repeat(64),
      validationStatus: "validated",
      publicationStatus: "unpublished",
      supersedesRevisionId: "revision-1",
      validatedAt: "2026-07-13T00:00:00Z",
      finalizedAt: null,
    },
    revisions: [],
    sections: [
      {
        sectionId: "section-1",
        organizationIdAtTime: "organization-1",
        organizationNameAtSnapshot: "Acme",
        sectionKind: "findings",
        ordinal: 0,
        renderedContent: null,
        claims: [
          {
            claimId: "claim-1",
            claimKind: "candidate_disposition",
            subjectRef: "candidate-1",
            predicate: "verified",
            value: { severity: "high" },
            ordinal: 0,
            citations: [
              {
                citationId: "citation-1",
                sourceKind: "finding_lineage",
                sourceIdKind: "uuid",
                sourceIdValue: "lineage-1",
                sourceRowVersion: 4,
                sourceHash: "c".repeat(64),
                evidenceAuditId: 41,
                organizationIdAtTime: "organization-1",
                displayLabel: "Verified lineage",
              },
            ],
          },
        ],
      },
    ],
    artifacts: [],
  };
}

function modelForOperation(operationId: string, revisionId: string, rowVersion: number) {
  const model = validatedModel();
  return {
    ...model,
    operationId,
    current: model.current
      ? {
          ...model.current,
          revisionId,
          rowVersion,
          sourceSetHash: revisionId.padEnd(64, "0").slice(0, 64),
        }
      : null,
  };
}

describe("ReportReadModelView", () => {
  it("renders citation lineage and finalizes only after explicit confirmation", async () => {
    const model = validatedModel();
    const final = {
      ...model,
      current: { ...model.current, publicationStatus: "final" },
      artifacts: [
        {
          revisionId: "revision-2",
          artifactKind: "markdown",
          contentKey: `sha256/${"d".repeat(64)}.md`,
          sha256: "d".repeat(64),
          byteLen: 99,
          redactionVersion: 1,
        },
      ],
    };
    const api: ReportingViewApi = {
      getReadModel: vi.fn().mockResolvedValue(model),
      buildReadModel: vi.fn().mockResolvedValue(model),
      finalizeRevision: vi.fn().mockResolvedValue(final),
    } as never;
    render(<ReportReadModelView operationId="operation-1" api={api} />);
    expect(await screen.findByText("Acme")).toBeInTheDocument();
    expect(screen.getByText(/Evidence 41/)).toHaveTextContent("finding_lineage:lineage-1@v4");
    fireEvent.click(screen.getByRole("button", { name: "Finalize report" }));
    expect(api.finalizeRevision).not.toHaveBeenCalled();
    expect(screen.getByRole("status")).toHaveTextContent("revision-2");
    expect(screen.getByRole("status")).toHaveTextContent("row version 3");
    fireEvent.click(screen.getByRole("button", { name: "Confirm final publish" }));
    await waitFor(() => expect(api.finalizeRevision).toHaveBeenCalledTimes(1));
    expect(api.finalizeRevision).toHaveBeenCalledWith({
      operationId: "operation-1",
      revisionId: "revision-2",
      expectedSourceHash: "b".repeat(64),
      expectedRevisionVersion: 3,
      confirmFinalPublish: true,
    });
    expect(await screen.findByText("Final artifact")).toBeInTheDocument();
  });

  it("cancels the frozen finalize request before a refresh can load a drifted revision", async () => {
    const first = modelForOperation("operation-1", "revision-before-refresh", 3);
    const drifted = modelForOperation("operation-1", "revision-after-refresh", 4);
    const api: ReportingViewApi = {
      getReadModel: vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(drifted),
      buildReadModel: vi.fn().mockResolvedValue(drifted),
      finalizeRevision: vi.fn(),
    };

    render(<ReportReadModelView operationId="operation-1" api={api} />);
    await screen.findByText("Acme");
    fireEvent.click(screen.getByRole("button", { name: "Finalize report" }));
    expect(screen.getByRole("status")).toHaveTextContent("revision-before-refresh");

    fireEvent.click(screen.getByRole("button", { name: "Refresh cited report" }));

    expect(screen.queryByRole("button", { name: "Confirm final publish" })).not.toBeInTheDocument();
    await waitFor(() => expect(api.getReadModel).toHaveBeenCalledTimes(2));
    expect(api.finalizeRevision).not.toHaveBeenCalled();
  });

  it("cancels the frozen finalize request when an external refresh hint advances", async () => {
    const first = modelForOperation("operation-1", "revision-before-hint", 3);
    const refreshed = modelForOperation("operation-1", "revision-after-hint", 4);
    const api: ReportingViewApi = {
      getReadModel: vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(refreshed),
      buildReadModel: vi.fn(),
      finalizeRevision: vi.fn(),
    };

    const view = render(
      <ReportReadModelView operationId="operation-1" api={api} refreshVersion={1} />
    );
    await screen.findByText("Acme");
    fireEvent.click(screen.getByRole("button", { name: "Finalize report" }));
    expect(screen.getByRole("status")).toHaveTextContent("revision-before-hint");

    view.rerender(<ReportReadModelView operationId="operation-1" api={api} refreshVersion={2} />);

    expect(screen.queryByRole("button", { name: "Confirm final publish" })).not.toBeInTheDocument();
    await waitFor(() => expect(api.getReadModel).toHaveBeenCalledTimes(2));
    expect(api.finalizeRevision).not.toHaveBeenCalled();
  });

  it("cancels the frozen finalize request immediately when a rebuild starts", async () => {
    const first = modelForOperation("operation-1", "revision-before-rebuild", 3);
    const rebuilt = modelForOperation("operation-1", "revision-after-rebuild", 4);
    const api: ReportingViewApi = {
      getReadModel: vi.fn().mockResolvedValue(first),
      buildReadModel: vi.fn().mockResolvedValue(rebuilt),
      finalizeRevision: vi.fn(),
    };

    render(<ReportReadModelView operationId="operation-1" api={api} />);
    await screen.findByText("Acme");
    fireEvent.click(screen.getByRole("button", { name: "Finalize report" }));
    expect(screen.getByRole("status")).toHaveTextContent("revision-before-rebuild");

    fireEvent.click(screen.getByRole("button", { name: "Rebuild from DB truth" }));

    expect(screen.queryByRole("button", { name: "Confirm final publish" })).not.toBeInTheDocument();
    await waitFor(() => expect(api.buildReadModel).toHaveBeenCalledTimes(1));
    expect(api.finalizeRevision).not.toHaveBeenCalled();
  });

  it("cancels the frozen finalize request when the operation identity changes", async () => {
    const operationOne = modelForOperation("operation-1", "revision-operation-1", 3);
    const operationTwo = modelForOperation("operation-2", "revision-operation-2", 1);
    const api: ReportingViewApi = {
      getReadModel: vi
        .fn()
        .mockResolvedValueOnce(operationOne)
        .mockResolvedValueOnce(operationTwo),
      buildReadModel: vi.fn(),
      finalizeRevision: vi.fn(),
    };

    const view = render(<ReportReadModelView operationId="operation-1" api={api} />);
    await screen.findByText("Acme");
    fireEvent.click(screen.getByRole("button", { name: "Finalize report" }));
    expect(screen.getByRole("status")).toHaveTextContent("revision-operation-1");

    view.rerender(<ReportReadModelView operationId="operation-2" api={api} />);

    expect(screen.queryByRole("button", { name: "Confirm final publish" })).not.toBeInTheDocument();
    await waitFor(() =>
      expect(api.getReadModel).toHaveBeenLastCalledWith({ operationId: "operation-2" })
    );
    expect(api.finalizeRevision).not.toHaveBeenCalled();
  });

  it("refuses the second confirmation when the displayed revision drifts from the frozen CAS", async () => {
    const model = modelForOperation("operation-1", "revision-frozen", 3);
    const api: ReportingViewApi = {
      getReadModel: vi.fn().mockResolvedValue(model),
      buildReadModel: vi.fn(),
      finalizeRevision: vi.fn(),
    };

    render(<ReportReadModelView operationId="operation-1" api={api} />);
    await screen.findByText("Acme");
    fireEvent.click(screen.getByRole("button", { name: "Finalize report" }));
    expect(screen.getByRole("status")).toHaveTextContent("revision-frozen");

    if (model.current) {
      model.current.revisionId = "revision-drifted";
      model.current.rowVersion = 4;
    }
    fireEvent.click(screen.getByRole("button", { name: "Confirm final publish" }));

    expect(api.finalizeRevision).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("changed before confirmation");
    expect(screen.queryByRole("button", { name: "Confirm final publish" })).not.toBeInTheDocument();
  });

  it("renders explicit loading, error and empty states", async () => {
    const deferred = new Promise<null>(() => undefined);
    const loadingApi: ReportingViewApi = {
      getReadModel: vi.fn().mockReturnValue(deferred),
      buildReadModel: vi.fn(),
      finalizeRevision: vi.fn(),
    };
    const loading = render(<ReportReadModelView operationId="operation-1" api={loadingApi} />);
    expect(screen.getByText("Loading cited report…")).toBeInTheDocument();
    loading.unmount();

    const errorApi: ReportingViewApi = {
      getReadModel: vi.fn().mockRejectedValue(new Error("DB unavailable")),
      buildReadModel: vi.fn(),
      finalizeRevision: vi.fn(),
    };
    const failed = render(<ReportReadModelView operationId="operation-1" api={errorApi} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("DB unavailable");
    failed.unmount();

    const emptyApi: ReportingViewApi = {
      getReadModel: vi.fn().mockResolvedValue(null),
      buildReadModel: vi.fn().mockResolvedValue(validatedModel()),
      finalizeRevision: vi.fn(),
    };
    render(<ReportReadModelView operationId="operation-1" api={emptyApi} />);
    expect(await screen.findByText(/No report revision has been built/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Build cited report" }));
    await waitFor(() => expect(emptyApi.buildReadModel).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Acme")).toBeInTheDocument();
  });
});
