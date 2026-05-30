import { describe, expect, it } from "vitest";
import { ApiError, parseGolishError } from "./client";

describe("parseGolishError", () => {
  it("extracts code+message from a structured GolishError", () => {
    expect(parseGolishError({ code: "NOT_FOUND", message: "Not found: x" })).toEqual({
      code: "NOT_FOUND",
      message: "Not found: x",
    });
  });

  it("falls back to UNKNOWN for legacy bare-string errors", () => {
    expect(parseGolishError("boom")).toEqual({ code: "UNKNOWN", message: "boom" });
  });

  it("falls back to UNKNOWN for Error instances", () => {
    expect(parseGolishError(new Error("kaboom"))).toEqual({
      code: "UNKNOWN",
      message: "kaboom",
    });
  });
});

describe("ApiError", () => {
  it("exposes the parsed code and threads traceId + command + message", () => {
    const e = new ApiError("pipeline_list", { code: "PIPELINE", message: "boom" }, "ab12cd34");
    expect(e.code).toBe("PIPELINE");
    expect(e.message).toContain("ab12cd34");
    expect(e.message).toContain("pipeline_list");
    expect(e.message).toContain("boom");
  });

  it("does not stringify object cause as [object Object]", () => {
    const e = new ApiError("x_cmd", { code: "INTERNAL", message: "real text" }, "ffffffff");
    expect(e.message).not.toContain("[object Object]");
    expect(e.message).toContain("real text");
  });
});
