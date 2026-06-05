import { describe, expect, it } from "vitest";
import type { OrganizationReconRunSnapshot } from "@/lib/api/organization-recon";
import type { Organization } from "@/lib/api/organizations";
import type { Target } from "@/lib/pentest/types";
import {
  applyOrganizationReconEvent,
  canExportCurrentReconAssets,
  currentOrganizationReconMessage,
  displayOrganizationReconStatus,
  findReconAssetsWorkbook,
  hasExportableCurrentReconAssets,
  isOrganizationOwnedTarget,
  isOrganizationReconRunning,
  organizationReconLogDetailOperationDisplay,
  organizationReconLogGroupIsRunning,
  organizationReconLogGroupOperationDisplay,
  organizationReconLogGroups,
  organizationReconOperationDisplay,
  organizationReconProgress,
  recentOrganizationReconTraceEvents,
  suggestedReconAssetsFilename,
} from "./organization-recon";

function run(
  runId: string,
  status: OrganizationReconRunSnapshot["status"]
): OrganizationReconRunSnapshot {
  return {
    runId,
    organizationId: "org-1",
    projectPath: "/tmp/project",
    status,
    stages: [],
    tasks: [],
    errors: [],
    traceEvents: [],
    createdAt: 1,
    updatedAt: 1,
  } satisfies OrganizationReconRunSnapshot;
}

function organization(patch: Partial<Organization> = {}): Organization {
  return {
    id: "org-1",
    project_path: "/tmp/project",
    name: "中国平安",
    parent_id: null,
    description: "",
    owner: "",
    sort_order: 0,
    aliases: [],
    industry: "",
    tier: "",
    credit_code: "",
    domains: [],
    ip_ranges: [],
    asns: [],
    email_domains: [],
    scope_rules: {},
    intel: {},
    notes: "",
    certificates: [],
    subsidiaries: [],
    business_systems: [],
    cloud_assets: [],
    github_orgs: [],
    social_accounts: [],
    historical_vulns: [],
    contacts: [],
    created_at: 1,
    updated_at: 1,
    ...patch,
  };
}

describe("organization recon event isolation", () => {
  it("ignores a delayed event from an older run", () => {
    const current = run("new-run", "running");
    const delayed = run("old-run", "completed");

    expect(applyOrganizationReconEvent(current, delayed)).toBe(current);
  });

  it("accepts updates for the active run", () => {
    const current = run("same-run", "queued");
    const incoming = run("same-run", "running");

    expect(applyOrganizationReconEvent(current, incoming)).toBe(incoming);
    expect(isOrganizationReconRunning(incoming)).toBe(true);
  });

  it("keeps the authorized organization name in the suggested workbook filename", () => {
    expect(suggestedReconAssetsFilename("中国平安", "12345678-aaaa")).toBe(
      "中国平安-recon-assets-12345678.xlsx"
    );
  });

  it("allows current asset export after asset-intel completes without a staged run", () => {
    expect(canExportCurrentReconAssets("completed")).toBe(true);
    expect(canExportCurrentReconAssets("partial")).toBe(true);
    expect(canExportCurrentReconAssets("failed")).toBe(false);
    expect(canExportCurrentReconAssets()).toBe(false);
  });

  it("allows current export after reload when org profile keeps passive assets", () => {
    expect(
      hasExportableCurrentReconAssets(
        organization({
          domains: [{ domain: "pingan.com.cn" }],
          intel: { mobile_apps: ["平安金管家"] },
        })
      )
    ).toBe(true);
    expect(hasExportableCurrentReconAssets(organization())).toBe(false);
  });

  it("allows current export after reload when only Quake intel fields are present", () => {
    expect(
      hasExportableCurrentReconAssets(
        organization({
          intel: { quake_services: ["https"], quake_http_titles: ["中国平安"] },
        })
      )
    ).toBe(true);
  });

  it("allows current export when targets exist even without hydrate run state", () => {
    const targets = [{ id: "target-1", value: "pingan.com.cn" }] as Target[];

    expect(hasExportableCurrentReconAssets(organization(), targets)).toBe(true);
  });

  it("renders checked_empty as completed in the activity UI", () => {
    expect(displayOrganizationReconStatus("checked_empty")).toBe("completed");
    expect(displayOrganizationReconStatus("failed")).toBe("failed");
  });

  it("shows progress as soon as a staged recon run starts", () => {
    const snapshot = run("run-started", "running");
    snapshot.stages.push({
      stage: "passive_internet",
      status: "running",
      taskIds: ["passive-internet"],
    });

    expect(organizationReconProgress(snapshot)).toBe(5);
  });

  it("uses running task and recent trace events for live status text", () => {
    const snapshot = run("run-live", "running");
    snapshot.tasks.push({
      taskId: "passive-internet",
      stage: "passive_internet",
      sourceId: "passive-internet",
      status: "running",
      recordCount: 0,
      artifacts: [],
      errors: [],
    });
    snapshot.traceEvents.push(
      {
        id: "event-1",
        kind: "run_started",
        timestamp: 1,
        stage: null,
        taskId: null,
        status: "running",
        level: "info",
        message: "Organization Recon run started",
      },
      {
        id: "event-2",
        kind: "step_started",
        timestamp: 2,
        stage: "passive_internet",
        taskId: "passive-internet",
        status: "running",
        level: "info",
        message: "passive-internet started",
      }
    );

    expect(currentOrganizationReconMessage(snapshot)).toBe("passive-internet");
    expect(recentOrganizationReconTraceEvents(snapshot, 1)[0]?.id).toBe("event-2");
  });

  it("shows the concrete active child task as the current running operation", () => {
    const snapshot = run("run-active-current", "running");
    snapshot.tasks.push(
      {
        taskId: "active-collection",
        stage: "active_collection",
        sourceId: "active-collection",
        status: "running",
        recordCount: 0,
        artifacts: [],
        errors: [],
      },
      {
        taskId: "amass-www.example.com",
        stage: "active_collection",
        sourceId: "amass",
        status: "running",
        recordCount: 0,
        artifacts: [],
        errors: [],
      }
    );
    snapshot.traceEvents.push({
      id: "event-1",
      kind: "step_log",
      timestamp: 1,
      stage: "active_collection",
      taskId: "amass-www.example.com",
      status: "running",
      level: "info",
      message: "active_tool_running: tool=amass seed=www.example.com elapsed=600s timeout=1800s",
    });

    expect(currentOrganizationReconMessage(snapshot)).toContain("amass-www.example.com");
    expect(currentOrganizationReconMessage(snapshot)).toContain("elapsed=600s");
  });

  it("groups active tool install and runtime output under expandable log rows", () => {
    const snapshot = run("run-active-logs", "running");
    snapshot.traceEvents.push(
      {
        id: "event-1",
        kind: "step_started",
        timestamp: 1,
        stage: "active_collection",
        taskId: "active-collection",
        status: "running",
        level: "info",
        message: "Step active-collection started",
      },
      {
        id: "event-2",
        kind: "step_log",
        timestamp: 2,
        stage: "active_collection",
        taskId: "subfinder-example.com",
        status: null,
        level: "info",
        message: "active_tool_auto_install_start: tool=subfinder method=github",
      },
      {
        id: "event-3",
        kind: "step_log",
        timestamp: 3,
        stage: "active_collection",
        taskId: "subfinder-example.com",
        status: null,
        level: "info",
        message: "active_tool_auto_install_log: Cloning subfinder",
      },
      {
        id: "event-4",
        kind: "step_log",
        timestamp: 4,
        stage: "active_collection",
        taskId: "subfinder-example.com",
        status: null,
        level: "info",
        message: "active_tool_spawn: tool=subfinder seed=example.com",
      },
      {
        id: "event-5",
        kind: "step_log",
        timestamp: 5,
        stage: "active_collection",
        taskId: "subfinder-example.com",
        status: "running",
        level: "info",
        message: "active_tool_running: tool=subfinder seed=example.com elapsed=10s timeout=900s",
      },
      {
        id: "event-6",
        kind: "step_log",
        timestamp: 6,
        stage: "active_collection",
        taskId: "subfinder-example.com",
        status: "running",
        level: "info",
        message: "active_tool_stdout: www.example.com",
      },
      {
        id: "event-7",
        kind: "artifact_created",
        timestamp: 7,
        stage: "active_collection",
        taskId: "subfinder-example.com",
        status: "completed",
        level: "info",
        message: "Artifact created: stdout",
      }
    );

    const groups = organizationReconLogGroups(snapshot.traceEvents);

    expect(groups.map((group) => group.event.id)).toEqual(["event-1", "event-2", "event-4"]);
    expect(groups[1]?.details.map((event) => event.message)).toEqual([
      "active_tool_auto_install_log: Cloning subfinder",
    ]);
    expect(groups[2]?.details.map((event) => event.message)).toEqual([
      "active_tool_running: tool=subfinder seed=example.com elapsed=10s timeout=900s",
      "active_tool_stdout: www.example.com",
    ]);
    expect(organizationReconLogGroupIsRunning(groups[2])).toBe(true);
    expect(organizationReconOperationDisplay(groups[2]!.event)).toMatchObject({
      fallbackLabel: "执行",
      fallbackStatus: "进行中",
      tone: "running",
    });
    expect(organizationReconOperationDisplay(groups[2]!.details[0]!)).toMatchObject({
      fallbackLabel: "执行",
      fallbackStatus: "进行中",
      tone: "running",
    });
  });

  it("groups passive provider details under the step start log", () => {
    const snapshot = run("run-passive-logs", "running");
    snapshot.traceEvents.push(
      {
        id: "event-1",
        kind: "step_started",
        timestamp: 1,
        stage: "passive_internet",
        taskId: "passive-internet",
        status: "running",
        level: "info",
        message: "Step passive-internet started",
      },
      {
        id: "event-2",
        kind: "step_log",
        timestamp: 2,
        stage: "passive_internet",
        taskId: "passive-internet",
        status: "running",
        level: "info",
        message: "passive_provider_plan: selected 2 provider(s): 0.zone, quake",
      },
      {
        id: "event-3",
        kind: "step_completed",
        timestamp: 3,
        stage: "passive_internet",
        taskId: "passive-internet",
        status: "completed",
        level: "info",
        message: "Step passive-internet finished as Completed with 4 record(s)",
      }
    );

    const groups = organizationReconLogGroups(snapshot.traceEvents);

    expect(groups.map((group) => group.event.id)).toEqual(["event-1", "event-3"]);
    expect(groups[0]?.details[0]?.message).toContain("passive_provider_plan");
  });

  it("marks historical passive provider details completed after the stage finishes", () => {
    const snapshot = run("run-passive-completed-logs", "completed");
    snapshot.traceEvents.push(
      {
        id: "event-1",
        kind: "step_started",
        timestamp: 1,
        stage: "passive_internet",
        taskId: "passive-internet",
        status: "running",
        level: "info",
        message: "Step passive-internet started",
      },
      {
        id: "event-2",
        kind: "step_log",
        timestamp: 2,
        stage: "passive_internet",
        taskId: "passive-internet",
        status: "running",
        level: "info",
        message: "passive_provider_plan: selected 2 provider(s): 0.zone, quake",
      },
      {
        id: "event-3",
        kind: "step_log",
        timestamp: 3,
        stage: "passive_internet",
        taskId: "passive-internet",
        status: "running",
        level: "info",
        message: "passive_provider_run: company=深圳市比特梵德科技有限公司",
      },
      {
        id: "event-4",
        kind: "step_log",
        timestamp: 4,
        stage: "passive_internet",
        taskId: "passive-internet",
        status: "completed",
        level: "info",
        message: "passive_provider_finished: status=Completed organizations=0 targets=1",
      },
      {
        id: "event-5",
        kind: "step_completed",
        timestamp: 5,
        stage: "passive_internet",
        taskId: "passive-internet",
        status: "completed",
        level: "info",
        message: "Step passive-internet finished as Completed with 1 record(s)",
      }
    );

    const groups = organizationReconLogGroups(snapshot.traceEvents);

    expect(organizationReconLogGroupOperationDisplay(groups[0]!)).toMatchObject({
      fallbackStatus: "已完成",
      tone: "completed",
    });
    expect(organizationReconLogDetailOperationDisplay(groups[0]!.details[0]!, groups[0]!)).toMatchObject({
      fallbackLabel: "请求",
      fallbackStatus: "已完成",
      tone: "completed",
    });
    expect(organizationReconLogDetailOperationDisplay(groups[0]!.details[1]!, groups[0]!)).toMatchObject({
      fallbackLabel: "请求",
      fallbackStatus: "已完成",
      tone: "completed",
    });
    expect(organizationReconLogGroupIsRunning(groups[0]!)).toBe(false);
  });

  it("keeps promoted assets inside the organization-owned domain boundary", () => {
    const org = organization({
      domains: [{ domain: "pingan.com.cn" }],
      email_domains: ["126.com"],
      intel: { app_domains: ["app.pingan.com.cn"] },
    });

    expect(isOrganizationOwnedTarget(org, "https://www.pingan.com.cn/")).toBe(true);
    expect(isOrganizationOwnedTarget(org, "app.pingan.com.cn")).toBe(true);
    expect(isOrganizationOwnedTarget(org, "https://github.com/example/leak/blob/key")).toBe(false);
    expect(isOrganizationOwnedTarget(org, "126.com")).toBe(false);
  });
});

describe("organization recon asset export", () => {
  it("finds the Stage 4 workbook after processing completes", () => {
    const snapshot = run("run-with-xlsx", "completed");
    snapshot.tasks.push({
      taskId: "processing",
      stage: "processing",
      sourceId: "processing",
      status: "completed",
      recordCount: 3,
      artifacts: [
        {
          path: "/tmp/.golish/tool-output/recon/run/processing/processing/exports/recon-assets.xlsx",
          kind: "asset_workbook",
          bytes: 1024n,
        },
      ],
      errors: [],
    });

    expect(findReconAssetsWorkbook(snapshot)?.bytes).toBe(1024n);
  });

  it("does not expose export before Stage 4 finishes", () => {
    const snapshot = run("run-still-processing", "running");
    snapshot.tasks.push({
      taskId: "processing",
      stage: "processing",
      sourceId: "processing",
      status: "running",
      recordCount: 0,
      artifacts: [
        {
          path: "/tmp/recon-assets.xlsx",
          kind: "asset_workbook",
          bytes: 1n,
        },
      ],
      errors: [],
    });

    expect(findReconAssetsWorkbook(snapshot)).toBeUndefined();
  });
});
