import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  type InvestigationWorkspaceModel,
  InvestigationWorkspaceView,
} from "./InvestigationWorkspaceView";

const identity = {
  operationId: "operation-1",
  stageExecutionId: "execution-1",
  stageRunRequestId: "stage-run-1",
};

function actor(
  transcriptRequestId: string | null,
  label: string,
  overrides: Record<string, unknown> = {}
) {
  return {
    actorId: `actor:${label}`,
    actorKind: "worker" as const,
    label,
    organizationId: "org-1",
    hypothesisRevisionId: null,
    taskId: "task-1",
    subtaskId: null,
    workerRunId: `worker:${label}`,
    owningStageRunRequestId: identity.stageRunRequestId,
    transcriptRequestId,
    parentActorTranscriptRequestId: null,
    parentDispatchToolRequestId: null,
    status: "running",
    children: [],
    ...overrides,
  };
}

function model(): InvestigationWorkspaceModel {
  const nested = actor("nested-transcript", "Nested Researcher", {
    actorId: "nested",
    actorKind: "nested_worker",
    parentActorTranscriptRequestId: "analysis-worker-transcript",
    parentDispatchToolRequestId: "dispatch-nested",
  });
  const analysisWorker = actor("analysis-worker-transcript", "Dynamic Browser", {
    actorId: "analysis-worker",
    children: [nested],
  });
  const verificationWorker = actor("verification-worker-transcript", "Pentester", {
    actorId: "verification-worker",
    hypothesisRevisionId: "revision-1",
    taskId: "verification-task-1",
  });

  return {
    identity,
    projectionSchemaVersion: 2,
    changeSeq: 7,
    stale: false,
    stageStatus: "verifying",
    allowPreparedActionJit: true,
    main: actor("main-transcript", "Main", {
      actorId: "main",
      actorKind: "main",
      taskId: null,
    }),
    organizations: [
      {
        organizationId: "org-1",
        label: "Acme",
        readSession: actor("read-session-transcript", "Bounded read session", {
          actorId: "read-session",
          actorKind: "read_session",
          taskId: null,
        }),
        analysisTasks: [
          {
            taskId: "analysis-task-1",
            label: "Analysis Task",
            status: "running",
            primary: actor("analysis-primary-transcript", "Analysis Primary", {
              actorId: "analysis-primary",
              actorKind: "primary",
            }),
            subtasks: [
              {
                subtaskId: "analysis-subtask-1",
                ordinal: 1,
                label: "Inspect trust boundary",
                status: "running",
                workers: [analysisWorker],
              },
            ],
          },
        ],
      },
    ],
    hypotheses: [
      {
        revisionId: "revision-1",
        organizationId: "org-1",
        claim: "An exact, independently falsifiable claim",
        epistemicState: "supported",
        admissionDisposition: "scheduled",
        task: {
          taskId: "verification-task-1",
          label: "Verification Task",
          status: "running",
          primary: actor("verification-primary-transcript", "Verification Primary", {
            actorId: "verification-primary",
            actorKind: "primary",
            hypothesisRevisionId: "revision-1",
            taskId: "verification-task-1",
          }),
          subtasks: [
            {
              subtaskId: "verification-subtask-1",
              ordinal: 1,
              label: "Run designated falsifier",
              status: "running",
              workers: [verificationWorker],
            },
          ],
          operator: actor(null, "Typed Operator", {
            actorId: "operator",
            actorKind: "operator",
            hypothesisRevisionId: "revision-1",
            taskId: "verification-task-1",
            status: "authorization_required",
          }),
        },
        campaigns: [
          { campaignId: "campaign-1", label: "Campaign 1", status: "awaiting_authorization" },
        ],
        evidenceRefs: ["evidence-1"],
        methodologyCitations: ["methodology://signal-1"],
      },
    ],
    control: {
      stageTopologyContract: "unified_investigation_v1",
      investigationRunState: "verifying",
      investigationRunStateHead: "run-head-7",
      stopEpoch: 0,
      stopAllowed: true,
      stopReason: null,
      resetAllowed: false,
      resetReason: "active unified run",
      forkAllowed: false,
      forkReason: "active unified run",
      adoptionContractVersion: 1,
      controlPolicyVersion: 1,
    },
  };
}

describe("InvestigationWorkspaceView", () => {
  it("renders the real Main, per-org read session and single-Primary ordered task tree", () => {
    render(<InvestigationWorkspaceView identity={identity} state={{ status: "ready", model: model() }} />);

    expect(screen.getByRole("navigation", { name: "Investigation agents and hypotheses" })).toHaveTextContent(
      "Main"
    );
    expect(screen.getByText("Bounded read session")).toBeInTheDocument();
    expect(screen.getByText("Analysis Primary")).toBeInTheDocument();
    expect(screen.getByText(/Inspect trust boundary/)).toBeInTheDocument();
    expect(screen.getByText("Dynamic Browser")).toBeInTheDocument();
    expect(screen.getByText("Nested Researcher")).toBeInTheDocument();
    expect(screen.getByText("Verification Primary")).toBeInTheDocument();
    expect(screen.getByText("Typed Operator")).toBeInTheDocument();
  });

  it("clicking a hypothesis is view-only and keeps JIT and stop as separate controls", () => {
    const onRequestStop = vi.fn();
    const onPreparedAction = vi.fn();
    render(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "ready", model: model() }}
        onRequestStop={onRequestStop}
        renderPreparedActions={({ operationId, campaignId }) => {
          onPreparedAction(operationId, campaignId);
          return <div>Prepared action control</div>;
        }}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /An exact, independently falsifiable claim/ }));
    expect(onRequestStop).not.toHaveBeenCalled();
    expect(onPreparedAction).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Stop Investigation" })).not.toBe(
      screen.getByRole("button", { name: /An exact, independently falsifiable claim/ })
    );

    fireEvent.click(screen.getByRole("button", { name: "Campaign 1" }));
    expect(onPreparedAction).toHaveBeenCalledWith("operation-1", "campaign-1");
  });

  it("selects an exact nested transcript and reports missing identity as unavailable", () => {
    const { rerender } = render(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "ready", model: model() }}
        renderTranscript={({ transcriptRequestId, owningStageRunRequestId }) => (
          <div>{`${owningStageRunRequestId}::${transcriptRequestId}`}</div>
        )}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Nested Researcher/ }));
    expect(screen.getByText("stage-run-1::nested-transcript")).toBeInTheDocument();

    const missingReadIdentity = model();
    missingReadIdentity.organizations[0].readSession!.transcriptRequestId = null;
    rerender(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "ready", model: missingReadIdentity }}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /Bounded read session/ }));
    expect(screen.getByRole("alert")).toHaveTextContent("Transcript unavailable");
  });

  it("fails closed on a foreign topology or incomplete nested parent ownership", () => {
    const foreignTopology = model();
    foreignTopology.control.stageTopologyContract = "legacy_candidate_verification_v1";
    const { rerender } = render(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "ready", model: foreignTopology }}
      />
    );
    expect(screen.getByRole("alert")).toHaveTextContent("identity or topology conflict");

    const incompleteParent = model();
    incompleteParent.organizations[0].analysisTasks[0].subtasks[0].workers[0].children[0].parentDispatchToolRequestId =
      null;
    rerender(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "ready", model: incompleteParent }}
        renderTranscript={({ transcriptRequestId }) => <div>{transcriptRequestId}</div>}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /Nested Researcher/ }));
    expect(screen.getByRole("alert")).toHaveTextContent("parent ownership is incomplete");
  });

  it("never synthesizes Main and distinguishes loading error empty stale and identity conflict", () => {
    const withoutMain = model();
    withoutMain.main = null;
    const { rerender } = render(
      <InvestigationWorkspaceView identity={identity} state={{ status: "loading" }} />
    );
    expect(screen.getByRole("status")).toHaveTextContent("Loading exact Investigation projection");

    rerender(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "error", message: "DB read failed" }}
      />
    );
    expect(screen.getByRole("alert")).toHaveTextContent("DB read failed");

    rerender(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "ready", model: { ...withoutMain, hypotheses: [] } }}
      />
    );
    expect(screen.getByText(/No hypotheses were generated/)).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Main identity unavailable");
    expect(screen.queryByText("__main__")).not.toBeInTheDocument();

    rerender(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "stale", model: withoutMain, message: "projection head changed" }}
      />
    );
    expect(screen.getByText("projection head changed")).toBeInTheDocument();

    rerender(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "unavailable", message: "conflicting transcript identity" }}
      />
    );
    expect(screen.getByRole("alert")).toHaveTextContent("conflicting transcript identity");
  });

  it("submits only the exact server-projected run head to the explicit stop handler", () => {
    const onRequestStop = vi.fn();
    render(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "ready", model: model() }}
        onRequestStop={onRequestStop}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Stop Investigation" }));
    expect(onRequestStop).toHaveBeenCalledWith({
      identity,
      expectedChangeSeq: 7,
      expectedInvestigationRunStateHead: "run-head-7",
    });
    expect(screen.getByRole("button", { name: "Reset Investigation" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Fork Investigation successor" })).toBeDisabled();
  });

  it("keeps local focus stable across monotonic refresh and applies a deep-link only once", async () => {
    const first = model();
    const { rerender } = render(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "ready", model: first }}
        deepLinkTranscriptRequestId="nested-transcript"
      />
    );

    const nested = await screen.findByRole("button", { name: /Nested Researcher/ });
    await waitFor(() => expect(nested).toHaveAttribute("aria-current", "true"));
    nested.focus();

    const refreshed = model();
    refreshed.changeSeq = 8;
    refreshed.main!.transcriptRequestId = "new-main-transcript";
    rerender(
      <InvestigationWorkspaceView
        identity={identity}
        state={{ status: "ready", model: refreshed }}
        deepLinkTranscriptRequestId="new-main-transcript"
      />
    );

    expect(screen.getByRole("button", { name: /Nested Researcher/ })).toHaveAttribute(
      "aria-current",
      "true"
    );
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: /Nested Researcher/ })
    );
  });
});
