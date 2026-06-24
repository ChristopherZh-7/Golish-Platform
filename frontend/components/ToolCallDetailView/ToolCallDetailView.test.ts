import { describe, expect, it } from "vitest";
import {
  DETAIL_PENDING_OUTPUT_SPINNER_CLASS,
  DETAIL_RUNNING_SPINNER_CLASS,
  getShellOutputForDetail,
  isShellLikeToolForDetail,
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
});
