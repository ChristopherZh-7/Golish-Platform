import { describe, expect, it } from "vitest";
import { getReconIntelSummary, getToolPrimaryArg } from "./tools";

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

describe("getReconIntelSummary", () => {
  it("flags OSINT when an ENScan provider ran (enrich)", () => {
    const summary = getReconIntelSummary("recon_enrich_assets", {
      providers: ["enscan-go-enrichment", "0.zone"],
      targets: 12,
      organizations: 1,
      promoted_children: 0,
    });
    expect(summary).not.toBeNull();
    expect(summary?.osint).toBe(true);
    expect(summary?.providers).toEqual(["enscan-go-enrichment", "0.zone"]);
    expect(summary?.targets).toBe(12);
    expect(summary?.organizations).toBe(1);
  });

  it("parses a JSON string result (timeline card path)", () => {
    const summary = getReconIntelSummary(
      "recon_discover_subsidiaries",
      JSON.stringify({ providers: ["enscan-go"], promoted_children: 3 })
    );
    expect(summary?.osint).toBe(true);
    expect(summary?.promotedChildren).toBe(3);
  });

  it("is not OSINT when only non-ENScan providers ran", () => {
    const summary = getReconIntelSummary("recon_enrich_assets", {
      providers: ["0.zone", "quake"],
    });
    expect(summary?.osint).toBe(false);
  });

  it("returns null for non recon-intel tools and empty/absent providers", () => {
    expect(getReconIntelSummary("pentest_run", { providers: ["enscan-go"] })).toBeNull();
    expect(getReconIntelSummary("recon_enrich_assets", { providers: [] })).toBeNull();
    expect(getReconIntelSummary("recon_enrich_assets", {})).toBeNull();
    expect(getReconIntelSummary("recon_enrich_assets", "not json")).toBeNull();
  });
});
