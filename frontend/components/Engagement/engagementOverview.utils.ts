/**
 * Pure helpers behind {@link EngagementOverview} (Phase C, 设计 2026-06-13
 * §10 运行时态 vs DB 真值): merge the live worker-pool state onto the DB-truth
 * snapshot so the overview shows running/queued workers on top of the
 * persisted coverage baseline, and survives reloads by degrading gracefully
 * back to DB truth alone.
 */

import type { OrgRunStatusDto, OrgTreeNode } from "@/lib/api/engagement";
import type { WorkerOutcome, WorkerUnit } from "@/lib/engagement/pool";
import type { ChatConversation, ChatToolCall } from "@/store/slices/conversation";
import type { RunningWorker } from "@/store/slices/engagement-pool";

/** Pool-aware display status (superset of the DB snapshot's status). */
export type EffectiveStatus = OrgRunStatusDto | "running" | "queued";

export interface PoolView {
  running: Record<string, RunningWorker>;
  queue: WorkerUnit[];
  outcomes: Record<string, WorkerOutcome>;
}

export interface OverviewRow {
  node: OrgTreeNode;
  depth: number;
  effective: EffectiveStatus;
  /** Worker conversation to drill into (most relevant active one). */
  workerConvId: string | null;
}

/** The two unit ids that can affect one org's row. */
function unitIdsForOrg(node: OrgTreeNode): string[] {
  const ids = [`attack:${node.organizationId}`];
  if (node.parentId == null) ids.unshift(`recon:${node.organizationId}`);
  return ids;
}

/** Map a worker outcome onto the display status vocabulary. */
function outcomeToStatus(outcome: WorkerOutcome): EffectiveStatus {
  switch (outcome.status) {
    case "passed":
      return "passed";
    case "skipped":
      return "skippedAlreadyComplete";
    case "blocked":
      return "blocked";
    case "failed":
      return "failed";
  }
}

/**
 * Effective status for one org row: a live worker wins over a queued one,
 * which wins over recorded outcomes (attack outcome preferred over recon —
 * it's the later phase), which win over the DB snapshot status.
 */
export function effectiveStatusFor(node: OrgTreeNode, pool: PoolView): EffectiveStatus {
  const ids = unitIdsForOrg(node);
  for (const id of ids) {
    if (pool.running[id]) return "running";
  }
  if (pool.queue.some((u) => ids.includes(u.id))) return "queued";
  // Attack outcome (ids[last]) describes the org's furthest progress; check it
  // before the family recon outcome.
  for (const id of [...ids].reverse()) {
    const outcome = pool.outcomes[id];
    if (outcome) return outcomeToStatus(outcome);
  }
  return node.status;
}

/** Conversation id of the most relevant worker for this row (running first). */
export function workerConvFor(
  node: OrgTreeNode,
  pool: PoolView,
  findConvByUnit: (unitId: string) => string | null
): string | null {
  const ids = unitIdsForOrg(node);
  for (const id of ids) {
    const running = pool.running[id];
    if (running) return running.convId;
  }
  for (const id of [...ids].reverse()) {
    const conv = findConvByUnit(id);
    if (conv) return conv;
  }
  return null;
}

/**
 * Which worker phase currently drives this org's row (recon vs attack), if a
 * worker exists for it. Mirrors {@link effectiveStatusFor}'s priority (live →
 * queued → outcome, attack preferred over recon) so the phase chip matches the
 * status badge. Returns undefined for orgs the pool hasn't touched yet.
 */
export function activePhaseFor(node: OrgTreeNode, pool: PoolView): "recon" | "attack" | undefined {
  const ids = unitIdsForOrg(node);
  for (const id of ids) {
    if (pool.running[id] || pool.queue.some((u) => u.id === id)) {
      return id.startsWith("recon:") ? "recon" : "attack";
    }
  }
  for (const id of [...ids].reverse()) {
    if (pool.outcomes[id]) return id.startsWith("recon:") ? "recon" : "attack";
  }
  return undefined;
}

/**
 * Flatten the org forest into display rows (pre-order, honouring `expanded`)
 * with pool-merged effective statuses.
 */
export function buildOverviewRows(
  tree: OrgTreeNode[],
  expanded: Set<string>,
  pool: PoolView,
  findConvByUnit: (unitId: string) => string | null
): OverviewRow[] {
  const out: OverviewRow[] = [];
  const walk = (nodes: OrgTreeNode[], depth: number) => {
    for (const node of nodes) {
      out.push({
        node,
        depth,
        effective: effectiveStatusFor(node, pool),
        workerConvId: workerConvFor(node, pool, findConvByUnit),
      });
      if (expanded.has(node.organizationId) && node.children.length > 0) {
        walk(node.children, depth + 1);
      }
    }
  };
  walk(tree, 0);
  return out;
}

export interface OverviewSummary {
  totalOrgs: number;
  covered: number;
  active: number;
  queued: number;
  blocked: number;
  failed: number;
}

/** Aggregate counters for the overview header (pool-merged). */
export function summarizeRows(allRows: OverviewRow[]): OverviewSummary {
  const s: OverviewSummary = {
    totalOrgs: allRows.length,
    covered: 0,
    active: 0,
    queued: 0,
    blocked: 0,
    failed: 0,
  };
  for (const r of allRows) {
    switch (r.effective) {
      case "passed":
      case "skippedAlreadyComplete":
        s.covered += 1;
        break;
      case "running":
        s.active += 1;
        break;
      case "queued":
        s.queued += 1;
        break;
      case "blocked":
        s.blocked += 1;
        break;
      case "failed":
        s.failed += 1;
        break;
      default:
        break;
    }
  }
  return s;
}

/** Live one-line activity + tool count derived from a worker conversation. */
export interface WorkerActivity {
  /** What the worker is doing right now (only while it streams a turn). */
  activity?: string;
  /** Tool calls booked across the worker's conversation so far. */
  toolCount: number;
}

const ACTIVITY_ARG_KEYS = [
  "command",
  "target",
  "domain",
  "host",
  "url",
  "path",
  "file_path",
  "pattern",
  "query",
  "name",
] as const;

/** Pull the most telling argument out of a (JSON-string) tool-call args blob. */
function primaryToolArg(argsJson: string): string | undefined {
  if (!argsJson) return undefined;
  try {
    const parsed = JSON.parse(argsJson) as Record<string, unknown>;
    for (const key of ACTIVITY_ARG_KEYS) {
      const v = parsed[key];
      if (typeof v === "string" && v.trim()) return v.trim();
    }
  } catch {
    // args may be a partial JSON fragment mid-stream — no detail yet.
  }
  return undefined;
}

/** Reduce streaming assistant text to a single trailing line. */
function lastLine(text: string): string {
  const cleaned = text.replace(/<[^>]*>/g, " ");
  const lines = cleaned
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  return (lines[lines.length - 1] ?? cleaned).replace(/\s+/g, " ").trim();
}

/**
 * Derive a worker's live activity line + tool count from its conversation —
 * the engagement-level analogue of SubAgentInlineCard's `deriveActivity`. The
 * activity is only meaningful while the worker is mid-turn (`isStreaming`);
 * the tool count is always the running total. Priority for the live line:
 * in-flight tool → last finished tool → thinking → streaming text → generic.
 */
export function deriveWorkerActivity(conv: ChatConversation | undefined): WorkerActivity {
  if (!conv) return { toolCount: 0 };

  let toolCount = 0;
  let lastRunningTool: ChatToolCall | undefined;
  let lastTool: ChatToolCall | undefined;
  for (const m of conv.messages) {
    if (!m.toolCalls) continue;
    for (const tc of m.toolCalls) {
      toolCount += 1;
      lastTool = tc;
      // `success === undefined` == still in flight (mirrors ChatToolCall).
      if (tc.success === undefined) lastRunningTool = tc;
    }
  }

  if (!conv.isStreaming) return { toolCount };

  const tc = lastRunningTool ?? lastTool;
  if (tc) {
    const arg = primaryToolArg(tc.args);
    return { activity: arg ? `${tc.name} · ${arg}` : tc.name, toolCount };
  }

  const last = conv.messages[conv.messages.length - 1];
  if (last?.role === "assistant") {
    if (last.thinkingStartedAt != null && last.thinkingEndedAt == null && !last.content) {
      return { activity: "思考中", toolCount };
    }
    const snippet = last.content ? lastLine(last.content) : "";
    if (snippet) return { activity: snippet, toolCount };
  }
  return { activity: "工作中", toolCount };
}
