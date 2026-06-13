import { describe, expect, it } from "vitest";
import type { OrgTreeNode } from "@/lib/api/engagement";
import type { WorkerUnit } from "@/lib/engagement/pool";
import type { ChatConversation } from "@/store/slices/conversation";
import {
  activePhaseFor,
  buildOverviewRows,
  deriveWorkerActivity,
  effectiveStatusFor,
  type PoolView,
  summarizeRows,
} from "./engagementOverview.utils";

function node(
  id: string,
  name: string,
  status: OrgTreeNode["status"],
  children: OrgTreeNode[] = [],
  parentId: string | null = null
): OrgTreeNode {
  return {
    organizationId: id,
    name,
    parentId,
    ownershipPercent: null,
    inScope: true,
    status,
    weakness: null,
    children,
  };
}

function unit(id: string, kind: WorkerUnit["kind"], orgId: string): WorkerUnit {
  return { id, kind, orgId, orgName: orgId, familyRootId: "r1" };
}

const emptyPool: PoolView = { running: {}, queue: [], outcomes: {} };

describe("engagement overview merge", () => {
  it("falls back to DB snapshot status when the pool is idle", () => {
    const root = node("r1", "母A", "passed");
    expect(effectiveStatusFor(root, emptyPool)).toBe("passed");
  });

  it("running worker overrides everything", () => {
    const root = node("r1", "母A", "pending");
    const pool: PoolView = {
      running: {
        "recon:r1": {
          unit: unit("recon:r1", "recon_family", "r1"),
          convId: "conv-1",
          startedAt: 1,
        },
      },
      queue: [],
      outcomes: { "recon:r1": { unitId: "recon:r1", status: "failed" } },
    };
    expect(effectiveStatusFor(root, pool)).toBe("running");
  });

  it("queued beats outcomes; attack outcome beats recon outcome", () => {
    const root = node("r1", "母A", "pending");
    const queuedPool: PoolView = {
      running: {},
      queue: [unit("attack:r1", "attack_org", "r1")],
      outcomes: { "recon:r1": { unitId: "recon:r1", status: "passed" } },
    };
    expect(effectiveStatusFor(root, queuedPool)).toBe("queued");

    const outcomePool: PoolView = {
      running: {},
      queue: [],
      outcomes: {
        "recon:r1": { unitId: "recon:r1", status: "passed" },
        "attack:r1": { unitId: "attack:r1", status: "blocked" },
      },
    };
    expect(effectiveStatusFor(root, outcomePool)).toBe("blocked");
  });

  it("child orgs only consider their attack unit (recon is family-level)", () => {
    const child = node("c1", "子A1", "pending", [], "r1");
    const pool: PoolView = {
      running: {},
      queue: [],
      outcomes: { "attack:c1": { unitId: "attack:c1", status: "passed" } },
    };
    expect(effectiveStatusFor(child, pool)).toBe("passed");
  });

  it("buildOverviewRows honours expansion and summarize counts statuses", () => {
    const tree = [
      node("r1", "母A", "passed", [node("c1", "子A1", "pending", [], "r1")]),
      node("r2", "母B", "blocked"),
    ];
    const collapsed = buildOverviewRows(tree, new Set(), emptyPool, () => null);
    expect(collapsed.map((r) => r.node.organizationId)).toEqual(["r1", "r2"]);

    const expanded = buildOverviewRows(tree, new Set(["r1"]), emptyPool, () => null);
    expect(expanded.map((r) => r.node.organizationId)).toEqual(["r1", "c1", "r2"]);
    expect(expanded[1].depth).toBe(1);

    const all = buildOverviewRows(tree, new Set(["r1"]), emptyPool, () => null);
    const summary = summarizeRows(all);
    expect(summary.totalOrgs).toBe(3);
    expect(summary.covered).toBe(1);
    expect(summary.blocked).toBe(1);
  });

  it("activePhaseFor reflects the driving unit's kind", () => {
    const root = node("r1", "母A", "pending");
    expect(activePhaseFor(root, emptyPool)).toBeUndefined();

    const reconRunning: PoolView = {
      running: {
        "recon:r1": { unit: unit("recon:r1", "recon_family", "r1"), convId: "c", startedAt: 1 },
      },
      queue: [],
      outcomes: {},
    };
    expect(activePhaseFor(root, reconRunning)).toBe("recon");

    const attackOutcome: PoolView = {
      running: {},
      queue: [],
      outcomes: {
        "recon:r1": { unitId: "recon:r1", status: "passed" },
        "attack:r1": { unitId: "attack:r1", status: "blocked" },
      },
    };
    expect(activePhaseFor(root, attackOutcome)).toBe("attack");

    const child = node("c1", "子A1", "pending", [], "r1");
    expect(activePhaseFor(child, emptyPool)).toBeUndefined();
  });

  it("deriveWorkerActivity reports tool count + live in-flight tool", () => {
    const baseConv = (partial: Partial<ChatConversation>): ChatConversation => ({
      id: "c",
      title: "t",
      messages: [],
      createdAt: 0,
      aiSessionId: "s",
      aiInitialized: true,
      isStreaming: false,
      ...partial,
    });

    expect(deriveWorkerActivity(undefined)).toEqual({ toolCount: 0 });

    // Idle worker (not streaming): tool count only, no activity line.
    const idle = baseConv({
      isStreaming: false,
      messages: [
        {
          id: "m1",
          role: "assistant",
          content: "",
          timestamp: 0,
          toolCalls: [
            { name: "subfinder_enum", args: "{}", success: true },
            { name: "dns_resolve", args: "{}", success: true },
          ],
        },
      ],
    });
    expect(deriveWorkerActivity(idle)).toEqual({ toolCount: 2 });

    // Live worker: in-flight tool (success undefined) drives the activity line.
    const live = baseConv({
      isStreaming: true,
      messages: [
        {
          id: "m1",
          role: "assistant",
          content: "",
          timestamp: 0,
          toolCalls: [
            { name: "subfinder_enum", args: "{}", success: true },
            { name: "recon_enrich_assets", args: JSON.stringify({ target: "pingan.com.cn" }) },
          ],
        },
      ],
    });
    expect(deriveWorkerActivity(live)).toEqual({
      activity: "recon_enrich_assets · pingan.com.cn",
      toolCount: 2,
    });
  });

  it("workerConvId picks the running conversation first", () => {
    const tree = [node("r1", "母A", "pending")];
    const pool: PoolView = {
      running: {
        "recon:r1": {
          unit: unit("recon:r1", "recon_family", "r1"),
          convId: "conv-live",
          startedAt: 1,
        },
      },
      queue: [],
      outcomes: {},
    };
    const rows = buildOverviewRows(tree, new Set(), pool, () => "conv-old");
    expect(rows[0].workerConvId).toBe("conv-live");
  });
});
