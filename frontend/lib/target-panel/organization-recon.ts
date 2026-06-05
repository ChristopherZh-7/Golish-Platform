import type { OrganizationReconRunSnapshot } from "@/lib/api/organization-recon";
import type { Organization } from "@/lib/api/organizations";
import type { ReconArtifactRef } from "@/lib/generated/ReconArtifactRef";
import type { Target } from "@/lib/pentest/types";

type OrganizationReconTraceEvent = OrganizationReconRunSnapshot["traceEvents"][number];

export interface OrganizationReconLogGroup {
  event: OrganizationReconTraceEvent;
  details: OrganizationReconTraceEvent[];
  terminalEvent?: OrganizationReconTraceEvent;
}

export type OrganizationReconOperationTone =
  | "info"
  | "running"
  | "completed"
  | "warning"
  | "error"
  | "empty";

export interface OrganizationReconOperationDisplay {
  labelKey: string;
  fallbackLabel: string;
  statusKey: string;
  fallbackStatus: string;
  tone: OrganizationReconOperationTone;
}

const STAGE_TASK_IDS = new Set([
  "passive-internet",
  "active-collection",
  "processing",
  "persistence",
]);

export function applyOrganizationReconEvent(
  current: OrganizationReconRunSnapshot | undefined,
  incoming: OrganizationReconRunSnapshot
): OrganizationReconRunSnapshot {
  if (current && current.runId !== incoming.runId) return current;
  return incoming;
}

export function isOrganizationReconRunning(run: OrganizationReconRunSnapshot | undefined): boolean {
  return run?.status === "queued" || run?.status === "running";
}

export function findReconAssetsWorkbook(
  run: OrganizationReconRunSnapshot | undefined
): ReconArtifactRef | undefined {
  if (!run || isOrganizationReconRunning(run)) return undefined;
  const processingTask = run.tasks.find((task) => task.stage === "processing");
  if (
    !processingTask ||
    (processingTask.status !== "completed" && processingTask.status !== "checked_empty")
  ) {
    return undefined;
  }
  return processingTask.artifacts.find(
    (artifact) => artifact.kind === "asset_workbook" && artifact.path.endsWith("recon-assets.xlsx")
  );
}

export function suggestedReconAssetsFilename(orgName: string, runId?: string): string {
  const stem =
    orgName
      .trim()
      .replace(/[\\/:*?"<>|]/g, "_")
      .slice(0, 80) || "organization";
  const runSuffix = runId ? `-${runId.slice(0, 8)}` : "";
  return `${stem}-recon-assets${runSuffix}.xlsx`;
}

export function canExportCurrentReconAssets(hydrateStatus?: string): boolean {
  return hydrateStatus === "completed" || hydrateStatus === "partial";
}

export function displayOrganizationReconStatus(status: string): string {
  return status === "checked_empty" ? "completed" : status;
}

export function organizationReconProgress(run: OrganizationReconRunSnapshot | undefined): number {
  if (!run) return 0;
  if (run.status === "completed" || run.status === "partial" || run.status === "failed") {
    return 100;
  }
  if (run.stages.length === 0) return run.status === "running" ? 5 : 0;
  const terminalCount = run.stages.filter((stage) =>
    ["completed", "checked_empty", "failed", "skipped", "cancelled"].includes(stage.status)
  ).length;
  return Math.max(5, Math.round((terminalCount / run.stages.length) * 100));
}

export function currentOrganizationReconMessage(
  run: OrganizationReconRunSnapshot | undefined
): string | undefined {
  if (!run) return undefined;
  const latestDynamicTrace = [...run.traceEvents]
    .reverse()
    .find(
      (event) =>
        event.status === "running" &&
        event.taskId &&
        !STAGE_TASK_IDS.has(event.taskId) &&
        event.message.startsWith("active_tool_")
    );
  if (latestDynamicTrace?.taskId) {
    return `${latestDynamicTrace.taskId}: ${compactReconMessage(latestDynamicTrace.message)}`;
  }
  const runningTask = [...run.tasks]
    .reverse()
    .find((task) => task.status === "running" && !STAGE_TASK_IDS.has(task.taskId));
  if (runningTask) return runningTask.taskId;
  const stageRunningTask = [...run.tasks].reverse().find((task) => task.status === "running");
  if (stageRunningTask) return stageRunningTask.taskId;
  const latest = run.traceEvents[run.traceEvents.length - 1];
  return latest?.message;
}

export function recentOrganizationReconTraceEvents(
  run: OrganizationReconRunSnapshot | undefined,
  limit = 5
): NonNullable<OrganizationReconRunSnapshot["traceEvents"]> {
  return (run?.traceEvents ?? []).slice(-limit).reverse();
}

export function organizationReconLogGroups(
  events: OrganizationReconTraceEvent[]
): OrganizationReconLogGroup[] {
  const groups: OrganizationReconLogGroup[] = [];
  const latestByTask = new Map<string, OrganizationReconLogGroup>();
  const stepStartByTask = new Map<string, OrganizationReconLogGroup>();
  const installStartByTask = new Map<string, OrganizationReconLogGroup>();

  for (const event of events) {
    if (event.kind === "artifact_created") continue;
    const key = reconTraceTaskKey(event);
    if (isAutoInstallChildLog(event)) {
      appendReconLogDetail(event, installStartByTask.get(key) ?? latestByTask.get(key), groups);
      continue;
    }
    if (isPassiveProviderDetailLog(event)) {
      appendReconLogDetail(event, stepStartByTask.get(key) ?? latestByTask.get(key), groups);
      continue;
    }
    if (isLiveOutputDetailLog(event)) {
      appendReconLogDetail(event, latestByTask.get(key), groups);
      continue;
    }

    const terminalGroup = stepStartByTask.get(key) ?? latestByTask.get(key);
    if (isTerminalLogEvent(event) && terminalGroup) {
      terminalGroup.terminalEvent = event;
    }

    const group: OrganizationReconLogGroup = { event, details: [] };
    groups.push(group);
    latestByTask.set(key, group);
    if (event.kind === "step_started") {
      stepStartByTask.set(key, group);
    }
    if (event.kind === "step_log" && event.message.startsWith("active_tool_auto_install_start:")) {
      installStartByTask.set(key, group);
    }
  }

  return groups;
}

function appendReconLogDetail(
  event: OrganizationReconTraceEvent,
  group: OrganizationReconLogGroup | undefined,
  groups: OrganizationReconLogGroup[]
) {
  if (group) {
    group.details.push(event);
  } else {
    groups.push({ event, details: [] });
  }
}

function reconTraceTaskKey(event: OrganizationReconTraceEvent): string {
  return `${event.stage ?? "run"}:${event.taskId ?? "stage"}`;
}

function isAutoInstallChildLog(event: OrganizationReconTraceEvent): boolean {
  return (
    event.kind === "step_log" &&
    event.message.startsWith("active_tool_auto_install_") &&
    !event.message.startsWith("active_tool_auto_install_start:")
  );
}

function isPassiveProviderDetailLog(event: OrganizationReconTraceEvent): boolean {
  return event.kind === "step_log" && event.message.startsWith("passive_provider_");
}

function isLiveOutputDetailLog(event: OrganizationReconTraceEvent): boolean {
  return (
    event.kind === "step_log" &&
    (event.message.startsWith("active_tool_stdout:") ||
      event.message.startsWith("active_tool_stderr:") ||
      event.message.startsWith("active_tool_running:") ||
      event.message.startsWith("active_tool_stream_read_failed:"))
  );
}

export function organizationReconLogGroupIsRunning(group: OrganizationReconLogGroup): boolean {
  if (organizationReconLogGroupOperationDisplay(group).tone === "running") return true;
  return group.details.some(
    (detail) => organizationReconLogDetailOperationDisplay(detail, group).tone === "running"
  );
}

export function organizationReconLogGroupOperationDisplay(
  group: OrganizationReconLogGroup
): OrganizationReconOperationDisplay {
  const base = organizationReconOperationDisplay(group.event);
  const terminal = groupTerminalStatus(group);
  if (base.tone === "running" && terminal) {
    return withOperationStatus(base, terminal.status, terminal.level);
  }
  return base;
}

export function organizationReconLogDetailOperationDisplay(
  detail: OrganizationReconTraceEvent,
  group: OrganizationReconLogGroup
): OrganizationReconOperationDisplay {
  const base = organizationReconOperationDisplay(detail);
  const terminal = groupTerminalStatus(group);
  if (base.tone === "running" && terminal && isSameOperationLifecycle(detail, group.event)) {
    return withOperationStatus(base, terminal.status, terminal.level);
  }
  return base;
}

export function organizationReconOperationDisplay(
  event: OrganizationReconTraceEvent
): OrganizationReconOperationDisplay {
  const message = event.message;
  if (event.kind === "run_started") {
    return op("run", "运行", "running", "进行中", "running");
  }
  if (event.kind === "run_completed") {
    return op(
      "run",
      "运行",
      event.level === "error" ? "error" : "completed",
      event.level === "error" ? "出错" : "已完成",
      event.level === "error" ? "error" : "completed"
    );
  }
  if (event.kind === "step_started") {
    return op("stepStart", "步骤开始", "running", "进行中", "running");
  }
  if (event.kind === "step_completed") {
    return op(
      "stepComplete",
      "步骤结束",
      event.status === "failed" ? "error" : statusKeyFromStatus(event.status),
      fallbackStatusFromStatus(event.status),
      toneFromStatus(event.status, event.level)
    );
  }
  if (event.kind === "step_annotation") {
    return op(
      "annotation",
      "提示",
      event.level === "error" ? "error" : "info",
      event.level === "error" ? "出错" : "提示",
      event.level === "error" ? "error" : "info"
    );
  }

  if (message.startsWith("passive_provider_failed:")) {
    return op("request", "请求", "error", "出错", "error");
  }
  if (message.startsWith("passive_provider_finished:")) {
    return op("request", "请求", "completed", "已完成", "completed");
  }
  if (message.startsWith("passive_provider_")) {
    return op("request", "请求", "running", "进行中", "running");
  }

  if (message.startsWith("active_tool_auto_install_failed:")) {
    return op("install", "安装", "error", "出错", "error");
  }
  if (
    message.startsWith("active_tool_auto_install_ready:") ||
    message.startsWith("active_tool_auto_install_download:")
  ) {
    return op("install", "安装", "completed", "已完成", "completed");
  }
  if (message.startsWith("active_tool_auto_install_")) {
    return op("install", "安装", "running", "进行中", "running");
  }
  if (message.startsWith("active_tool_validation_failed:")) {
    return op("validation", "校验", "warning", "警告", "warning");
  }
  if (message.startsWith("active_tool_managed_executable_found:")) {
    return op("locate", "定位", "completed", "已完成", "completed");
  }
  if (
    message.startsWith("active_tool_config_missing:") ||
    message.startsWith("active_tool_install_unavailable:") ||
    message.startsWith("active_tool_spawn_failed:") ||
    message.startsWith("active_tool_wait_failed:") ||
    message.startsWith("active_tool_timeout:") ||
    message.startsWith("active_tool_nonzero_exit:") ||
    message.startsWith("active_tool_output_decode_failed:") ||
    message.startsWith("active_tool_output_parse_failed:")
  ) {
    return op("execute", "执行", "error", "出错", "error");
  }
  if (message.startsWith("active_tool_checked_empty:")) {
    return op("complete", "完成", "checkedEmpty", "空结果", "empty");
  }
  if (message.startsWith("active_tool_finished:")) {
    const status = activeFinishedStatus(message);
    return op(
      "complete",
      "完成",
      statusKeyFromStatus(status),
      fallbackStatusFromStatus(status),
      toneFromStatus(status, event.level)
    );
  }
  if (message.startsWith("active_tool_spawn:") || message.startsWith("active_tool_running:")) {
    return op("execute", "执行", "running", "进行中", "running");
  }
  if (
    message.startsWith("active_tool_stdout:") ||
    message.startsWith("active_tool_stderr:") ||
    message.startsWith("active_tool_stream_read_failed:")
  ) {
    return op(
      "output",
      "输出",
      event.level === "warning" ? "warning" : "running",
      event.level === "warning" ? "警告" : "进行中",
      event.level === "warning" ? "warning" : "running"
    );
  }

  return op(
    "log",
    "记录",
    statusKeyFromStatus(event.status),
    fallbackStatusFromStatus(event.status),
    toneFromStatus(event.status, event.level)
  );
}

function groupTerminalStatus(
  group: OrganizationReconLogGroup
): { status: string; level: string } | undefined {
  const terminalEvent =
    group.terminalEvent ??
    [...group.details].reverse().find((detail) => isTerminalLogEvent(detail));
  if (!terminalEvent) return undefined;
  const status = terminalStatusFromEvent(terminalEvent);
  if (!status) return undefined;
  return { status, level: terminalEvent.level };
}

function isSameOperationLifecycle(
  detail: OrganizationReconTraceEvent,
  groupEvent: OrganizationReconTraceEvent
): boolean {
  if (detail.message.startsWith("passive_provider_")) {
    return groupEvent.kind === "step_started" || groupEvent.message.startsWith("passive_provider_");
  }
  if (detail.message.startsWith("active_tool_")) {
    return groupEvent.message.startsWith("active_tool_");
  }
  return detail.kind === groupEvent.kind;
}

function withOperationStatus(
  base: OrganizationReconOperationDisplay,
  status: string,
  level: string
): OrganizationReconOperationDisplay {
  return {
    ...base,
    statusKey: `targetWorkspace.organizationRecon.operationStatus.${statusKeyFromStatus(status)}`,
    fallbackStatus: fallbackStatusFromStatus(status),
    tone: toneFromStatus(status, level),
  };
}

function isTerminalLogEvent(event: OrganizationReconTraceEvent): boolean {
  return terminalStatusFromEvent(event) !== undefined;
}

function terminalStatusFromEvent(event: OrganizationReconTraceEvent): string | undefined {
  if (event.kind === "run_completed" || event.kind === "step_completed") {
    return event.status ?? (event.level === "error" ? "failed" : "completed");
  }
  const message = event.message;
  if (message.startsWith("passive_provider_finished:")) return "completed";
  if (message.startsWith("passive_provider_failed:")) return "failed";
  if (message.startsWith("active_tool_checked_empty:")) return "checked_empty";
  if (message.startsWith("active_tool_finished:"))
    return activeFinishedStatus(message) ?? "completed";
  if (
    message.startsWith("active_tool_config_missing:") ||
    message.startsWith("active_tool_install_unavailable:") ||
    message.startsWith("active_tool_auto_install_failed:") ||
    message.startsWith("active_tool_spawn_failed:") ||
    message.startsWith("active_tool_wait_failed:") ||
    message.startsWith("active_tool_timeout:") ||
    message.startsWith("active_tool_nonzero_exit:") ||
    message.startsWith("active_tool_output_decode_failed:") ||
    message.startsWith("active_tool_output_parse_failed:")
  ) {
    return "failed";
  }
  if (
    event.status === "completed" ||
    event.status === "checked_empty" ||
    event.status === "failed" ||
    event.status === "skipped" ||
    event.status === "cancelled"
  ) {
    return event.status;
  }
  return undefined;
}

function op(
  label: string,
  fallbackLabel: string,
  status: string,
  fallbackStatus: string,
  tone: OrganizationReconOperationTone
): OrganizationReconOperationDisplay {
  return {
    labelKey: `targetWorkspace.organizationRecon.operations.${label}`,
    fallbackLabel,
    statusKey: `targetWorkspace.organizationRecon.operationStatus.${status}`,
    fallbackStatus,
    tone,
  };
}

function activeFinishedStatus(message: string): string | null {
  if (message.includes("status=Completed")) return "completed";
  if (message.includes("status=CheckedEmpty")) return "checked_empty";
  if (message.includes("status=Failed")) return "failed";
  return null;
}

function statusKeyFromStatus(status: string | null | undefined): string {
  if (status === "checked_empty") return "checkedEmpty";
  if (status === "failed") return "error";
  if (status === "completed") return "completed";
  if (status === "running") return "running";
  if (status === "skipped") return "skipped";
  if (status === "cancelled") return "cancelled";
  return "info";
}

function fallbackStatusFromStatus(status: string | null | undefined): string {
  if (status === "checked_empty") return "空结果";
  if (status === "failed") return "出错";
  if (status === "completed") return "已完成";
  if (status === "running") return "进行中";
  if (status === "skipped") return "已跳过";
  if (status === "cancelled") return "已取消";
  return "信息";
}

function toneFromStatus(
  status: string | null | undefined,
  level: string
): OrganizationReconOperationTone {
  if (level === "error" || status === "failed") return "error";
  if (status === "checked_empty") return "empty";
  if (level === "warning") return "warning";
  if (status === "running") return "running";
  if (status === "completed") return "completed";
  return "info";
}

function compactReconMessage(message: string): string {
  return message.replace(/^active_tool_/, "").replace(/: /, " ");
}

export function hasExportableCurrentReconAssets(
  organization: Organization,
  targets: Target[] = []
): boolean {
  if (targets.length > 0) return true;
  if (
    hasItems(organization.domains) ||
    hasItems(organization.ip_ranges) ||
    hasItems(organization.email_domains) ||
    hasItems(organization.certificates) ||
    hasItems(organization.business_systems) ||
    hasItems(organization.cloud_assets) ||
    hasItems(organization.github_orgs) ||
    hasItems(organization.social_accounts) ||
    hasItems(organization.historical_vulns) ||
    hasContactItems(organization.contacts)
  ) {
    return true;
  }

  const intel = organization.intel;
  if (!intel || typeof intel !== "object" || Array.isArray(intel)) return false;
  return [
    "mobile_apps",
    "mini_programs",
    "app_domains",
    "mail_mx",
    "leaks",
    "suppliers",
    "official_accounts",
    "quake_http_titles",
    "quake_http_servers",
    "quake_services",
  ].some((key) => hasItems((intel as Record<string, unknown>)[key]));
}

function hasItems(value: unknown): boolean {
  if (!Array.isArray(value)) return false;
  return value.some((item) => {
    if (typeof item === "string") return item.trim().length > 0;
    return item !== null && item !== undefined;
  });
}

function hasContactItems(value: unknown): boolean {
  if (Array.isArray(value)) return hasItems(value);
  if (!value || typeof value !== "object") return false;
  return Object.values(value as Record<string, unknown>).some((entry) =>
    Array.isArray(entry) ? hasItems(entry) : Boolean(entry)
  );
}

export function isOrganizationOwnedTarget(organization: Organization, value: string): boolean {
  const host = normalizedTargetHost(value);
  if (!host) return looksLikeIp(value);
  if (isKnownPublicNonAssetHost(host)) return false;
  const ownedDomains = collectOwnedDomains(organization);
  if (ownedDomains.size === 0) return false;
  return [...ownedDomains].some((domain) => host === domain || host.endsWith(`.${domain}`));
}

function collectOwnedDomains(organization: Organization): Set<string> {
  const domains = new Set<string>();
  for (const value of collectAtomValues(organization.domains)) {
    const host = normalizedTargetHost(value);
    if (host && !isKnownPublicNonAssetHost(host)) domains.add(host);
  }
  const appDomains = (organization.intel as Record<string, unknown> | undefined)?.app_domains;
  for (const value of collectAtomValues(appDomains)) {
    const host = normalizedTargetHost(value);
    if (host && !isKnownPublicNonAssetHost(host)) domains.add(host);
  }
  return domains;
}

function collectAtomValues(value: unknown): string[] {
  if (typeof value === "string") return value.trim() ? [value.trim()] : [];
  if (Array.isArray(value)) return value.flatMap(collectAtomValues);
  if (!value || typeof value !== "object") return [];
  const record = value as Record<string, unknown>;
  for (const key of ["domain", "url", "host", "value", "name"]) {
    const item = record[key];
    if (typeof item === "string" && item.trim()) return [item.trim()];
  }
  return Object.values(record).flatMap(collectAtomValues);
}

function normalizedTargetHost(value: string): string | null {
  const trimmed = value.trim().toLowerCase().replace(/\.$/, "");
  if (!trimmed || looksLikeIp(trimmed)) return null;
  try {
    const url = new URL(trimmed.includes("://") ? trimmed : `https://${trimmed}`);
    return (
      url.hostname
        .toLowerCase()
        .replace(/^www\./, "")
        .replace(/\.$/, "") || null
    );
  } catch {
    return null;
  }
}

function looksLikeIp(value: string): boolean {
  const trimmed = value.trim();
  return (
    /^\d{1,3}(?:\.\d{1,3}){3}$/.test(trimmed) || (!trimmed.includes("://") && trimmed.includes(":"))
  );
}

function isKnownPublicNonAssetHost(host: string): boolean {
  return [
    "github.com",
    "gitlab.com",
    "bitbucket.org",
    "gitee.com",
    "126.com",
    "163.com",
    "gmail.com",
    "hotmail.com",
    "outlook.com",
    "qq.com",
  ].some((domain) => host === domain || host.endsWith(`.${domain}`));
}
