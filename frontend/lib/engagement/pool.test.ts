import { describe, expect, it } from "vitest";
import type { EngagementSnapshot, OrgTreeNode } from "@/lib/api/engagement";
import {
  buildAttackUnits,
  buildReconUnits,
  buildWorkerPrompt,
  classifyWorkerError,
  STAGE_SLICES,
  unitAlreadyCovered,
  workerTitle,
} from "./pool";

function node(
  id: string,
  name: string,
  status: OrgTreeNode["status"],
  children: OrgTreeNode[] = []
): OrgTreeNode {
  return {
    organizationId: id,
    name,
    parentId: null,
    ownershipPercent: null,
    inScope: true,
    status,
    weakness: null,
    children,
  };
}

function snapshot(tree: OrgTreeNode[]): EngagementSnapshot {
  return {
    projectPath: "/tmp/p",
    mode: "checklist",
    rootCount: tree.length,
    totalOrgs: tree.length,
    covered: 0,
    blocked: 0,
    failed: 0,
    tree,
  };
}

describe("engagement pool pure logic", () => {
  it("buildReconUnits creates one family unit per root", () => {
    const tree = [
      node("r1", "母A", "pending", [node("c1", "子A1", "pending")]),
      node("r2", "母B", "pending"),
    ];
    const units = buildReconUnits(tree);
    expect(units.map((u) => u.id)).toEqual(["recon:r1", "recon:r2"]);
    expect(units[0].kind).toBe("recon_family");
    expect(units[0].familyRootId).toBe("r1");
  });

  it("buildAttackUnits flattens one family (root + all descendants)", () => {
    const tree = [
      node("r1", "母A", "passed", [
        node("c1", "子A1", "pending", [node("g1", "孙A1a", "pending")]),
        node("c2", "子A2", "pending"),
      ]),
      node("r2", "母B", "pending"),
    ];
    const units = buildAttackUnits(tree, "r1");
    expect(units.map((u) => u.id)).toEqual(["attack:r1", "attack:c1", "attack:g1", "attack:c2"]);
    for (const u of units) {
      expect(u.kind).toBe("attack_org");
      expect(u.familyRootId).toBe("r1");
    }
    // Unknown family root → empty (defensive).
    expect(buildAttackUnits(tree, "nope")).toEqual([]);
  });

  it("stage slices match the phased worker granularity", () => {
    expect(STAGE_SLICES.recon_family).toEqual({ from: "target_intel", to: "enumeration" });
    expect(STAGE_SLICES.attack_org).toEqual({ from: "vuln_triage", to: "reporting" });
  });

  it("unitAlreadyCovered only treats passed/skipped as covered (I8 fail-closed)", () => {
    const tree = [
      node("r1", "母A", "passed", [node("c1", "子A1", "pending")]),
      node("r2", "母B", "blocked"),
    ];
    const snap = snapshot(tree);
    const [reconA, reconB] = buildReconUnits(tree);
    expect(unitAlreadyCovered(reconA, snap)).toBe(true);
    expect(unitAlreadyCovered(reconB, snap)).toBe(false);
    // Child lookup recurses into the tree.
    const attackChild = buildAttackUnits(tree, "r1")[1];
    expect(unitAlreadyCovered(attackChild, snap)).toBe(false);
    // Unit not present in the snapshot at all → NOT covered.
    expect(
      unitAlreadyCovered(
        { id: "attack:zzz", kind: "attack_org", orgId: "zzz", orgName: "?", familyRootId: "r1" },
        snap
      )
    ).toBe(false);
  });

  it("worker prompts pin the org id, slice and discipline", () => {
    const [recon] = buildReconUnits([node("r1", "默安科技", "pending")]);
    const reconPrompt = buildWorkerPrompt(recon, { includeSubsidiaries: true, thresholdPct: 51 });
    expect(reconPrompt).toContain("organization_id=r1");
    expect(reconPrompt).toContain("target_intel");
    expect(reconPrompt).toContain("enumeration");
    expect(reconPrompt).toContain("51%");

    const attack = buildAttackUnits([node("r1", "默安科技", "passed")], "r1")[0];
    const attackPrompt = buildWorkerPrompt(attack, { includeSubsidiaries: false, thresholdPct: 51 });
    expect(attackPrompt).toContain("vuln_triage");
    expect(attackPrompt).toContain("reporting");
    expect(attackPrompt).toContain("checked_empty");
  });

  it("worker titles distinguish recon and attack", () => {
    const [recon] = buildReconUnits([node("r1", "默安", "pending")]);
    expect(workerTitle(recon)).toContain("recon");
    const [attack] = buildAttackUnits([node("r1", "默安", "passed")], "r1");
    expect(workerTitle(attack)).toContain("attack");
  });

  it("classifyWorkerError mirrors scheduler::classify_run_error", () => {
    expect(classifyWorkerError("stage blocked: coverage incomplete")).toBe("blocked");
    expect(classifyWorkerError("network exploded")).toBe("failed");
  });
});
