import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  getCleanupCloseoutGate,
  listCleanupObligations,
  waiveCleanupObligation,
} from "@/lib/api/cleanup";
import { CleanupObligationList } from "./CleanupObligationList";

vi.mock("@/lib/api/cleanup", () => ({
  getCleanupCloseoutGate: vi.fn(),
  listCleanupObligations: vi.fn(),
  waiveCleanupObligation: vi.fn(),
}));

const list = vi.mocked(listCleanupObligations);
const gate = vi.mocked(getCleanupCloseoutGate);
const waive = vi.mocked(waiveCleanupObligation);

const row = {
  obligationId: "obligation-1",
  operationId: "operation-1",
  projectScopeId: "project-1",
  scopeSnapshotId: "snapshot-1",
  organizationIdAtTime: "organization-1",
  sourceActionId: "action-1",
  status: "open",
  deadline: "2026-07-14T00:00:00Z",
  affectedResourceSnapshot: { kind: "account" },
  cleanupStrategy: { kind: "delete_created_resource" },
  residualRisk: null,
  rowVersion: 3,
};

const secondRow = {
  ...row,
  obligationId: "obligation-2",
  projectScopeId: "project-2",
  scopeSnapshotId: "snapshot-2",
  sourceActionId: "action-2",
  rowVersion: 7,
};

describe("CleanupObligationList", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    list.mockResolvedValue([row] as never);
    gate.mockResolvedValue({
      operationId: "operation-1",
      organizationIdAtTime: "organization-1",
      missingObligationCount: 0,
      nonterminalObligationCount: 1,
      undisclosedResidualCount: 0,
      invalidTerminalTruthCount: 0,
      allowsCloseout: false,
    } as never);
  });

  it("requires a second confirmation and freezes exact scope/CAS per obligation", async () => {
    list.mockResolvedValue([row, secondRow] as never);
    waive.mockResolvedValue({ ...row, status: "waived_by_user", rowVersion: 4 } as never);
    vi.spyOn(globalThis.crypto, "randomUUID").mockReturnValue(
      "00000000-0000-4000-8000-000000000001"
    );
    render(<CleanupObligationList operationId="operation-1" organizationIdAtTime="organization-1" />);
    expect(await screen.findByText("obligation-1")).toBeInTheDocument();
    expect(screen.getByText(/missing 0 · open 1 · residual 0 · invalid 0/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Waiver reason obligation-1"), {
      target: { value: "Owner retains resource" },
    });
    fireEvent.change(screen.getByLabelText("Residual summary obligation-1"), {
      target: { value: "Documented medium residual" },
    });
    fireEvent.change(screen.getByLabelText("Waiver evidence obligation-1"), {
      target: { value: "41" },
    });
    fireEvent.change(screen.getByLabelText("Waiver reason obligation-2"), {
      target: { value: "Independent second draft" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Review waiver obligation-1" }));
    expect(waive).not.toHaveBeenCalled();
    expect(screen.getByText(/project-1.*snapshot-1.*organization-1.*row 3/)).toBeInTheDocument();

    // Editing after review cannot alter the frozen confirmation payload, and
    // another obligation's draft remains independent.
    fireEvent.change(screen.getByLabelText("Waiver reason obligation-1"), {
      target: { value: "Drifted after review" },
    });
    expect(screen.getByLabelText("Waiver reason obligation-2")).toHaveValue(
      "Independent second draft"
    );
    fireEvent.click(screen.getByRole("button", { name: "Confirm waiver obligation-1" }));
    await waitFor(() => expect(waive).toHaveBeenCalledTimes(1));
    expect(waive.mock.calls[0][0]).toEqual({
      waiverId: "00000000-0000-4000-8000-000000000001",
      obligationId: "obligation-1",
      operationId: "operation-1",
      projectScopeId: "project-1",
      scopeSnapshotId: "snapshot-1",
      organizationIdAtTime: "organization-1",
      expectedRowVersion: 3,
      reason: "Owner retains resource",
      residualSummary: "Documented medium residual",
      residualSeverity: "medium",
      evidenceIds: [41],
    });
    expect(waive.mock.calls[0][0]).not.toHaveProperty("actorId");
  });

  it("renders explicit loading, error and empty states", async () => {
    list.mockRejectedValueOnce(new Error("DB unavailable"));
    const first = render(<CleanupObligationList operationId="operation-1" organizationIdAtTime="organization-1" />);
    expect(screen.getByText("Loading cleanup obligations…")).toBeInTheDocument();
    expect(await screen.findByRole("alert")).toHaveTextContent("DB unavailable");
    first.unmount();
    list.mockResolvedValueOnce([] as never);
    render(<CleanupObligationList operationId="operation-1" organizationIdAtTime="organization-1" />);
    expect(await screen.findByText(/No cleanup obligations were created/)).toBeInTheDocument();
  });
});
