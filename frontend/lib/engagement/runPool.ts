/**
 * Engagement fan-out pool loop (设计 2026-06-13-engagement-scoping-fanout
 * §6.1, Phase B).
 *
 * Drives the whole engagement after scoping locked the range:
 *  - seeds the queue with recon FAMILY units (one per root org),
 *  - keeps at most K workers active (spawn → await → release slot),
 *  - resume-skips units whose DB truth already covers their slice end,
 *  - on a recon family pass, fans that family out into per-org ATTACK units,
 *  - isolates failures (one worker's BLOCK/FAIL never stops siblings),
 *  - honours a graceful stop request (in-flight workers finish, queue freezes).
 *
 * The loop is a long-lived promise living at module scope (not inside a
 * component); page reload drops runtime state but DB truth survives, so a
 * restarted pool resume-skips covered units (spec §10).
 */

import { getEngagementSnapshot } from "@/lib/api/engagement";
import { logger } from "@/lib/logger";
import { useStore } from "@/store";
import {
  buildAttackUnits,
  buildReconUnits,
  STAGE_SLICES,
  unitAlreadyCovered,
  type WorkerUnit,
} from "./pool";
import { type SpawnWorkerOptions, spawnWorkerAndRun } from "./spawn";

export interface StartEngagementRunArgs extends SpawnWorkerOptions {
  projectPath: string;
  concurrency: number;
  /** Conversation acting as the overview (the scoping chat), for drill-back. */
  overviewConvId: string | null;
}

let poolActive = false;

/** Whether the module-scope pool loop is currently driving a run. */
export function isPoolRunning(): boolean {
  return poolActive;
}

/**
 * Start the fan-out run. Resolves when the pool drains (or a stop request
 * lets the in-flight workers finish). Safe to call only when no run is
 * active — a second concurrent call is rejected.
 */
export async function startEngagementRun(args: StartEngagementRunArgs): Promise<void> {
  if (poolActive) {
    throw new Error("an engagement run is already active");
  }
  poolActive = true;
  const store = useStore.getState();
  store.poolConfigure({
    concurrency: args.concurrency,
    projectPath: args.projectPath,
    overviewConvId: args.overviewConvId,
  });
  store.poolSetPhase("running");

  try {
    // Seed: one recon family unit per root org in the locked scope.
    const reconSnapshot = await getEngagementSnapshot({
      projectPath: args.projectPath,
      toStage: STAGE_SLICES.recon_family.to,
    });
    const reconUnits = buildReconUnits(reconSnapshot.tree);
    if (reconUnits.length === 0) {
      logger.warn("[engagement] no root orgs in scope — did scoping run?");
      useStore.getState().poolSetPhase("complete");
      return;
    }
    useStore.getState().poolEnqueue(reconUnits);

    // Resume-skip recon units already covered by DB truth.
    await skipCoveredQueuedUnits(args.projectPath);

    const inflight = new Map<string, Promise<void>>();

    const launch = (unit: WorkerUnit) => {
      const task = (async () => {
        // spawnWorkerAndRun adds the conversation synchronously (before its
        // first await), so the conv id is resolvable right after the call.
        const outcomePromise = spawnWorkerAndRun(unit, args);
        const convId = findConvForUnit(unit.id) ?? `pending-${unit.id}`;
        useStore.getState().poolMarkRunning(unit, convId);
        const outcome = await outcomePromise;
        useStore.getState().poolMarkOutcome(outcome);

        // Family recon passed → fan out that family's per-org attack units.
        if (unit.kind === "recon_family" && outcome.status === "passed") {
          const attackSnapshot = await getEngagementSnapshot({
            projectPath: args.projectPath,
            toStage: STAGE_SLICES.recon_family.to,
          });
          const attackUnits = buildAttackUnits(attackSnapshot.tree, unit.familyRootId);
          useStore.getState().poolEnqueue(attackUnits);
        }
      })().catch((err) => {
        // spawnWorkerAndRun never throws by contract; this is a belt-and-braces
        // guard so an unexpected error still releases the slot.
        logger.error(`[engagement] worker ${unit.id} crashed unexpectedly:`, err);
        useStore.getState().poolMarkOutcome({
          unitId: unit.id,
          status: "failed",
          detail: err instanceof Error ? err.message : String(err),
        });
      });
      inflight.set(unit.id, task);
      task.finally(() => inflight.delete(unit.id));
    };

    // Main scheduling loop: fill slots up to K, then wait for any completion.
    for (;;) {
      const state = useStore.getState();
      const pool = state.engagementPool;
      const stopping = pool.phase === "stopping";

      if (!stopping) {
        while (
          Object.keys(useStore.getState().engagementPool.running).length +
            countPendingLaunches(inflight) <
            useStore.getState().engagementPool.concurrency &&
          useStore.getState().engagementPool.queue.length > 0
        ) {
          const unit = useStore.getState().poolDequeue();
          if (!unit) break;
          launch(unit);
        }
      }

      if (inflight.size === 0) {
        const remaining = useStore.getState().engagementPool.queue.length;
        if (stopping || remaining === 0) break;
        // Queue non-empty but nothing launched (shouldn't happen) — avoid spin.
        await sleep(500);
        continue;
      }
      await Promise.race(inflight.values());
    }

    useStore.getState().poolSetPhase("complete");
    logSummary();
  } catch (err) {
    logger.error("[engagement] pool loop aborted:", err);
    useStore.getState().poolSetPhase("complete");
    throw err;
  } finally {
    poolActive = false;
  }
}

/** Graceful stop: freeze the queue; in-flight workers run to completion. */
export function stopEngagementRun(): void {
  useStore.getState().poolRequestStop();
}

/** Resume-skip: outcome `skipped` for queued units whose truth is in the DB. */
async function skipCoveredQueuedUnits(projectPath: string): Promise<void> {
  const queued = [...useStore.getState().engagementPool.queue];
  if (queued.length === 0) return;
  try {
    const byKind = {
      recon_family: await getEngagementSnapshot({
        projectPath,
        toStage: STAGE_SLICES.recon_family.to,
      }),
      attack_org: await getEngagementSnapshot({
        projectPath,
        toStage: STAGE_SLICES.attack_org.to,
      }),
    };
    for (const unit of queued) {
      if (unitAlreadyCovered(unit, byKind[unit.kind])) {
        // Remove from queue + record the skip.
        const state = useStore.getState();
        const idx = state.engagementPool.queue.findIndex((u) => u.id === unit.id);
        if (idx !== -1) {
          // poolDequeue only pops the head; rebuild via outcomes path instead.
          useStore.setState((s) => {
            s.engagementPool.queue.splice(idx, 1);
          });
        }
        state.poolMarkOutcome({ unitId: unit.id, status: "skipped" });
      }
    }
  } catch (e) {
    // Fail-closed: snapshot unavailable → skip nothing, just run everything.
    logger.warn("[engagement] resume-skip check failed; running all units", e);
  }
}

function findConvForUnit(unitId: string): string | null {
  const state = useStore.getState();
  for (const conv of Object.values(state.conversations)) {
    if (conv.workerMeta?.unitId === unitId) return conv.id;
  }
  return null;
}

/** Launches started but whose running-mark hasn't landed yet (race guard). */
function countPendingLaunches(inflight: Map<string, Promise<void>>): number {
  const running = useStore.getState().engagementPool.running;
  let pending = 0;
  for (const unitId of inflight.keys()) {
    if (!running[unitId]) pending += 1;
  }
  return pending;
}

function logSummary(): void {
  const { outcomes, knownUnits } = useStore.getState().engagementPool;
  const total = Object.keys(knownUnits).length;
  const byStatus = { passed: 0, blocked: 0, failed: 0, skipped: 0 };
  for (const o of Object.values(outcomes)) byStatus[o.status] += 1;
  logger.info(
    `[engagement] run complete: ${byStatus.passed + byStatus.skipped}/${total} covered ` +
      `(passed=${byStatus.passed} skipped=${byStatus.skipped} blocked=${byStatus.blocked} failed=${byStatus.failed})`
  );
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
