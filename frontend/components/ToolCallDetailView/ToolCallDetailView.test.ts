import { describe, expect, it } from "vitest";
import {
  DETAIL_PENDING_OUTPUT_SPINNER_CLASS,
  DETAIL_RUNNING_SPINNER_CLASS,
  getLiveOutputForDetail,
  getReportingStageRunOperationId,
  getShellOutputForDetail,
  isAttackCandidateStageRun,
  isCleanupStageRun,
  isShellLikeToolForDetail,
  TOOL_DETAIL_STATUS_BADGE_STYLES,
} from "./ToolCallDetailView";

describe("getShellOutputForDetail", () => {
  it("shows an output panel placeholder while a shell-like tool is running", () => {
    expect(getShellOutputForDetail(undefined, undefined, "running")).toEqual({
      text: "Waiting for output...",
      pending: true,
    });
  });

  it("prefers streaming output as soon as chunks arrive", () => {
    expect(getShellOutputForDetail(undefined, "httpx line\n", "running")).toEqual({
      text: "httpx line",
      pending: false,
    });
  });

  it("uses final stdout after completion", () => {
    expect(
      getShellOutputForDetail({ stdout: "done\n", stderr: "", exit_code: 0 }, undefined, "completed")
    ).toEqual({
      text: "done",
      pending: false,
    });
  });

  it("keeps an output panel visible for completed shell-like tools with empty output", () => {
    expect(
      getShellOutputForDetail({ stdout: "", stderr: "", exit_code: 0 }, undefined, "completed")
    ).toEqual({
      text: "No output.",
      pending: false,
    });
  });

  it("shows partial output from a backgrounded shell-like tool", () => {
    expect(
      getShellOutputForDetail(
        { status: "backgrounded", partial_stdout: "scanning...\n", partial_stderr: "" },
        undefined,
        "backgrounded"
      )
    ).toEqual({
      text: "scanning...",
      pending: false,
    });
  });

  it("treats background tool-wrapper args as shell-like detail output", () => {
    expect(
      isShellLikeToolForDetail("whatweb", {
        tool_name: "whatweb",
        args: "-a 1 https://example.com",
        background: true,
        timeout_secs: 60,
      })
    ).toBe(true);
  });

  it("uses readable running spinners in detail surfaces", () => {
    expect(DETAIL_RUNNING_SPINNER_CLASS).toContain("h-4");
    expect(DETAIL_RUNNING_SPINNER_CLASS).toContain("w-4");
    expect(DETAIL_PENDING_OUTPUT_SPINNER_CLASS).toContain("h-4");
    expect(DETAIL_PENDING_OUTPUT_SPINNER_CLASS).toContain("w-4");
  });

  it("uses high-contrast colors for live detail status badges", () => {
    expect(TOOL_DETAIL_STATUS_BADGE_STYLES.running).toContain("text-[var(--ansi-blue)]");
    expect(TOOL_DETAIL_STATUS_BADGE_STYLES.running).toContain("border-[var(--ansi-blue)]/45");
    expect(TOOL_DETAIL_STATUS_BADGE_STYLES.backgrounded).toContain("text-amber-300");
  });

  it("mounts Candidate review only on attack_candidate stage_run detail", () => {
    expect(isAttackCandidateStageRun("stage_run", { stage: "attack_candidate" })).toBe(true);
    expect(isAttackCandidateStageRun("stage_run", '{"stage_id":"attack_candidate"}')).toBe(true);
    expect(isAttackCandidateStageRun("stage_run", { stage: "verification" })).toBe(false);
    expect(isAttackCandidateStageRun("submit_stage_deliverable", { stage: "attack_candidate" })).toBe(
      false
    );
  });

  it("mounts Cleanup closeout only on cleanup stage_run detail", () => {
    expect(isCleanupStageRun("stage_run", { stage: "cleanup" })).toBe(true);
    expect(isCleanupStageRun("stage_run", '{"stage_id":"cleanup"}')).toBe(true);
    expect(isCleanupStageRun("stage_run", { stage: "reporting" })).toBe(false);
    expect(isCleanupStageRun("cleanup_inspect_obligation", { stage: "cleanup" })).toBe(false);
  });

  it("derives a Reporting operation only from matching stage_run args or result identity", () => {
    expect(
      getReportingStageRunOperationId(
        "stage_run",
        { stage: "reporting", operation_id: "operation-from-args" },
        undefined
      )
    ).toBe("operation-from-args");
    expect(
      getReportingStageRunOperationId(
        "stage_run",
        { orgs: [] },
        { stage: "reporting", operationId: "operation-from-result" }
      )
    ).toBe("operation-from-result");
    expect(
      getReportingStageRunOperationId(
        "stage_run",
        '{"stage_id":"reporting","operation_id":"operation-json"}',
        undefined
      )
    ).toBe("operation-json");
    expect(
      getReportingStageRunOperationId(
        "stage_run",
        { stage: "reporting", operation_id: "operation-a" },
        { stage: "reporting", operation_id: "operation-b" }
      )
    ).toBeNull();
    expect(
      getReportingStageRunOperationId(
        "stage_run",
        { stage: "reporting", operation_id: "operation-a" },
        { stage: "cleanup", operation_id: "operation-a" }
      )
    ).toBeNull();
    expect(
      getReportingStageRunOperationId(
        "submit_stage_deliverable",
        { stage: "reporting", operation_id: "operation-a" },
        undefined
      )
    ).toBeNull();
  });
});

describe("getLiveOutputForDetail", () => {
  it("shows a placeholder for running non-shell tools before chunks arrive", () => {
    expect(getLiveOutputForDetail(undefined, "running")).toEqual({
      text: "Waiting for output...",
      pending: true,
    });
  });

  it("uses streamed chunks for running non-shell tools", () => {
    expect(getLiveOutputForDetail("scanned 2 JS files\nfound 3 endpoints\n", "running")).toEqual({
      text: "scanned 2 JS files\nfound 3 endpoints",
      pending: false,
    });
  });
});
