import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AttackCandidateStageRunRows } from "./AttackCandidateStageRunRows";
import type { StageRunRow } from "./StageRunOrgRows";

function blockedRow(): StageRunRow {
  return {
    id: "org-1",
    name: "Acme Root",
    ownershipPercent: null,
    status: "blocked",
    agentRequestId: "tool-1::org::org-1",
    activity:
      "ATTACK_OBSERVATION_SCHEMA_UNSUPPORTED: Candidate observation schema has no immutable verifier classifier",
    evidenceCount: 0,
    coverage: {},
    stage: "attack_candidate",
  };
}

describe("AttackCandidateStageRunRows", () => {
  it("keeps an exact backend blocker visible and links the Analyst run", () => {
    const onDrillIn = vi.fn();

    render(
      <AttackCandidateStageRunRows
        rows={[blockedRow()]}
        roleLabel="Attack Analyst"
        onDrillIn={onDrillIn}
      />
    );

    expect(screen.getByText("已阻塞")).toBeInTheDocument();
    expect(screen.getByText(/ATTACK_OBSERVATION_SCHEMA_UNSUPPORTED/)).toBeInTheDocument();
    screen
      .getByRole("button", { name: /查看 Acme Root 的 Attack Analyst Agent 运行流/ })
      .click();
    expect(onDrillIn).toHaveBeenCalledWith("tool-1::org::org-1");
  });

  it("does not render unrelated stage rows", () => {
    const { container } = render(
      <AttackCandidateStageRunRows rows={[{ ...blockedRow(), stage: "verification" }]} />
    );

    expect(container).toBeEmptyDOMElement();
  });
});
