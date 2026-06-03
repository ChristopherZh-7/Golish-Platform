import { describe, expect, it } from "vitest";
import { getToolPrimaryArg } from "./tools";

describe("getToolPrimaryArg", () => {
  it("summarizes pentest_run as '<tool> <args>'", () => {
    expect(
      getToolPrimaryArg("pentest_run", {
        tool_name: "dig",
        args: "example.com NS +noall +answer",
      })
    ).toBe("dig example.com NS +noall +answer");
  });

  it("handles pentest_run with no args", () => {
    expect(getToolPrimaryArg("pentest_run", { tool_name: "nmap" })).toBe("nmap");
  });

  it("returns null for pentest_run without a tool_name", () => {
    expect(getToolPrimaryArg("pentest_run", {})).toBeNull();
  });

  it("shows the command for shell tools", () => {
    expect(getToolPrimaryArg("run_command", { command: "ls -la" })).toBe("ls -la");
  });

  it("falls back to common arg fields", () => {
    expect(getToolPrimaryArg("read_file", { path: "/tmp/x" })).toBe("/tmp/x");
    expect(getToolPrimaryArg("fetch", { url: "https://example.com" })).toBe("https://example.com");
  });

  it("returns null when nothing matches", () => {
    expect(getToolPrimaryArg("unknown", { foo: "bar" })).toBeNull();
  });
});
