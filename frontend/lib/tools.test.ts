import { describe, expect, it } from "vitest";
import { getToolActionLabel, getToolPrimaryArg } from "./tools";

describe("getToolActionLabel", () => {
  it("renders wait_for_background_jobs as a sentence-like action", () => {
    expect(getToolActionLabel("wait_for_background_jobs")).toBe("Waiting for background jobs");
  });

  it("renders wrapped pentest_run tools as readable actions", () => {
    expect(getToolActionLabel("pentest_run", { tool_name: "whatweb" })).toBe(
      "Fingerprinting web services"
    );
  });

  it("renders EAS web fingerprint wrapper as a readable action", () => {
    expect(getToolActionLabel("eas_fingerprint_web_stack")).toBe(
      "Fingerprinting web services"
    );
  });

  it("summarizes nmap service probes by intent instead of command syntax", () => {
    expect(
      getToolActionLabel("pentest_run", {
        tool_name: "nmap",
        args: "-sV -iL {{input_file}} -p 80,443,10180 -T3",
      })
    ).toBe("Probing services");
  });

  it("summarizes port scanners by intent", () => {
    expect(getToolActionLabel("pentest_run", { tool_name: "naabu" })).toBe("Scanning ports");
  });

  it("renders enumeration crawler wrapper as a readable action", () => {
    expect(getToolActionLabel("enum_crawl_same_origin_urls")).toBe(
      "Crawling same-origin URLs"
    );
  });

  it("renders vuln formulaic wrapper as a readable action", () => {
    expect(getToolActionLabel("vuln_run_formulaic_sweep")).toBe(
      "Running formulaic vuln sweep"
    );
  });

  it("falls back without exposing snake_case underscores", () => {
    expect(getToolActionLabel("custom_internal_tool")).toBe("Using Custom Internal Tool");
  });
});

describe("getToolPrimaryArg", () => {
  it("summarizes pentest_run as '<tool> · <context>'", () => {
    expect(
      getToolPrimaryArg("pentest_run", {
        tool_name: "dig",
        args: "example.com NS +noall +answer",
      })
    ).toBe("dig · example.com NS +noall +answer");
  });

  it("handles pentest_run with no args", () => {
    expect(getToolPrimaryArg("pentest_run", { tool_name: "nmap" })).toBeNull();
  });

  it("summarizes wait_for_background_jobs timeout while collapsed", () => {
    expect(getToolPrimaryArg("wait_for_background_jobs", { timeout_secs: 180 })).toBe(
      "wait up to 180s"
    );
  });

  it("shows the wait_for_background_jobs default when no timeout is provided", () => {
    expect(getToolPrimaryArg("wait_for_background_jobs", {})).toBe("default wait up to 300s");
  });

  it("includes custom wait_for_background_jobs polling cadence", () => {
    expect(
      getToolPrimaryArg("wait_for_background_jobs", {
        timeout_secs: 180,
        poll_interval_ms: 500,
      })
    ).toBe("wait up to 180s | poll 500ms");
  });

  it("summarizes pentest_run batch input lines", () => {
    expect(
      getToolPrimaryArg("pentest_run", {
        tool_name: "naabu",
        args: "-list {{input_file}} -top-ports 1000 -s c -silent",
        input_lines: ["113.105.78.99", "118.190.17.199", "120.233.149.95"],
      })
    ).toBe(
      "Naabu · batch 3 targets (113.105.78.99 ... 120.233.149.95) · top 1000 ports"
    );
  });

  it("summarizes pentest_run stdin batches", () => {
    expect(
      getToolPrimaryArg("pentest_run", {
        tool_name: "httpx",
        args: "-silent",
        stdin: "\nhttps://a.example.com\nhttps://b.example.com\n",
      })
    ).toBe("HTTPX · batch 2 targets (https://a.example.com ... https://b.example.com)");
  });

  it("summarizes nmap batch service probes without repeating the raw command", () => {
    expect(
      getToolPrimaryArg("pentest_run", {
        tool_name: "nmap",
        args: "-sV -iL {{input_file}} -p 80,443,10180 -T3",
        input_lines: ["10.0.0.1", "10.0.0.2", "10.0.0.3"],
      })
    ).toBe("Nmap · batch 3 targets (10.0.0.1 ... 10.0.0.3) · ports 80,443,10180");
  });

  it("returns null for pentest_run without a tool_name", () => {
    expect(getToolPrimaryArg("pentest_run", {})).toBeNull();
  });

  it("summarizes enumeration crawler wrapper targets", () => {
    expect(
      getToolPrimaryArg("enum_crawl_same_origin_urls", {
        target_urls: [
          "https://a.example.com/",
          "https://b.example.com/",
          "https://c.example.com/",
        ],
        depth: 2,
      })
    ).toBe("batch 3 targets (https://a.example.com/ ... https://c.example.com/) · depth 2");
  });

  it("summarizes vuln formulaic wrapper targets and techniques", () => {
    expect(
      getToolPrimaryArg("vuln_run_formulaic_sweep", {
        targets: ["https://a.example.com/", "https://b.example.com/"],
        techniques: ["WSTG-INPV-05", "WSTG-INPV-01"],
      })
    ).toBe("batch 2 targets (https://a.example.com/ ... https://b.example.com/) · 2 techniques");
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
