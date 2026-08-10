import { render, screen } from "@testing-library/react";
import { createElement } from "react";
import { describe, expect, it } from "vitest";
import {
  CollapsibleToolCall,
  stageRunBlockedCopy,
  stageRunResultPassed,
  ToolCallSummary,
  toolResultIsBackgrounded,
  toolResultIsFailure,
} from "./ToolCallSummary";

describe("CollapsibleToolCall Gate truth", () => {
  it("renders an accepted closeout as a completed submission without claiming Gate completion", () => {
    render(
      createElement(CollapsibleToolCall, {
        tc: {
          name: "submit_stage_deliverable",
          success: true,
          result: '{"status":"accepted"}',
        },
      })
    );

    expect(screen.getByText("Submitted")).toBeInTheDocument();
    expect(screen.getByLabelText("Submission completed")).toBeInTheDocument();
    expect(screen.queryByText(/Awaiting Gate/)).not.toBeInTheDocument();
  });

  it("uses a green terminal card instead of an amber waiting clock after submit", () => {
    const { container } = render(
      createElement(ToolCallSummary, {
        toolCalls: [
          {
            name: "submit_stage_deliverable",
            success: true,
            result: '{"status":"accepted"}',
          },
        ],
      })
    );

    const card = container.querySelector('[data-terminal-state="submitted"]');
    expect(card).toHaveClass("border-[var(--ansi-green)]/30");
    expect(screen.getByLabelText("Submission completed")).toBeInTheDocument();
    expect(card?.querySelector(".lucide-clock")).toBeNull();
  });

  it("renders a blocked stage_run as blocked rather than successful", () => {
    render(
      createElement(CollapsibleToolCall, {
        tc: { name: "stage_run", success: true, result: '{"passed":false,"gaps":[]}' },
      })
    );

    expect(screen.getByText("Blocked")).toBeInTheDocument();
    expect(screen.queryByText("✓")).not.toBeInTheDocument();
  });
});

describe("stageRunBlockedCopy", () => {
  it("directs a blocked continue card into its recovery details", () => {
    expect(stageRunBlockedCopy(1)).toEqual({
      detailLabel: "处理阻塞 →",
      summaryLabel: "1 阻塞 · 点此卡处理",
    });
    expect(stageRunBlockedCopy(0)).toEqual({
      detailLabel: "Details →",
      summaryLabel: null,
    });
  });
});

describe("toolResultIsFailure", () => {
  it("flags a rejected status body as a failure (shows ❌ not ✅)", () => {
    expect(toolResultIsFailure('{"status":"rejected","reason":"stage_id mismatch"}')).toBe(true);
  });

  it("flags needs_fix / error / failed statuses too", () => {
    expect(toolResultIsFailure('{"status": "needs_fix"}')).toBe(true);
    expect(toolResultIsFailure('{ "status" : "error" }')).toBe(true);
    expect(toolResultIsFailure('{"status":"failed"}')).toBe(true);
    expect(toolResultIsFailure('{"status":"killed"}')).toBe(true);
  });

  it("does not treat a partial provider survey as failed because one nested provider failed", () => {
    expect(
      toolResultIsFailure(
        JSON.stringify({
          status: "partial",
          action: "map_assets",
          providerStatus: [
            { providerId: "0.zone", status: "failed", message: "HTTP 502" },
            { providerId: "quake", status: "completed", message: "normalized 419 record(s)" },
          ],
        })
      )
    ).toBe(false);
  });

  it("flags stderr ERROR output even when the process exit code is zero", () => {
    expect(
      toolResultIsFailure(
        '{"stdout":"","stderr":"\\u001b[1m\\u001b[31mERROR Opening: https://example.test - can\'t modify frozen Hash\\u001b[0m","exit_code":0}'
      )
    ).toBe(true);
  });

  it("flags missing tool dependencies even when the wrapper exits zero", () => {
    expect(
      toolResultIsFailure(
        JSON.stringify({
          stdout:
            "WhatWeb is not installed and is missing dependencies.\nThe following gems are missing:\n - addressable",
          stderr: "",
          exit_code: 0,
        })
      )
    ).toBe(true);
  });

  it("does NOT flag an accepted deliverable", () => {
    expect(
      toolResultIsFailure('{"status":"accepted","note":"structure OK; final gate at stage close."}')
    ).toBe(false);
  });

  it("ignores unrelated / non-status results and empty input", () => {
    expect(toolResultIsFailure('{"result":"done"}')).toBe(false);
    expect(
      toolResultIsFailure('{"stdout":"usable output","stderr":"WARNING: noisy scanner banner"}')
    ).toBe(false);
    expect(toolResultIsFailure("plain text output")).toBe(false);
    expect(toolResultIsFailure(undefined)).toBe(false);
    expect(toolResultIsFailure("")).toBe(false);
  });

  it("detects a backgrounded result as a live non-terminal state", () => {
    const result = '{"status":"backgrounded","job_id":"job_42","partial_stdout":"scanning"}';
    expect(toolResultIsBackgrounded(result)).toBe(true);
    expect(toolResultIsFailure(result)).toBe(false);
  });
});

describe("stageRunResultPassed", () => {
  it("recognizes only an explicit terminal aggregate pass", () => {
    expect(stageRunResultPassed('{"passed":true,"team_units_passed":1}')).toBe(true);
    expect(stageRunResultPassed('{"passed":false,"team_units_passed":0}')).toBe(false);
    expect(stageRunResultPassed('{"status":"accepted"}')).toBe(false);
    expect(stageRunResultPassed("not json")).toBe(false);
    expect(stageRunResultPassed(undefined)).toBe(false);
  });
});
