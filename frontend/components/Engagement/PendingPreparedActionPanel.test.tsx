import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { decidePreparedAction, listPendingPreparedActions } from "@/lib/api/attack";
import { PendingPreparedActionPanel } from "./PendingPreparedActionPanel";

vi.mock("@/lib/api/attack", () => ({
  listPendingPreparedActions: vi.fn(),
  decidePreparedAction: vi.fn(),
}));

const mockedList = vi.mocked(listPendingPreparedActions);
const mockedDecide = vi.mocked(decidePreparedAction);

function item(overrides: Record<string, unknown> = {}) {
  return {
    preparedActionId: "action-1",
    operationId: "operation-1",
    campaignId: "campaign-1",
    displayProjection: {
      actionKind: "directory_fingerprint",
      targetAtTime: "https://example.test",
      method: "GET",
      redactedSequence: ["candidate request", "soft-404 control"],
      expectedControl: "different content fingerprint",
      destinationScopeSummary: "exact origin",
      redirectPolicy: "same-origin only",
      maxRedirectHops: 0,
      networkPolicyHash: "sha256:network",
      plannedBudgetAxes: [{ axis: "requests", plannedLimit: 2, unit: "request" }],
      cleanupSummary: "release isolated cookie jar",
    },
    privateManifestHash: "sha256:manifest",
    displayProjectionHash: "sha256:display",
    rendererVersion: "renderer.v1",
    riskTier: "T2",
    reviewState: "pending",
    rowVersion: 7,
    expiresAt: "2026-08-02T12:00:00Z",
    authorization: null,
    ...overrides,
  };
}

describe("PendingPreparedActionPanel", () => {
  beforeEach(() => vi.resetAllMocks());

  it("bootstraps from DB and renders only the redacted display projection", async () => {
    mockedList.mockResolvedValue([item()] as never);
    render(<PendingPreparedActionPanel operationId="operation-1" />);

    expect(screen.getByText("Loading prepared actions from the database…")).toBeInTheDocument();
    expect(await screen.findByText("https://example.test")).toBeInTheDocument();
    expect(mockedList).toHaveBeenCalledWith({ operationId: "operation-1", campaignId: null });
    expect(screen.getByText("candidate request → soft-404 control")).toBeInTheDocument();
    expect(document.body).not.toHaveTextContent("Authorization:");
    expect(document.body).not.toHaveTextContent("secret-token");
    expect(document.body).not.toHaveTextContent("raw body");
  });

  it("renders explicit error and empty states", async () => {
    mockedList.mockRejectedValueOnce(new Error("DB unavailable"));
    const failed = render(<PendingPreparedActionPanel operationId="operation-1" />);
    expect(await screen.findByRole("alert")).toHaveTextContent("DB unavailable");
    failed.unmount();

    mockedList.mockResolvedValueOnce([]);
    render(<PendingPreparedActionPanel operationId="operation-1" />);
    expect(await screen.findByText("No prepared actions require review.")).toBeInTheDocument();
  });

  it("shows human controls only for T2/T3 and submits exact CAS material", async () => {
    mockedList.mockResolvedValueOnce([item()] as never).mockResolvedValueOnce([]);
    mockedDecide.mockResolvedValue({} as never);
    render(<PendingPreparedActionPanel operationId="operation-1" campaignId="campaign-1" />);

    await screen.findByText("https://example.test");
    fireEvent.click(screen.getByRole("button", { name: "Approve exact action" }));
    await waitFor(() => expect(mockedDecide).toHaveBeenCalledTimes(1));
    expect(mockedDecide.mock.calls[0][0]).toEqual({
      operationId: "operation-1",
      campaignId: "campaign-1",
      preparedActionId: "action-1",
      decision: "approve",
      privateManifestHash: "sha256:manifest",
      displayProjectionHash: "sha256:display",
      rendererVersion: "renderer.v1",
      expectedRowVersion: 7,
      stableRequestId: expect.any(String),
      requestedExpiry: null,
    });
    await waitFor(() => expect(mockedList).toHaveBeenCalledTimes(2));
  });

  it("never exposes approval buttons for T0/T1 policy decisions", async () => {
    mockedList.mockResolvedValue([item({ riskTier: "T1" })] as never);
    render(<PendingPreparedActionPanel operationId="operation-1" />);
    await screen.findByText("https://example.test");
    expect(screen.queryByRole("button", { name: "Approve exact action" })).not.toBeInTheDocument();
    expect(screen.getByText(/T0\/T1 policy decisions are server-owned/)).toBeInTheDocument();
  });

  it("reloads on refresh hints and after a drift failure", async () => {
    mockedList.mockResolvedValue([item()] as never);
    mockedDecide.mockRejectedValueOnce(new Error("renderer drift"));
    const view = render(<PendingPreparedActionPanel operationId="operation-1" refreshVersion={1} />);
    await screen.findByText("https://example.test");
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    await waitFor(() => expect(mockedList).toHaveBeenCalledTimes(2));

    view.rerender(<PendingPreparedActionPanel operationId="operation-1" refreshVersion={2} />);
    await waitFor(() => expect(mockedList).toHaveBeenCalledTimes(3));
  });
});
