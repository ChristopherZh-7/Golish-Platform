/**
 * Engagement worker-pool pure logic (设计 2026-06-13-engagement-scoping-fanout
 * §5/§6.1, Phase B).
 *
 * Unit construction, stage slices, worker prompts and spawn-eligibility
 * decisions — all pure functions so the scheduling semantics are fully unit
 * testable without Tauri/Zustand. The side-effectful pieces (spawning real
 * conversations, awaiting prompts) live in `spawn.ts` / `runPool.ts`.
 *
 * Stage-phased worker granularity (the core idea of the redesign):
 * - recon family worker  = one session per ROOT org, subsidiaries included
 *   (`target_intel..=enumeration`, Phase 3 母先收 → 子逐个收 keeps A/B 关联).
 * - attack org worker    = one session per org (`vuln_triage..=reporting`),
 *   enqueued only after its family's recon worker passed.
 */

import type { EngagementSnapshot, OrgTreeNode } from "@/lib/api/engagement";

export type WorkerUnitKind = "recon_family" | "attack_org";

export interface WorkerUnit {
  /** Stable unit id: `recon:<rootOrgId>` / `attack:<orgId>`. */
  id: string;
  kind: WorkerUnitKind;
  /** The org this worker binds to (family root for recon, the org for attack). */
  orgId: string;
  orgName: string;
  /** Family grouping key (the root org id) for overview grouping + fan-out. */
  familyRootId: string;
}

/** Per-kind stage slices (red_team DAG, 设计 §5). */
export const STAGE_SLICES: Record<WorkerUnitKind, { from: string; to: string }> = {
  recon_family: { from: "target_intel", to: "enumeration" },
  attack_org: { from: "vuln_triage", to: "reporting" },
};

/** Worker terminal outcome (mirrors scheduler::OrgRunStatus + skip). */
export type WorkerOutcomeStatus = "passed" | "blocked" | "failed" | "skipped";

export interface WorkerOutcome {
  unitId: string;
  status: WorkerOutcomeStatus;
  detail?: string;
}

/** One recon family unit per ROOT org in the snapshot tree. */
export function buildReconUnits(tree: OrgTreeNode[]): WorkerUnit[] {
  return tree.map((root) => ({
    id: `recon:${root.organizationId}`,
    kind: "recon_family" as const,
    orgId: root.organizationId,
    orgName: root.name,
    familyRootId: root.organizationId,
  }));
}

/** Flatten one family subtree (root + all descendants) into attack units. */
export function buildAttackUnits(tree: OrgTreeNode[], familyRootId: string): WorkerUnit[] {
  const root = tree.find((r) => r.organizationId === familyRootId);
  if (!root) return [];
  const out: WorkerUnit[] = [];
  const walk = (node: OrgTreeNode) => {
    out.push({
      id: `attack:${node.organizationId}`,
      kind: "attack_org",
      orgId: node.organizationId,
      orgName: node.name,
      familyRootId,
    });
    for (const child of node.children) walk(child);
  };
  walk(root);
  return out;
}

/**
 * Whether a unit's work is already covered by DB truth (resume-skip, I8
 * fail-closed: anything not positively "passed" in the snapshot is NOT
 * skipped — better to re-run than to wrongly skip). The snapshot must have
 * been fetched with `toStage` = the unit's slice end.
 */
export function unitAlreadyCovered(unit: WorkerUnit, snapshot: EngagementSnapshot): boolean {
  const findNode = (nodes: OrgTreeNode[]): OrgTreeNode | null => {
    for (const n of nodes) {
      if (n.organizationId === unit.orgId) return n;
      const hit = findNode(n.children);
      if (hit) return hit;
    }
    return null;
  };
  const node = findNode(snapshot.tree);
  return node != null && (node.status === "passed" || node.status === "skippedAlreadyComplete");
}

/**
 * Build the seed task prompt for a worker session. The hard constraints (org
 * axis, stage slice) are enforced by the pinned worker scope on the backend;
 * the prompt aligns the model with them so it doesn't waste turns fighting
 * the rails.
 */
export function buildWorkerPrompt(
  unit: WorkerUnit,
  opts: { includeSubsidiaries: boolean; thresholdPct: number }
): string {
  const slice = STAGE_SLICES[unit.kind];
  if (unit.kind === "recon_family") {
    const subs = opts.includeSubsidiaries
      ? `本家族含子公司（持股 ≥${opts.thresholdPct}% 的已入库子公司会被一并收集：母公司先收，子公司逐个收）。`
      : "本次只收集该公司自身（不含子公司）。";
    return (
      `对组织「${unit.orgName}」(organization_id=${unit.orgId}) 执行信息收集：` +
      `从 ${slice.from} 阶段做到 ${slice.to} 阶段（含），过每个阶段的 gate 后停止。` +
      `${subs}` +
      `纪律：只收集这个组织（及其入库子公司）的资产，不要碰其它组织；` +
      `严格按证据账本工作，查过为空就如实标 checked_empty，绝不编造。`
    );
  }
  return (
    `对组织「${unit.orgName}」(organization_id=${unit.orgId}) 执行漏洞研判到报告：` +
    `从 ${slice.from} 阶段做到 ${slice.to} 阶段（含），过最后的报告 gate 后停止。` +
    `信息收集阶段的资产/端点/指纹数据已在数据库中（按该组织隔离），直接据此工作。` +
    `纪律：只打这个组织名下 in-scope 的资产；每个发现都要有可追溯证据；` +
    `查过为空就如实标 checked_empty，绝不编造。`
  );
}

/** Conversation title for a worker session (tab + overview display). */
export function workerTitle(unit: WorkerUnit): string {
  return unit.kind === "recon_family"
    ? `🔎 ${unit.orgName} · recon`
    : `🎯 ${unit.orgName} · attack`;
}

/**
 * Classify a worker failure (mirrors `scheduler::classify_run_error`): an
 * error message mentioning a blocked stage is a BLOCK terminal, anything else
 * is a hard failure.
 */
export function classifyWorkerError(message: string): "blocked" | "failed" {
  return message.toLowerCase().includes("block") ? "blocked" : "failed";
}
