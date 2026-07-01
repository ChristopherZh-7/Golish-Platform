import { describe, expect, it } from "vitest";
import { bodyRenderMode, prettyBody } from "./inspectorModel";

describe("inspectorModel", () => {
  it("picks json mode for json content-type and pretty-prints", () => {
    expect(bodyRenderMode("application/json")).toBe("json");
    expect(prettyBody("json", '{"a":1}')).toBe('{\n  "a": 1\n}');
  });

  it("falls back to text for html/plain bodies", () => {
    expect(bodyRenderMode("text/html")).toBe("text");
    expect(prettyBody("text", "<html>")).toBe("<html>");
  });

  it("keeps invalid json unchanged", () => {
    expect(prettyBody("json", "{oops")).toBe("{oops");
  });
});
