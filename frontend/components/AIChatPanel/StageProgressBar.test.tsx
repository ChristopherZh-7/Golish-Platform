import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { StageProgressBar } from "./StageProgressBar";
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

const stagePlans: StagePlansViewModel = {
  stageOrder: ["scoping", "external_attack_surface", "enumeration"],
  plansByStage: {
    scoping: makePlan([{ step: "Confirm scope", status: "completed" }]),
    external_attack_surface: makePlan([
      { step: "Deploy per-organization scanner", status: "in_progress" },
      { step: "Verify coverage matrix", status: "pending" },
      { step: "Submit stage deliverable", status: "pending" },
    ]),
    enumeration: makePlan([{ step: "Synthetic seed", status: "pending" }], 0),
  },
  passedStages: ["scoping"],
};

describe("StageProgressBar", () => {
  it("keeps the top status to one compact line without duplicating the stage plan", () => {
    render(<StageProgressBar stagePlans={stagePlans} isRunning={false} />);

    expect(screen.getByText("External Attack Surface")).toBeInTheDocument();
    expect(screen.getByText("Stage 2/3")).toBeInTheDocument();
    expect(screen.getByText("· Step 0/3")).toBeInTheDocument();
    expect(screen.getByText(/Deploy per-organization scanner/)).toBeInTheDocument();
    expect(screen.queryByText("Verify coverage matrix")).toBeNull();
    expect(screen.queryByRole("button", { name: /plan/i })).toBeNull();
  });

  it("opens the complete workflow from its separate control", async () => {
    const user = userEvent.setup();
    render(<StageProgressBar stagePlans={stagePlans} isRunning={false} />);

    await user.click(screen.getByRole("button", { name: "Show workflow" }));
    const workflow = screen.getByRole("region", { name: "Stage workflow" });

    expect(within(workflow).getByText("Scoping")).toBeInTheDocument();
    expect(within(workflow).getByText("External Attack Surface")).toBeInTheDocument();
    expect(within(workflow).getByText("Enumeration")).toBeInTheDocument();
  });
});
