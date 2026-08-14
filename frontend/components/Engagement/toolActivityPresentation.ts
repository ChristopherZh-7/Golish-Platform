import { getToolActionLabel, getToolPrimaryArg } from "@/lib/tools";
import type { SubAgentToolCall } from "@/store";

export type ToolCommandProvenance = "executed" | "requested";

export interface HttpQueryBindingPresentation {
  name: string;
  value: string;
}

export interface HttpResponsePresentation {
  contentTypeFamily: string | null;
  declaredLength: number | null;
  capturedLength: number | null;
  prefixSha256: string | null;
  truncated: boolean | null;
}

export interface HttpRequestPresentation {
  endpointId: string;
  method: string;
  path: string;
  queryBindings: HttpQueryBindingPresentation[];
  networkAttempted: boolean | null;
  statusCode: number | null;
  verdict: string | null;
  errorClass: string | null;
  response: HttpResponsePresentation | null;
}

export interface HttpExecutionPresentation {
  kind: "http";
  origin: string | null;
  selectedCount: number | null;
  networkAttempted: boolean | null;
  completionState: string | null;
  requests: HttpRequestPresentation[];
}

export interface ToolActivityPresentation {
  action: string;
  completedAction: string;
  runner: string | null;
  subject: string | null;
  command: string | null;
  commandProvenance: ToolCommandProvenance | null;
  stdout: string | null;
  stderr: string | null;
  hint: string | null;
  jobId: string | null;
  execution: HttpExecutionPresentation | null;
}

interface ActivityCopy {
  action: string;
  completedAction: string;
  runner: string | null;
}

const ACTIVITY_COPY: Record<string, ActivityCopy> = {
  eas_discover_ports: {
    action: "Scanning ports",
    completedAction: "Scanned ports",
    runner: "Naabu",
  },
  eas_probe_http_liveness: {
    action: "Checking web services",
    completedAction: "Checked web services",
    runner: "HTTPX",
  },
  eas_fingerprint_services: {
    action: "Probing services",
    completedAction: "Probed services",
    runner: "Nmap",
  },
  eas_fingerprint_web_stack: {
    action: "Fingerprinting web services",
    completedAction: "Fingerprinted web services",
    runner: "WhatWeb",
  },
  vuln_probe_anonymous_access: {
    action: "Probing anonymous access",
    completedAction: "Probed anonymous access",
    runner: "Golish HTTP client",
  },
  vuln_nuclei_general: {
    action: "Scanning with Nuclei",
    completedAction: "Scanned with Nuclei",
    runner: "Nuclei",
  },
  vuln_nuclei_fingerprint_targeted: {
    action: "Scanning fingerprints with Nuclei",
    completedAction: "Scanned fingerprints with Nuclei",
    runner: "Nuclei",
  },
};

const COMPLETED_ACTIONS: Record<string, string> = {
  run_command: "Ran shell command",
  run_pty_cmd: "Ran shell command",
  shell: "Ran shell command",
  wait_for_background_jobs: "Waited for background jobs",
  check_job: "Checked background job",
  kill_job: "Stopped background job",
  list_jobs: "Listed background jobs",
  read_file: "Read file",
  write_file: "Wrote file",
  edit_file: "Edited file",
  search_files: "Searched files",
  grep_file: "Searched files",
  web_search: "Searched the web",
  web_search_answer: "Searched the web",
  web_fetch: "Fetched URL",
  query_target_data: "Read target data",
};

function plainRecord(value: unknown): Record<string, unknown> | null {
  if (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    (Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null)
  ) {
    return value as Record<string, unknown>;
  }
  return null;
}

function normalizedRecord(value: unknown): Record<string, unknown> | null {
  const native = plainRecord(value);
  if (native) return native;
  if (typeof value !== "string") return null;
  try {
    const parsed: unknown = JSON.parse(value);
    return plainRecord(parsed);
  } catch {
    return null;
  }
}

function exactString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function exactBoolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

function exactNonNegativeInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function httpQueryBindings(value: unknown): HttpQueryBindingPresentation[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item) => {
    const record = plainRecord(item);
    const name = exactString(record?.name);
    const bindingValue = exactString(record?.value);
    return name && bindingValue ? [{ name, value: bindingValue }] : [];
  });
}

function httpResponse(value: unknown): HttpResponsePresentation | null {
  const response = plainRecord(value);
  if (!response) return null;
  return {
    contentTypeFamily: exactString(response.content_type_family),
    declaredLength: exactNonNegativeInteger(response.declared_length),
    capturedLength: exactNonNegativeInteger(response.captured_length),
    prefixSha256: exactString(response.prefix_sha256),
    truncated: exactBoolean(response.truncated),
  };
}

function httpRequest(value: unknown): HttpRequestPresentation | null {
  const request = plainRecord(value);
  const endpointId = exactString(request?.endpoint_id);
  const method = exactString(request?.method);
  const path = exactString(request?.path);
  if (!endpointId || !method || !path) return null;
  return {
    endpointId,
    method,
    path,
    queryBindings: httpQueryBindings(request?.query_bindings),
    networkAttempted: exactBoolean(request?.network_attempted),
    statusCode: exactNonNegativeInteger(request?.status_code),
    verdict: exactString(request?.verdict),
    errorClass: exactString(request?.error_class),
    response: httpResponse(request?.response),
  };
}

function httpExecution(
  tool: SubAgentToolCall,
  result: Record<string, unknown> | null
): HttpExecutionPresentation | null {
  if (tool.name !== "vuln_probe_anonymous_access") return null;
  const observations = Array.isArray(result?.observations) ? result.observations : [];
  return {
    kind: "http",
    origin: exactString(result?.exact_origin),
    selectedCount: exactNonNegativeInteger(result?.selected_count),
    networkAttempted: exactBoolean(result?.network_attempted),
    completionState: exactString(result?.completion_state),
    requests: observations.flatMap((observation) => {
      const request = httpRequest(observation);
      return request ? [request] : [];
    }),
  };
}

function activityCopy(tool: SubAgentToolCall): ActivityCopy {
  const exact = ACTIVITY_COPY[tool.name];
  if (exact) return exact;

  const action = getToolActionLabel(tool.name, tool.args);
  const completedAction =
    COMPLETED_ACTIONS[tool.name] ??
    (action.startsWith("Using ") ? `Used ${action.slice("Using ".length)}` : action);
  const runner =
    tool.name === "pentest_run" && typeof tool.args.tool_name === "string"
      ? tool.args.tool_name.trim() || null
      : null;
  return { action, completedAction, runner };
}

function targetSubject(tool: SubAgentToolCall): string | null {
  const targets = Array.isArray(tool.args.targets)
    ? tool.args.targets
    : Array.isArray(tool.args.target_urls)
      ? tool.args.target_urls
      : null;
  if (!targets) return getToolPrimaryArg(tool.name, tool.args);

  const count = targets.length;
  const parts = [`${count} ${count === 1 ? "target" : "targets"}`];
  if (typeof tool.args.scan_profile === "string" && tool.args.scan_profile.trim()) {
    parts.push(`${tool.args.scan_profile.trim()} profile`);
  }
  return parts.join(" · ");
}

function requestedShellCommand(tool: SubAgentToolCall): string | null {
  if (tool.name !== "run_command" && tool.name !== "run_pty_cmd" && tool.name !== "shell") {
    return null;
  }
  return exactString(tool.args.command);
}

export function presentToolActivity(tool: SubAgentToolCall): ToolActivityPresentation {
  const result = normalizedRecord(tool.result);
  const execution = httpExecution(tool, result);
  const runnerExecution = plainRecord(result?.runner_execution);
  const executedCommand = exactString(result?.command) ?? exactString(runnerExecution?.command);
  const requestedCommand = executedCommand ? null : requestedShellCommand(tool);
  const copy = activityCopy(tool);
  const streamingOutput = exactString(tool.streamingOutput);

  return {
    ...copy,
    subject: execution?.origin ?? targetSubject(tool),
    command: executedCommand ?? requestedCommand,
    commandProvenance: executedCommand ? "executed" : requestedCommand ? "requested" : null,
    stdout:
      streamingOutput ??
      exactString(result?.stdout) ??
      exactString(result?.partial_stdout) ??
      exactString(result?.output),
    stderr: streamingOutput
      ? null
      : (exactString(result?.stderr) ?? exactString(result?.partial_stderr)),
    hint: exactString(result?.hint),
    jobId:
      exactString(result?.job_id) ??
      exactString(result?.managed_job_id) ??
      tool.backgroundRun?.jobId ??
      null,
    execution,
  };
}

function actionForSummary(tool: SubAgentToolCall): string {
  const presentation = presentToolActivity(tool);
  return tool.status === "running" || tool.status === "backgrounded"
    ? presentation.action
    : presentation.completedAction;
}

function lowerInitial(value: string): string {
  return value ? `${value[0]?.toLowerCase() ?? ""}${value.slice(1)}` : value;
}

export function summarizeToolActivities(tools: readonly SubAgentToolCall[]): string {
  const actions: string[] = [];
  for (const tool of tools) {
    const action = actionForSummary(tool);
    if (!actions.includes(action)) actions.push(action);
  }

  if (actions.length === 0) return "Tool activity";
  if (actions.length === 1) return actions[0] ?? "Tool activity";
  const first = actions[0] ?? "Tool activity";
  const second = lowerInitial(actions[1] ?? "");
  if (actions.length === 2) return `${first}, ${second}`;
  return `${first}, ${second}, and ${actions.length - 2} more ${actions.length === 3 ? "activity" : "activities"}`;
}
