import { describe, expect, it } from "vitest";
import type { SubAgentToolCall } from "@/store";
import {
  presentToolActivity,
  summarizeToolActivities,
} from "./toolActivityPresentation";

function tool(overrides: Partial<SubAgentToolCall>): SubAgentToolCall {
  return {
    id: "tool-1",
    name: "eas_discover_ports",
    args: {},
    status: "completed",
    startedAt: "2026-08-11T00:00:00Z",
    ...overrides,
  };
}

describe("presentToolActivity", () => {
  it("reads the exact backend command and partial output from an EAS result", () => {
    const presentation = presentToolActivity(
      tool({
        args: { targets: ["192.0.2.10"], scan_profile: "standard" },
        status: "backgrounded",
        result: {
          command: "naabu -list '/tmp/input file.txt' -top-ports 1000",
          partial_stdout: "192.0.2.10:443\n",
          partial_stderr: "warming up\n",
          job_id: "job_ports",
          hint: "Managed process is still running",
        },
      })
    );

    expect(presentation).toMatchObject({
      action: "Scanning ports",
      completedAction: "Scanned ports",
      runner: "Naabu",
      subject: "1 target · standard profile",
      command: "naabu -list '/tmp/input file.txt' -top-ports 1000",
      commandProvenance: "executed",
      stdout: "192.0.2.10:443\n",
      stderr: "warming up\n",
      jobId: "job_ports",
      hint: "Managed process is still running",
    });
  });

  it("normalizes a historical JSON-string result once", () => {
    const presentation = presentToolActivity(
      tool({
        name: "eas_probe_http_liveness",
        result: JSON.stringify({
          command: "httpx -l /tmp/targets -json -silent",
          stdout: "https://example.test [200]",
        }),
      })
    );

    expect(presentation.command).toBe("httpx -l /tmp/targets -json -silent");
    expect(presentation.commandProvenance).toBe("executed");
    expect(presentation.stdout).toBe("https://example.test [200]");
  });

  it("prefers live streaming output and keeps a shell args command labelled as requested", () => {
    const presentation = presentToolActivity(
      tool({
        name: "run_command",
        args: { command: "printf '\\\\n'" },
        streamingOutput: "live output",
        result: { stdout: "older output", partial_stderr: "already included in stream" },
        status: "running",
      })
    );

    expect(presentation.command).toBe("printf '\\\\n'");
    expect(presentation.commandProvenance).toBe("requested");
    expect(presentation.stdout).toBe("live output");
    expect(presentation.stderr).toBeNull();
  });

  it("replaces a requested shell command with the backend execution fact when it arrives", () => {
    const presentation = presentToolActivity(
      tool({
        name: "run_command",
        args: { command: "echo requested\\ntext" },
        result: {
          command: "env -i sh -lc 'echo requested\\ntext'",
          stdout: "requested\\ntext",
        },
      })
    );

    expect(presentation.command).toBe("env -i sh -lc 'echo requested\\ntext'");
    expect(presentation.commandProvenance).toBe("executed");
  });

  it("reads the exact whitelisted Nuclei runner execution command", () => {
    const presentation = presentToolActivity(
      tool({
        name: "vuln_nuclei_general",
        args: {
          target_id: "target-1",
          target_url: "https://api.example.test:443/",
        },
        result: {
          completion_state: "complete",
          runner_execution: {
            command: "nuclei -list '/tmp/golish-input.txt' -jsonl -silent",
            exit_code: 0,
            duration_ms: 1234,
          },
          report: { matches: [] },
        },
      })
    );

    expect(presentation).toMatchObject({
      action: "Scanning with Nuclei",
      completedAction: "Scanned with Nuclei",
      runner: "Nuclei",
      command: "nuclei -list '/tmp/golish-input.txt' -jsonl -silent",
      commandProvenance: "executed",
    });
  });

  it("does not reconstruct a command for wrapped or non-command tools", () => {
    const wrapped = presentToolActivity(
      tool({
        name: "pentest_run",
        args: { tool_name: "naabu", args: "-host 192.0.2.10 -p 443" },
        result: { response: { command: "nested command must not be searched" } },
      })
    );
    const databaseTool = presentToolActivity(
      tool({
        name: "query_target_data",
        args: { query: "all targets" },
        result: { rows: [] },
      })
    );
    const encodedNestedRunner = presentToolActivity(
      tool({
        name: "vuln_nuclei_general",
        result: {
          runner_execution: JSON.stringify({ command: "nested JSON must not be parsed" }),
        },
      })
    );

    expect(wrapped.command).toBeNull();
    expect(wrapped.commandProvenance).toBeNull();
    expect(databaseTool.command).toBeNull();
    expect(databaseTool.commandProvenance).toBeNull();
    expect(encodedNestedRunner.command).toBeNull();
    expect(encodedNestedRunner.commandProvenance).toBeNull();
  });

  it("presents exact in-process HTTP observations without inventing a command", () => {
    const presentation = presentToolActivity(
      tool({
        name: "vuln_probe_anonymous_access",
        result: {
          exact_origin: "https://api.example.test:443",
          selected_count: 2,
          network_attempted: true,
          completion_state: "complete",
          observations: [
            {
              endpoint_id: "00000000-0000-0000-0000-000000000001",
              method: "GET",
              path: "/admin",
              query_bindings: [{ name: "tenant", value: "42" }],
              network_attempted: true,
              status_code: 200,
              verdict: "suspicious",
              error_class: null,
              response: {
                content_type_family: "json",
                declared_length: 4096,
                captured_length: 1024,
                prefix_sha256: "a".repeat(64),
                truncated: true,
              },
            },
            {
              endpoint_id: "00000000-0000-0000-0000-000000000002",
              method: "HEAD",
              path: "/profile",
              query_bindings: [],
              network_attempted: true,
              status_code: null,
              verdict: "inconclusive",
              error_class: "request_timeout",
              response: null,
            },
          ],
        },
      })
    );

    expect(presentation).toMatchObject({
      action: "Probing anonymous access",
      completedAction: "Probed anonymous access",
      runner: "Golish HTTP client",
      subject: "https://api.example.test:443",
      execution: {
        kind: "http",
        origin: "https://api.example.test:443",
        selectedCount: 2,
        networkAttempted: true,
        completionState: "complete",
        requests: [
          {
            method: "GET",
            path: "/admin",
            statusCode: 200,
            verdict: "suspicious",
            queryBindings: [{ name: "tenant", value: "42" }],
            response: {
              contentTypeFamily: "json",
              declaredLength: 4096,
              capturedLength: 1024,
              prefixSha256: "a".repeat(64),
              truncated: true,
            },
          },
          {
            method: "HEAD",
            path: "/profile",
            errorClass: "request_timeout",
            statusCode: null,
          },
        ],
      },
    });
    expect(presentation.command).toBeNull();
    expect(presentation.commandProvenance).toBeNull();
    expect(presentation.stdout).toBeNull();
    expect(presentation.stderr).toBeNull();
  });

  it("keeps an empty HTTP execution visible and supports one historical JSON layer", () => {
    const presentation = presentToolActivity(
      tool({
        name: "vuln_probe_anonymous_access",
        result: JSON.stringify({
          selected_count: 0,
          network_attempted: false,
          completion_state: "complete",
          observations: [],
        }),
      })
    );

    expect(presentation.execution).toEqual({
      kind: "http",
      origin: null,
      selectedCount: 0,
      networkAttempted: false,
      completionState: "complete",
      requests: [],
    });
    expect(presentation.command).toBeNull();
  });

  it("ignores malformed HTTP observations and never recursively parses nested JSON", () => {
    const presentation = presentToolActivity(
      tool({
        name: "vuln_probe_anonymous_access",
        result: {
          exact_origin: "https://api.example.test:443",
          selected_count: "2",
          network_attempted: "true",
          observations: [
            null,
            "not-an-object",
            { endpoint_id: "missing-path", method: "GET" },
            {
              endpoint_id: "valid",
              method: "GET",
              path: "/health",
              status_code: "200",
              network_attempted: true,
              query_bindings: [{ name: "ok", value: "1" }, { name: "bad", value: 2 }],
              response: "{\"captured_length\":10}",
            },
          ],
          nested: JSON.stringify({ command: "curl must not be discovered" }),
        },
      })
    );

    expect(presentation.execution).toMatchObject({
      kind: "http",
      selectedCount: null,
      networkAttempted: null,
      requests: [
        {
          endpointId: "valid",
          method: "GET",
          path: "/health",
          statusCode: null,
          queryBindings: [{ name: "ok", value: "1" }],
          response: null,
        },
      ],
    });
    expect(presentation.command).toBeNull();
  });

  it("shows managed job context without fabricating a prelaunch EAS command", () => {
    const presentation = presentToolActivity(
      tool({
        result: {
          managed_job_id: "job_policy_blocked",
          wrapped_tool_name: "naabu",
          wrapped_args: "-top-ports 1000",
          status: "blocked",
        },
      })
    );

    expect(presentation.jobId).toBe("job_policy_blocked");
    expect(presentation.command).toBeNull();
    expect(presentation.commandProvenance).toBeNull();
  });
});

describe("summarizeToolActivities", () => {
  it("deduplicates actions in first-seen order and reflects live work", () => {
    expect(
      summarizeToolActivities([
        tool({ id: "ports-1", status: "completed" }),
        tool({ id: "ports-2", status: "completed" }),
        tool({ id: "http", name: "eas_probe_http_liveness", status: "backgrounded" }),
        tool({ id: "service", name: "eas_fingerprint_services", status: "completed" }),
      ])
    ).toBe("Scanned ports, checking web services, and 1 more activity");
  });
});
