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

  it("flags a JSON-wrapped planner clarification (missing `subtasks`) as a warning", () => {
    expect(
      classifyErrorSeverity(
        'Generator failed: Failed to parse task planner JSON (missing field `subtasks` at line 3 column 1). Raw response: ```json { "message": "你好！请提供具体任务" }```'
      )
    ).toBe("warning");
  });

  it("treats a missing subtasks field as a soft warning even without the parse prefix", () => {
    expect(classifyErrorSeverity("missing field `subtasks` at line 1")).toBe("warning");
  });

  it("keeps real failures as hard errors", () => {
    expect(classifyErrorSeverity("Network error: connection refused")).toBe("error");
    expect(classifyErrorSeverity("Authentication failed (401)")).toBe("error");
    expect(classifyErrorSeverity("")).toBe("error");
  });
});
