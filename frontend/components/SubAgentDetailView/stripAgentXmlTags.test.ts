import { render } from "@testing-library/react";
import { createElement } from "react";
import { describe, expect, it } from "vitest";
import {
  getSubAgentHeaderDisplayStatus,
  getSubAgentShellOutputFieldsForDetail,
  getSubAgentShellOutputForDetail,
  getSubAgentShellOutputJsonValueForDetail,
  getSubAgentToolDisplayStatus,
  isSubAgentShellLikeOutputTool,
  normalizeSubAgentEntriesForDetail,
  SUB_AGENT_DETAIL_PENDING_OUTPUT_SPINNER_CLASS,
  SUB_AGENT_DETAIL_RUNNING_SPINNER_CLASS,
  SUB_AGENT_HEADER_STATUS_BADGE_STYLES,
  SubAgentShellOutputText,
  stripAgentXmlTags,
} from "./SubAgentDetailView";

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

  it("leaves plain prose untouched", () => {
    expect(stripAgentXmlTags("just a normal answer")).toBe("just a normal answer");
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

describe("getSubAgentToolDisplayStatus", () => {
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
