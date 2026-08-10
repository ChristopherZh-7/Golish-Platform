import { describe, expect, it } from "vitest";
import {
  getStageRunAgentLabel,
  getToolActionLabel,
  getToolPrimaryArg,
  getToolTerminalPresentation,
  toolResultIndicatesFailure,
} from "./tools";

describe("getToolTerminalPresentation", () => {
  it("does not leave an immutable accepted submission claiming Gate is still awaiting", () => {
    expect(
      getToolTerminalPresentation("submit_stage_deliverable", { status: "accepted" }, true)
    ).toEqual({ kind: "submitted", label: "Submitted" });
  });

  it("distinguishes stage_run BLOCK from a server-ready closeout", () => {
    expect(getToolTerminalPresentation("stage_run", { passed: false }, true)).toEqual({
      kind: "blocked",
      label: "Blocked",
    });
    expect(getToolTerminalPresentation("stage_run", { passed: true }, true)).toEqual({
      kind: "ready_to_close",
      label: "Ready to close",
    });
  });

  it("retains ordinary success and failure semantics", () => {
    expect(getToolTerminalPresentation("read_file", { content: "ok" }, true)).toEqual({
      kind: "completed",
      label: "Completed",
    });
    expect(getToolTerminalPresentation("read_file", { status: "needs_fix" }, true)).toEqual({
      kind: "failed",
      label: "Needs attention",
    });
  });
});

describe("getStageRunAgentLabel", () => {
  it("renders the durable Controller role as one consistent product label", () => {
    expect(getStageRunAgentLabel("company_stage_controller")).toBe("Company Controller");
    expect(getStageRunAgentLabel("Company Controller")).toBe("Company Controller");
    expect(getStageRunAgentLabel("Application Model Controller")).toBe(
      "Application Model Controller"
    );
  });

  it("humanizes downstream specialist slugs without changing ordinary labels", () => {
    expect(getStageRunAgentLabel("vuln_scanner")).toBe("Vuln Scanner Agent");
    expect(getStageRunAgentLabel("Prober")).toBe("Prober Agent");
  });
});

describe("toolResultIndicatesFailure", () => {
  const storageHalt = {
    scheduler: "company_controller_v1",
    passed: false,
    operator_recovery_required: false,
    retry_budget_exhausted: true,
    runtime_control: {
      kind: "halt_current_request",
      reason: "company_controller_storage_failed",
    },
    gaps: [{ code: "COMPANY_CONTROLLER_STORAGE_FAILED" }],
  };

  it("marks an exact Company Controller storage halt as a displayed failure", () => {
    expect(toolResultIndicatesFailure(storageHalt)).toBe(true);
    expect(toolResultIndicatesFailure(JSON.stringify(storageHalt))).toBe(true);
  });

  it("rejects lookalike storage-halt payloads", () => {
    expect(
      toolResultIndicatesFailure({
        ...storageHalt,
        gaps: [{ code: "COMPANY_CONTROLLER_FAILED" }],
      })
    ).toBe(false);
    expect(
      toolResultIndicatesFailure({
        ...storageHalt,
        runtime_control: {
          kind: "halt_current_request",
          reason: "company_controller_blocked",
        },
      })
    ).toBe(false);
    expect(toolResultIndicatesFailure({ ...storageHalt, passed: true })).toBe(false);
    expect(toolResultIndicatesFailure({ ...storageHalt, scheduler: "other" })).toBe(false);
  });
});

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

  it("renders both Nuclei wrappers as distinct readable actions", () => {
    expect(getToolActionLabel("vuln_nuclei_general")).toBe("Running general Nuclei scan");
    expect(getToolActionLabel("vuln_nuclei_fingerprint_targeted")).toBe(
      "Running fingerprint-targeted Nuclei scan"
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

  it("summarizes singular Nuclei wrapper target and techniques", () => {
    expect(
      getToolPrimaryArg("vuln_nuclei_general", {
        target_id: "11111111-1111-1111-1111-111111111111",
        target_url: "https://a.example.com/",
        techniques: ["WSTG-INPV-05", "WSTG-INPV-01"],
      })
    ).toBe("https://a.example.com/ · 2 techniques");
    expect(
      getToolPrimaryArg("vuln_nuclei_fingerprint_targeted", {
        target_id: "22222222-2222-2222-2222-222222222222",
        target_url: "https://cms.example.com/",
        techniques: ["GOLISH-NDAY"],
      })
    ).toBe("https://cms.example.com/ · GOLISH-NDAY");
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
