import { describe, expect, it } from "vitest";
import type { AiToolExecution } from "@/store";
import { getToolExecutionDisplayStatus } from "./ToolExecutionCard";

function execution(result: unknown): Pick<AiToolExecution, "status" | "result"> {
  return { status: "completed", result };
}

describe("getToolExecutionDisplayStatus", () => {
  it("treats a completed shell-like result with stderr ERROR as display error", () => {
    expect(
      getToolExecutionDisplayStatus(
        execution({
          stdout: "",
          stderr: "ERROR Opening: https://113.204.125.13 - can't modify frozen Hash",
          exit_code: 0,
        })
      )
    ).toBe("error");
  });

  it("does not treat stderr warnings as display errors", () => {
    expect(
      getToolExecutionDisplayStatus(
        execution({
          stdout: "usable output",
          stderr: "WARNING: noisy scanner banner",
          exit_code: 0,
        })
      )
    ).toBe("completed");
  });
});
