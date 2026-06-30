import { fireEvent, render } from "@testing-library/react";
import { createElement } from "react";
import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "@/store";
import {
  extractAssetSubjectFromText,
  extractAssetSubjectFromToolCall,
  extractAssetSubjectsFromText,
  extractAssetSubjectsFromToolCall,
  getSubAgentHeaderDisplayStatus,
  getSubAgentLiveOutputForDetail,
  getSubAgentShellOutputFieldsForDetail,
  getSubAgentShellOutputForDetail,
  getSubAgentShellOutputJsonValueForDetail,
  getSubAgentToolCallVisualRelation,
  getSubAgentToolDisplayStatus,
  inferCoverageTechniquesFromToolCall,
  isSubAgentShellLikeOutputTool,
  isTerminalStageRunToolStatus,
  normalizeSubAgentEntriesForDetail,
  parseStageRefinerDirectiveSummary,
  parseStageRunOrgRequestId,
  resolveStageCoverageContextForSubAgent,
  SUB_AGENT_DETAIL_PENDING_OUTPUT_SPINNER_CLASS,
  SUB_AGENT_DETAIL_RUNNING_SPINNER_CLASS,
  SUB_AGENT_HEADER_STATUS_BADGE_STYLES,
  SubAgentDetailView,
  SubAgentShellOutputText,
  shouldSeparateSubAgentDetailEntries,
  stripAgentXmlTags,
  summarizeSubAgentAssetWork,
} from "./SubAgentDetailView";

beforeEach(() => {
  useStore.setState({
    activeSubAgents: {},
    backgroundJobs: {},
    sessions: {},
    timelines: {},
  });
});

describe("stripAgentXmlTags", () => {
  it("removes empty <tool_call></tool_call> wrappers", () => {
    expect(stripAgentXmlTags("<tool_call>\n</tool_call>")).toBe("");
  });

  it("removes a complete tool_call block with an inner function", () => {
    const text =
      "before\n<tool_call>\n<function=graph_search>\n<parameter=query>example.com</parameter>\n</function>\n</tool_call>\nafter";
    const out = stripAgentXmlTags(text);
    expect(out).not.toContain("<tool_call>");
    expect(out).not.toContain("</tool_call>");
    expect(out).not.toContain("<function=");
    expect(out).toContain("before");
    expect(out).toContain("after");
  });

  it("removes a lone/unterminated trailing <tool_call> tag", () => {
    const text = "Let me also check the evidence directory for more files.<tool_call>";
    const out = stripAgentXmlTags(text);
    expect(out).not.toContain("<tool_call>");
    expect(out).toContain("Let me also check the evidence directory for more files.");
  });

  it("still strips function/parameter and context tags", () => {
    const text =
      "<task_assignment>do x</task_assignment>\n<function=dns_resolve>\n<parameter=domain>example.com</parameter>\n</function>";
    const out = stripAgentXmlTags(text);
    expect(out).not.toContain("<function=");
    expect(out).not.toContain("<parameter=");
    expect(out).not.toContain("task_assignment");
  });

  it("removes DSML textual tool-call blocks leaked into sub-agent prose", () => {
    const text =
      'Let me submit directly:< | | DSML | | tool_calls>< | | DSML | | invoke name="submit_stage_deliverable">< | | DSML | | parameter name="claims" string="false">[{"subject":"http://115.159.235.124:8080"}]< / | | DSML | | parameter>< / | | DSML | | invoke>< / | | DSML | | tool_calls>done.';
    const out = stripAgentXmlTags(text);

    expect(out).toBe("Let me submit directly:done.");
    expect(out).not.toContain("DSML");
    expect(out).not.toContain("submit_stage_deliverable");
    expect(out).not.toContain("115.159.235.124");
  });

  it("removes full-width DSML textual tool-call blocks leaked into sub-agent prose", () => {
    const text =
      'Let me attempt a probe on route.moresec.cn to confirm it is unresolvable. <｜｜DSML｜｜tool_calls> <｜｜DSML｜｜invoke name="pentest_run"> <｜｜DSML｜｜parameter name="args" string="true">-host route.moresec.cn -top-ports 100</｜｜DSML｜｜parameter> <｜｜DSML｜｜parameter name="tool_name" string="true">naabu</｜｜DSML｜｜parameter> </｜｜DSML｜｜invoke> </｜｜DSML｜｜tool_calls>';
    const out = stripAgentXmlTags(text);

    expect(out).toBe(
      "Let me attempt a probe on route.moresec.cn to confirm it is unresolvable."
    );
    expect(out).not.toContain("DSML");
    expect(out).not.toContain("pentest_run");
    expect(out).not.toContain("naabu");
  });

  it("removes unterminated DSML blocks from the first leaked tag onward", () => {
    const text =
      'Before submit.<|DSML|tool_calls><|DSML|invoke name="submit_stage_deliverable"><|DSML|parameter name="coverage">[';

    expect(stripAgentXmlTags(text)).toBe("Before submit.");
  });

  it("leaves plain prose untouched", () => {
    expect(stripAgentXmlTags("just a normal answer")).toBe("just a normal answer");
  });
});

describe("SubAgentDetailView rendering", () => {
  it("renders stage-run backed detail without unstable selector loops", () => {
    const sessionId = "detail-session";
    const startedAt = "2026-06-28T14:00:00.000Z";
    const parentRequestId = "stage-run-request::org::org-1";

    useStore.setState({
      activeSubAgents: {
        [sessionId]: [
          {
            agentId: "agent-1",
            agentName: "Prober Agent",
            depth: 0,
            entries: [
              { kind: "thinking", text: "Reviewing the target batch.", startedAt: 1, endedAt: 2 },
              { kind: "text", text: "Let me probe the live services." },
            ],
            parentRequestId,
            startedAt,
            status: "running",
            task: "Probe services",
            toolCalls: [],
          },
        ],
      },
      backgroundJobs: {},
      sessions: {
        [sessionId]: {
          createdAt: startedAt,
          detailViewMode: "sub-agent-detail",
          id: sessionId,
          mode: "agent",
          name: "Detail Session",
          toolDetailRequestIds: [parentRequestId],
          workingDirectory: "/tmp",
        },
      },
      timelines: {
        [sessionId]: [
          {
            data: {
              args: {},
              requestId: "stage-run-request",
              startedAt,
              status: "running",
              toolName: "stage_run",
            },
            id: "stage-run-block",
            timestamp: startedAt,
            type: "ai_tool_execution",
          },
        ],
      },
    });

    const { container } = render(createElement(SubAgentDetailView, { sessionId }));

    expect(container.textContent).toContain("Prober Agent");
    expect(container.textContent).toContain("Let me probe the live services.");
  });

  it("renders live output for running direct tools such as js_extract_apis", () => {
    const sessionId = "direct-tool-session";
    const startedAt = "2026-06-29T10:00:00.000Z";
    const parentRequestId = "stage-run-request::org::org-1";

    useStore.setState({
      activeSubAgents: {
        [sessionId]: [
          {
            agentId: "agent-1",
            agentName: "Enumerator Agent",
            depth: 0,
            entries: [{ kind: "tool_call", toolCallId: "tool-js" }],
            parentRequestId,
            startedAt,
            status: "running",
            task: "Extract JS APIs",
            toolCalls: [
              {
                args: {
                  target_id: "target-1",
                  target_url: "https://example.com",
                },
                id: "tool-js",
                name: "js_extract_apis",
                startedAt,
                status: "running",
              },
            ],
          },
        ],
      },
      backgroundJobs: {},
      sessions: {
        [sessionId]: {
          createdAt: startedAt,
          detailViewMode: "sub-agent-detail",
          id: sessionId,
          mode: "agent",
          name: "Direct Tool Session",
          toolDetailRequestIds: [parentRequestId],
          workingDirectory: "/tmp",
        },
      },
      timelines: {},
    });

    const { container } = render(createElement(SubAgentDetailView, { sessionId }));
    const toolTrigger = Array.from(container.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("Using Js Extract Apis")
    );

    expect(toolTrigger).toBeTruthy();
    fireEvent.click(toolTrigger as HTMLButtonElement);

    expect(container.textContent).toContain("Using Js Extract Apis");
    expect(container.textContent).toContain("Output");
    expect(container.textContent).toContain("Waiting for output...");
  });
});

describe("parseStageRefinerDirectiveSummary", () => {
  it("returns null for ordinary agent output", () => {
    expect(parseStageRefinerDirectiveSummary("I've completed the scan.")).toBeNull();
  });

  it("extracts a compact summary from resumed coverage-gap directives", () => {
    const summary = parseStageRefinerDirectiveSummary(
      "Resuming submit repair: STAGE REFINER DIRECTIVE (deterministic, DB-backed): deterministic gate found 289 non-terminal coverage gap action(s)\n" +
        "Stage: external_attack_surface. Repair kind: CoverageGap. Do not restart the stage; perform only the actions below.\n" +
        "Allowed next tools: [pentest_list_tools, pentest_run, query_target_data, check_stage_asset_coverage, wait_for_background_jobs, check_job, kill_job, submit_stage_deliverable].\n" +
        "Forbidden in this repair: [list_in_scope_targets, list_attack_surface_seeds, manage_targets, manage_organizations].\n" +
        "Batching: EAS repair is batch-first.\n" +
        "Actions:\n 1. PORT/naabu: one pentest_run tool_name=naabu\n 2. SERVICE/nmap: group hosts\nThen call submit_stage_deliverable once."
    );

    expect(summary).toEqual({
      rootCause: "deterministic gate found 289 non-terminal coverage gap action(s)",
      stageLabel: "External Attack Surface",
      repairKindLabel: "Coverage Gap",
      gapCount: 289,
      actionCount: 2,
      allowedTools: [
        "pentest_list_tools",
        "pentest_run",
        "query_target_data",
        "check_stage_asset_coverage",
        "wait_for_background_jobs",
        "check_job",
        "kill_job",
        "submit_stage_deliverable",
      ],
      forbiddenTools: [
        "list_in_scope_targets",
        "list_attack_surface_seeds",
        "manage_targets",
        "manage_organizations",
      ],
      batchFirst: true,
    });
  });

  it("extracts evidence-ref repair directives without a gap count", () => {
    const summary = parseStageRefinerDirectiveSummary(
      "STAGE REFINER DIRECTIVE (deterministic, DB-backed): deliverable evidence references are missing\n" +
        "Stage: target_intel. Repair kind: EvidenceRefs. Do not restart the stage.\n" +
        "Allowed next tools: [query_target_data, submit_stage_deliverable].\n" +
        "Actions:\n 1. map real evidence ids to claims"
    );

    expect(summary).toMatchObject({
      rootCause: "deliverable evidence references are missing",
      stageLabel: "Target Intel",
      repairKindLabel: "Evidence Refs",
      gapCount: null,
      actionCount: 1,
      allowedTools: ["query_target_data", "submit_stage_deliverable"],
      batchFirst: false,
    });
  });
});

describe("getSubAgentShellOutputForDetail", () => {
  it("shows a pending output placeholder while a sub-agent pentest_run is running", () => {
    expect(getSubAgentShellOutputForDetail({ status: "running" })).toEqual({
      text: "Waiting for output...",
      pending: true,
    });
  });

  it("uses streaming output before the final result arrives", () => {
    expect(
      getSubAgentShellOutputForDetail({
        status: "running",
        streamingOutput: "httpx result\n",
      })
    ).toEqual({
      text: "httpx result",
      pending: false,
    });
  });

  it("shows partial output from a backgrounded sub-agent shell-like tool", () => {
    expect(
      getSubAgentShellOutputForDetail({
        status: "backgrounded",
        result: { status: "backgrounded", partial_stdout: "whatweb scanning...\n" },
      })
    ).toEqual({
      text: "whatweb scanning...",
      pending: false,
    });
  });

  it("does not keep a stopped child tool in pending output mode", () => {
    expect(getSubAgentShellOutputForDetail({ status: "interrupted" })).toEqual({
      text: null,
      pending: false,
    });
  });

  it("keeps live shell-like output in terminal mode instead of completed JSON mode", () => {
    expect(
      getSubAgentShellOutputJsonValueForDetail({
        status: "running",
        result: { stdout: "still running\n", stderr: "", exit_code: 0 },
      })
    ).toBeNull();
  });

  it("treats background tool-wrapper args as shell-like sub-agent output", () => {
    expect(
      isSubAgentShellLikeOutputTool({
        name: "whatweb",
        args: {
          tool_name: "whatweb",
          args: "-a 1 https://example.com",
          background: true,
          timeout_secs: 60,
        },
      })
    ).toBe(true);
  });

  it("combines final stdout and stderr for shell-like sub-agent tools", () => {
    expect(
      getSubAgentShellOutputForDetail({
        status: "completed",
        result: { stdout: "ok\n", stderr: "warn\n", exit_code: 0 },
      })
    ).toEqual({
      text: "ok\n\nstderr:\nwarn",
      pending: false,
    });
  });

  it("keeps an output panel visible for completed sub-agent shell-like tools with empty output", () => {
    expect(
      getSubAgentShellOutputForDetail({
        status: "completed",
        result: { stdout: "", stderr: "", exit_code: 0 },
      })
    ).toEqual({
      text: "No output.",
      pending: false,
    });
  });

  it("keeps structured shell results as output key/value fields", () => {
    expect(
      getSubAgentShellOutputFieldsForDetail({
        result: {
          stdout: "\x1b[1;31mFTL\x1b[0m blocked\n",
          stderr: "permission denied\n",
          exit_code: 1,
        },
      })
    ).toEqual([
      { key: "stdout", value: "FTL blocked" },
      { key: "stderr", value: "permission denied" },
      { key: "exit_code", value: "1" },
    ]);
  });

  it("converts structured shell results into the same JSON value shape as input", () => {
    expect(
      getSubAgentShellOutputJsonValueForDetail({
        result: { stdout: "ok\n", stderr: "warn\n", exit_code: 0 },
      })
    ).toEqual({ stdout: "ok", stderr: "warn", exit_code: "0" });
  });

  it("uses readable running spinners in sub-agent detail surfaces", () => {
    expect(SUB_AGENT_DETAIL_RUNNING_SPINNER_CLASS).toContain("h-4");
    expect(SUB_AGENT_DETAIL_RUNNING_SPINNER_CLASS).toContain("w-4");
    expect(SUB_AGENT_DETAIL_PENDING_OUTPUT_SPINNER_CLASS).toContain("h-4");
    expect(SUB_AGENT_DETAIL_PENDING_OUTPUT_SPINNER_CLASS).toContain("w-4");
  });

  it("renders ANSI color codes instead of leaking raw escape fragments", () => {
    const { container } = render(
      createElement(SubAgentShellOutputText, {
        text: "\x1b[1;31mFTL\x1b[0m Could not open/create output file",
      })
    );

    expect(container.textContent).toContain("FTL Could not open/create output file");
    expect(container.textContent).not.toContain("[1;31m");
    expect(container.textContent).not.toContain("[0m");
  });
});

describe("getSubAgentLiveOutputForDetail", () => {
  it("shows a pending output placeholder for running direct tools", () => {
    expect(getSubAgentLiveOutputForDetail({ status: "running" })).toEqual({
      text: "Waiting for output...",
      pending: true,
    });
  });

  it("normalizes streamed chunks for direct tools", () => {
    expect(
      getSubAgentLiveOutputForDetail({
        status: "running",
        streamingOutput: "loaded 2 scripts\nfound 5 endpoints\n",
      })
    ).toEqual({
      text: "loaded 2 scripts\nfound 5 endpoints",
      pending: false,
    });
  });
});

describe("sub-agent asset work summary", () => {
  it("extracts the active asset from wrapped pentest_run args", () => {
    expect(
      extractAssetSubjectFromToolCall({
        name: "pentest_run",
        args: {
          tool_name: "nmap",
          args: "-sV 10.18.2.4 -p 443",
        },
      })
    ).toBe("10.18.2.4");
  });

  it("extracts urls and domains from command text", () => {
    expect(extractAssetSubjectFromText("httpx -title https://pay.example.com:8443")).toBe(
      "https://pay.example.com:8443"
    );
    expect(extractAssetSubjectFromText("whatweb --no-errors api.example.com")).toBe(
      "api.example.com"
    );
  });

  it("extracts multiple assets from batch commands", () => {
    expect(
      extractAssetSubjectsFromText(
        "httpx https://a.example.com https://b.example.com 10.18.2.4:443"
      )
    ).toEqual(["https://a.example.com", "https://b.example.com", "10.18.2.4:443"]);
    expect(
      extractAssetSubjectsFromToolCall({
        name: "pentest_run",
        args: {
          tool_name: "nmap",
          args: "-sV 10.18.2.4 10.18.2.5 -p 80,443",
        },
      })
    ).toEqual(["10.18.2.4", "10.18.2.5"]);
  });

  it("extracts multiple assets from pentest_run batch input lines", () => {
    expect(
      extractAssetSubjectsFromToolCall({
        name: "pentest_run",
        args: {
          tool_name: "naabu",
          args: "-list {{input_file}} -top-ports 1000 -s c -silent",
          input_lines: ["120.233.149.102", "yun.pingan.com.cdn.pingan.com.cn"],
        },
      })
    ).toEqual(["120.233.149.102", "yun.pingan.com.cdn.pingan.com.cn"]);
  });

  it("maps active EAS tools to coverage dimensions", () => {
    expect(
      inferCoverageTechniquesFromToolCall({
        name: "pentest_run",
        args: { tool_name: "httpx", args: "-json https://edge.example.com" },
      })
    ).toEqual(["LIVENESS"]);
    expect(
      inferCoverageTechniquesFromToolCall({
        name: "pentest_run",
        args: { tool_name: "nmap", args: "-sV 10.18.2.4 -p 443" },
      })
    ).toEqual(["PORT", "SERVICE"]);
  });

  it("summarizes running tool calls into asset work items", () => {
    expect(
      summarizeSubAgentAssetWork([
        {
          id: "tool-1",
          name: "pentest_run",
          args: {
            tool_name: "httpx",
            args: "-json https://edge.example.com",
          },
          status: "running",
          startedAt: "2026-06-27T10:00:00.000Z",
          streamingOutput: "probing\nhttps://edge.example.com [200]\n",
        },
        {
          id: "tool-2",
          name: "submit_stage_deliverable",
          args: {},
          status: "running",
          startedAt: "2026-06-27T10:00:03.000Z",
        },
      ])
    ).toEqual([
      expect.objectContaining({
        id: "tool-1",
        displayToolName: "httpx",
        subject: "https://edge.example.com",
        subjects: ["https://edge.example.com"],
        techniques: ["LIVENESS"],
        status: "running",
        outputPreview: "https://edge.example.com [200]",
      }),
    ]);
  });

  it("summarizes batch pentest runs with expanded subjects", () => {
    expect(
      summarizeSubAgentAssetWork([
        {
          id: "tool-batch",
          name: "pentest_run",
          args: {
            tool_name: "naabu",
            args: "-list {{input_file}} -top-ports 1000 -s c -silent",
            input_lines: ["120.233.149.102", "yun.pingan.com.cdn.pingan.com.cn"],
          },
          status: "running",
          startedAt: "2026-06-27T10:00:00.000Z",
        },
      ])
    ).toEqual([
      expect.objectContaining({
        id: "tool-batch",
        displayToolName: "naabu",
        subject: "120.233.149.102",
        subjects: ["120.233.149.102", "yun.pingan.com.cdn.pingan.com.cn"],
        techniques: ["PORT"],
        primary:
          "Naabu · batch 2 targets (120.233.149.102 ... yun.pingan.com.cdn.pingan.com.cn) · top 1000 ports",
      }),
    ]);
  });
});

describe("normalizeSubAgentEntriesForDetail", () => {
  it("removes short streaming text prefixes covered by a later accumulated text entry", () => {
    expect(
      normalizeSubAgentEntriesForDetail([
        { kind: "text", text: "n" },
        { kind: "thinking", text: "thinking", startedAt: 1, endedAt: 2 },
        { kind: "text", text: "nmap needs root for SYN scan." },
      ])
    ).toEqual([
      { kind: "thinking", text: "thinking", startedAt: 1, endedAt: 2 },
      { kind: "text", text: "nmap needs root for SYN scan." },
    ]);
  });

  it("keeps matching prefixes once a tool call creates a new response boundary", () => {
    expect(
      normalizeSubAgentEntriesForDetail([
        { kind: "text", text: "Let me run" },
        { kind: "tool_call", toolCallId: "tool-1" },
        { kind: "text", text: "Let me run the next probe." },
      ])
    ).toEqual([
      { kind: "text", text: "Let me run" },
      { kind: "tool_call", toolCallId: "tool-1" },
      { kind: "text", text: "Let me run the next probe." },
    ]);
  });
});

describe("shouldSeparateSubAgentDetailEntries", () => {
  it("keeps thought and agent output in the same visual group", () => {
    expect(
      shouldSeparateSubAgentDetailEntries({ kind: "thinking" }, { kind: "text" })
    ).toBe(false);
    expect(
      shouldSeparateSubAgentDetailEntries({ kind: "text" }, { kind: "thinking" })
    ).toBe(false);
  });

  it("connects a tool call to the preceding agent narrative", () => {
    expect(
      shouldSeparateSubAgentDetailEntries({ kind: "text" }, { kind: "tool_call" })
    ).toBe(false);
    expect(getSubAgentToolCallVisualRelation({ kind: "text" })).toBe("after_narrative");
    expect(getSubAgentToolCallVisualRelation({ kind: "thinking" })).toBe("after_narrative");
  });

  it("starts a new visual group after tool calls", () => {
    expect(
      shouldSeparateSubAgentDetailEntries({ kind: "tool_call" }, { kind: "thinking" })
    ).toBe(true);
    expect(
      shouldSeparateSubAgentDetailEntries({ kind: "tool_call" }, { kind: "tool_call" })
    ).toBe(false);
    expect(getSubAgentToolCallVisualRelation({ kind: "tool_call" })).toBe("stacked");
    expect(getSubAgentToolCallVisualRelation(null)).toBe("standalone");
  });
});

describe("stage-run org coverage context", () => {
  it("parses a stage_run org specialist request id", () => {
    expect(parseStageRunOrgRequestId("tool-1::org::org-1")).toEqual({
      stageRunRequestId: "tool-1",
      organizationId: "org-1",
    });
    expect(parseStageRunOrgRequestId("plain-tool")).toBeNull();
  });

  it("resolves target_intel coverage context for the current sub-agent detail page", () => {
    expect(
      resolveStageCoverageContextForSubAgent(
        "tool-1::org::org-1",
        {
          "tool-1": {
            requestId: "tool-1",
            stageLabel: "Target Intel",
            roleLabel: "Recon",
            coverageAxis: ["DNS", "WHOIS", "ASN", "CT", "Subdomain", "OSINT"],
            summary: { total: 1, covered: 1, active: 0, queued: 0, blocked: 0 },
            rows: [
              {
                id: "org-1",
                name: "Acme Root",
                ownershipPercent: 100,
                status: "passed",
                agentRequestId: "tool-1::org::org-1",
                evidenceCount: 3,
                coverage: { DNS: "found" },
                stage: "target_intel",
              },
            ],
          },
        },
        null
      )
    ).toEqual({
      organizationId: "org-1",
      organizationName: "Acme Root",
      stage: "target_intel",
      stageLabel: "Target Intel",
    });
  });
});

describe("getSubAgentToolDisplayStatus", () => {
  it("projects a stale running sub-agent tool as interrupted after its parent stage stopped", () => {
    expect(
      getSubAgentToolDisplayStatus(
        {
          status: "running",
          result: undefined,
        },
        { parentStageStopped: true }
      )
    ).toBe("interrupted");
  });

  it("treats completed sub-agent tool payload failures as error", () => {
    expect(
      getSubAgentToolDisplayStatus({
        status: "completed",
        result: {
          stdout:
            "WhatWeb is not installed and is missing dependencies.\nThe following gems are missing:\n - addressable",
          stderr: "",
          exit_code: 0,
        },
      })
    ).toBe("error");
  });

  it("keeps partial recon provider surveys completed when only a nested provider failed", () => {
    expect(
      getSubAgentToolDisplayStatus({
        status: "completed",
        result: {
          status: "partial",
          action: "map_assets",
          providerStatus: [
            { providerId: "0.zone", status: "failed", message: "HTTP 502" },
            { providerId: "quake", status: "completed", message: "normalized 419 record(s)" },
          ],
        },
      })
    ).toBe("completed");
  });

  it("keeps completed sub-agent tool warnings as completed", () => {
    expect(
      getSubAgentToolDisplayStatus({
        status: "completed",
        result: { stdout: "usable output", stderr: "WARNING: noisy scanner banner", exit_code: 0 },
      })
    ).toBe("completed");
  });
});

describe("getSubAgentHeaderDisplayStatus", () => {
  it("projects a stale running sub-agent header as interrupted after its parent stage stopped", () => {
    expect(
      getSubAgentHeaderDisplayStatus(
        {
          status: "running",
          toolCalls: [
            {
              id: "tool-1",
              name: "recon_map_assets",
              args: {},
              status: "running",
              startedAt: "2026-06-24T09:00:00.000Z",
            },
          ],
        },
        { parentStageStopped: true }
      )
    ).toBe("interrupted");
  });

  it("keeps a completed sub-agent visually running while a tool is still running", () => {
    expect(
      getSubAgentHeaderDisplayStatus({
        status: "completed",
        toolCalls: [
          {
            id: "tool-1",
            name: "pentest_run",
            args: {},
            status: "running",
            startedAt: "2026-06-24T09:00:00.000Z",
          },
        ],
      })
    ).toBe("running");
  });

  it("shows error when a completed sub-agent's latest tool failed", () => {
    expect(
      getSubAgentHeaderDisplayStatus({
        status: "completed",
        toolCalls: [
          {
            id: "tool-1",
            name: "submit_stage_deliverable",
            args: {},
            status: "error",
            startedAt: "2026-06-24T09:00:00.000Z",
            completedAt: "2026-06-24T09:00:01.000Z",
          },
        ],
      })
    ).toBe("error");
  });

  it("uses high-contrast colors for live header status badges", () => {
    expect(SUB_AGENT_HEADER_STATUS_BADGE_STYLES.running.badgeClass).toContain(
      "text-[var(--ansi-blue)]"
    );
    expect(SUB_AGENT_HEADER_STATUS_BADGE_STYLES.running.badgeClass).toContain(
      "border-[var(--ansi-blue)]/45"
    );
    expect(SUB_AGENT_HEADER_STATUS_BADGE_STYLES.backgrounded.badgeClass).toContain(
      "text-amber-300"
    );
  });
});

describe("isTerminalStageRunToolStatus", () => {
  it("treats interrupted/completed/error parent stage_run states as terminal", () => {
    expect(isTerminalStageRunToolStatus("interrupted")).toBe(true);
    expect(isTerminalStageRunToolStatus("completed")).toBe(true);
    expect(isTerminalStageRunToolStatus("error")).toBe(true);
  });

  it("does not stop child detail while the parent stage_run is still live or unknown", () => {
    expect(isTerminalStageRunToolStatus("running")).toBe(false);
    expect(isTerminalStageRunToolStatus("backgrounded")).toBe(false);
    expect(isTerminalStageRunToolStatus(null)).toBe(false);
  });
});
