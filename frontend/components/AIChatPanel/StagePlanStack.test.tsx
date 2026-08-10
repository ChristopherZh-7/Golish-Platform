import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "@/store";
import { StagePlanStack } from "./StagePlanStack";
import type { StagePlansViewModel, TaskPlanViewModel } from "./TaskPlan";

function makePlan(steps: TaskPlanViewModel["steps"], version = 1): TaskPlanViewModel {
  return {
    version,
    steps,
    summary: {
      total: steps.length,
      completed: steps.filter((step) => step.status === "completed").length,
      in_progress: steps.filter((step) => step.status === "in_progress").length,
      pending: steps.filter((step) => step.status === "pending").length,
    },
  };
}

function makeStagePlans(passedStages = ["scoping"]): StagePlansViewModel {
  return {
    stageOrder: ["scoping", "external_attack_surface", "enumeration"],
    plansByStage: {
      scoping: makePlan([{ step: "Confirm scope", status: "completed" }]),
      external_attack_surface: makePlan([
        { step: "Deploy per-organization scanner", status: "in_progress" },
        { step: "Verify coverage matrix", status: "pending" },
        { step: "Submit stage deliverable", status: "pending" },
      ]),
      enumeration: makePlan([{ step: "Synthetic future seed", status: "pending" }], 0),
    },
    passedStages,
  };
}

describe("StagePlanStack", () => {
  beforeEach(() => {
    useStore.setState({
      activeConversationId: "conv-stage-card",
      conversations: {
        "conv-stage-card": { id: "conv-stage-card", isStreaming: true } as any,
      },
    });
  });

  it("renders only the stages anchored to this message and opens the active plan", async () => {
    const user = userEvent.setup();
    render(
      <StagePlanStack
        stagePlans={makeStagePlans()}
        stageIds={["external_attack_surface"]}
      />
    );

    expect(screen.getByText("External Attack Surface")).toBeInTheDocument();
    expect(screen.getByText("Stage 2/3")).toBeInTheDocument();
    expect(screen.getByText("Verify coverage matrix")).toBeInTheDocument();
    expect(screen.queryByText("Scoping")).toBeNull();
    expect(screen.queryByText("Enumeration")).toBeNull();

    const card = screen.getByRole("button", { name: /External Attack Surface/ });
    expect(card).toHaveAttribute("aria-expanded", "true");
    await user.click(card);
    expect(card).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Verify coverage matrix")).toBeNull();
  });

  it("auto-collapses a completed stage in place and still allows review", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <StagePlanStack
        stagePlans={makeStagePlans()}
        stageIds={["external_attack_surface"]}
      />
    );
    expect(screen.getByText("Verify coverage matrix")).toBeInTheDocument();

    rerender(
      <StagePlanStack
        stagePlans={makeStagePlans(["scoping", "external_attack_surface"])}
        stageIds={["external_attack_surface"]}
      />
    );
    const card = screen.getByRole("button", { name: /External Attack Surface/ });
    expect(card).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Verify coverage matrix")).toBeNull();

    await user.click(card);
    expect(card).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Verify coverage matrix")).toBeInTheDocument();
  });

  it("shows an entered v0 stage as preparation without exposing its synthetic todo", () => {
    render(
      <StagePlanStack
        stagePlans={{
          stageOrder: ["target_intel"],
          plansByStage: {
            target_intel: makePlan([{ step: "Synthetic entry seed", status: "in_progress" }], 0),
          },
          passedStages: [],
        }}
        stageIds={["target_intel"]}
      />
    );

    expect(screen.getByText("Target Intel")).toBeInTheDocument();
    expect(screen.getByText(/Preparing stage plan/)).toBeInTheDocument();
    expect(screen.queryByText("Synthetic entry seed")).toBeNull();
    expect(screen.getByRole("button", { name: /Target Intel/ })).toBeDisabled();
  });
});
