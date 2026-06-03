import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { InlinePlanCard } from "./InlinePlanCard";
import type { TaskPlanViewModel } from "./TaskPlan";

function makePlan(
  steps: TaskPlanViewModel["steps"],
  summary?: Partial<TaskPlanViewModel["summary"]>
): TaskPlanViewModel {
  const completed = steps.filter((s) => s.status === "completed").length;
  const in_progress = steps.filter((s) => s.status === "in_progress").length;
  const pending = steps.filter((s) => s.status === "pending").length;
  return {
    version: 1,
    steps,
    summary: { total: steps.length, completed, in_progress, pending, ...summary },
  };
}

describe("InlinePlanCard", () => {
  it("never renders 'Infinity more' for an empty (lazy) plan", () => {
    // Regression: spreading an empty `visibleIndices` into Math.min/Math.max
    // yields Infinity/-Infinity, which rendered as two "Infinity more" rows.
    render(<InlinePlanCard plan={makePlan([])} />);
    expect(screen.queryByText(/Infinity/)).toBeNull();
    expect(screen.queryByText(/NaN/)).toBeNull();
    expect(screen.queryByText(/-?Infinity more/)).toBeNull();
  });

  it("shows finite collapsed 'N more' counts for a real plan", () => {
    const plan = makePlan([
      { step: "Confirm scope", status: "completed" },
      { step: "Resolve DNS", status: "in_progress" },
      { step: "Enumerate subdomains", status: "pending" },
      { step: "Probe HTTP", status: "pending" },
    ]);
    render(<InlinePlanCard plan={plan} />);
    expect(screen.getByText(/1 \/ 4 tasks done/)).toBeInTheDocument();
    // last-completed=0, current=1 → after = 4-1-1 = 2 hidden steps, before = 0.
    expect(screen.getByText(/^2 more$/)).toBeInTheDocument();
    expect(screen.queryByText(/Infinity/)).toBeNull();
  });
});
