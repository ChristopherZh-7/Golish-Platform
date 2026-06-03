import { describe, expect, it } from "vitest";
import { classifyErrorSeverity } from "./errorSeverity";

describe("classifyErrorSeverity", () => {
  it("flags the planner-refusal family as a warning", () => {
    expect(
      classifyErrorSeverity(
        "Generator failed: The task planner declined to produce a plan — it returned a message instead of a plan."
      )
    ).toBe("warning");
  });

  it("classifies the same message identically once wrapped by the invoke rejection", () => {
    expect(
      classifyErrorSeverity(
        "[API trace=abc123] send_ai_prompt_session: Generator failed: The task planner declined to produce a plan."
      )
    ).toBe("warning");
  });

  it("matches case-insensitively", () => {
    expect(classifyErrorSeverity("IT RETURNED A MESSAGE INSTEAD OF A PLAN")).toBe("warning");
  });

  it("keeps real failures as hard errors", () => {
    expect(classifyErrorSeverity("Network error: connection refused")).toBe("error");
    expect(classifyErrorSeverity("Authentication failed (401)")).toBe("error");
    expect(classifyErrorSeverity("")).toBe("error");
  });
});
