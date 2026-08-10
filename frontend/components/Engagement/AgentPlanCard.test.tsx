import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  AgentPlanCard,
  parseAgentPlanArgs,
  projectAgentPlanForPassedStage,
  resolveLatestVisibleAgentPlanRequest,
  type StageToolRequest,
} from "./AgentPlanCard";

const planArgs = {
  explanation: "先建立接口发现计划，再逐步确认参数。",
  plan: [
    { step: "收集并分类 JS", status: "completed" },
    { step: "定位 API 与主路径", status: "in_progress" },
    { step: "确认请求参数", status: "pending" },
  ],
};

function planRequest(
  id: string,
  status: StageToolRequest["status"] = "completed",
  args: unknown = planArgs
): StageToolRequest {
  return { id, name: "update_plan", args, status, result: { updated: true } };
}

describe("AgentPlanCard", () => {
  it("parses only bounded plans with at most one active step", () => {
    expect(parseAgentPlanArgs(planArgs)).toMatchObject({
      completedCount: 1,
      inProgressCount: 1,
      totalCount: 3,
    });
    expect(
      parseAgentPlanArgs({
        plan: [
          { step: "first", status: "in_progress" },
          { step: "second", status: "in_progress" },
        ],
      })
    ).toBeNull();
    expect(
      parseAgentPlanArgs({
        plan: Array.from({ length: 13 }, (_, index) => ({
          step: `step ${index + 1}`,
          status: "pending",
        })),
      })
    ).toBeNull();
  });

  it("resolves the latest valid visible update_plan in timeline order", () => {
    const first = planRequest("plan-first");
    const invalid = planRequest("plan-invalid", "completed", {
      plan: [{ step: "invalid", status: "unknown" }],
    });
    const rejected = planRequest("plan-rejected", "error");
    const current = planRequest("plan-current", "running", {
      explanation: "当前版本",
      plan: [{ step: "确认参数", status: "in_progress" }],
    });

    expect(
      resolveLatestVisibleAgentPlanRequest([current, rejected, first, invalid], {
        entries: [
          { kind: "tool_call", toolCallId: "plan-first" },
          { kind: "tool_call", toolCallId: "plan-invalid" },
          { kind: "tool_call", toolCallId: "plan-rejected" },
          { kind: "tool_call", toolCallId: "plan-current" },
        ],
      })?.id
    ).toBe("plan-current");
    expect(
      resolveLatestVisibleAgentPlanRequest([first, current], { parentStageStopped: true })?.id
    ).toBe("plan-first");
  });

  it("renders exact plan text and projects every step to completed only after stage pass", () => {
    const parsed = parseAgentPlanArgs(planArgs);
    expect(parsed).not.toBeNull();
    expect(projectAgentPlanForPassedStage(parsed!, false)).toBe(parsed);

    render(<AgentPlanCard tool={planRequest("plan-pass")} parentStagePassed />);

    expect(screen.getByRole("region", { name: "Controller plan" })).toBeInTheDocument();
    expect(screen.getByText("计划已完成")).toBeInTheDocument();
    expect(screen.getByText("3/3 已完成")).toBeInTheDocument();
    expect(screen.getByText("先建立接口发现计划，再逐步确认参数。")).toBeInTheDocument();
    expect(screen.getByText("定位 API 与主路径")).toBeInTheDocument();
    expect(screen.getAllByLabelText("步骤状态：completed")).toHaveLength(3);
    expect(screen.queryByLabelText("步骤状态：in_progress")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("步骤状态：pending")).not.toBeInTheDocument();
  });
});
