import { describe, expect, it } from "vitest";
import {
  DETAIL_PENDING_OUTPUT_SPINNER_CLASS,
  DETAIL_RUNNING_SPINNER_CLASS,
  getLiveOutputForDetail,
  getShellOutputForDetail,
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
