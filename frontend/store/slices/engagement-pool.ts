/**
 * Engagement worker-pool slice (设计 2026-06-13-engagement-scoping-fanout
 * §6.1, Phase B).
 *
 * Holds the fan-out pool's observable state: queued units, running workers
 * (with their conversation ids for drill-in), terminal outcomes, and the
 * pool lifecycle phase. The scheduling loop itself lives in
 * `lib/engagement/runPool.ts` — this slice is the single source of truth it
 * mutates, and what the Phase C overview renders.
 */

import type { WorkerOutcome, WorkerUnit } from "@/lib/engagement/pool";
import type { SliceCreator } from "./types";

/** A worker session currently occupying a pool slot. */
export interface RunningWorker {
  unit: WorkerUnit;
  /** Conversation id of the spawned worker session (drill-in target). */
  convId: string;
  startedAt: number;
}

export type EngagementPoolPhase = "idle" | "running" | "stopping" | "complete";

export interface EngagementPoolState {
  engagementPool: {
    phase: EngagementPoolPhase;
    /** Max simultaneously active workers (spec default 3). */
    concurrency: number;
    /** FIFO of units waiting for a slot. */
    queue: WorkerUnit[];
    /** unitId → running worker. */
    running: Record<string, RunningWorker>;
    /** unitId → terminal outcome (passed/blocked/failed/skipped). */
    outcomes: Record<string, WorkerOutcome>;
    /** All units ever enqueued this run, in order (overview denominator). */
    knownUnits: Record<string, WorkerUnit>;
    /** Project the pool is running against (engagement identity). */
    projectPath: string | null;
    /** Conversation that acts as the engagement overview (scoping chat). */
    overviewConvId: string | null;
  };
}

export interface EngagementPoolActions {
  /** Reset + configure the pool for a fresh run. */
  poolConfigure: (args: {
    concurrency: number;
    projectPath: string;
    overviewConvId: string | null;
  }) => void;
  poolSetConcurrency: (k: number) => void;
  poolEnqueue: (units: WorkerUnit[]) => void;
  /** Take the next queued unit (returns null when queue empty). */
  poolDequeue: () => WorkerUnit | null;
  poolMarkRunning: (unit: WorkerUnit, convId: string) => void;
  poolMarkOutcome: (outcome: WorkerOutcome) => void;
  poolSetPhase: (phase: EngagementPoolPhase) => void;
  /** Request a graceful stop: in-flight workers finish, queue stops draining. */
  poolRequestStop: () => void;
  poolReset: () => void;
}

export interface EngagementPoolSlice extends EngagementPoolState, EngagementPoolActions {}

export const initialEngagementPoolState: EngagementPoolState = {
  engagementPool: {
    phase: "idle",
    concurrency: 3,
    queue: [],
    running: {},
    outcomes: {},
    knownUnits: {},
    projectPath: null,
    overviewConvId: null,
  },
};

export const createEngagementPoolSlice: SliceCreator<EngagementPoolSlice> = (set, get) => ({
  ...initialEngagementPoolState,

  poolConfigure: ({ concurrency, projectPath, overviewConvId }) =>
    set((state) => {
      state.engagementPool = {
        ...initialEngagementPoolState.engagementPool,
        concurrency: Math.max(1, concurrency),
        projectPath,
        overviewConvId,
      };
    }),

  poolSetConcurrency: (k) =>
    set((state) => {
      state.engagementPool.concurrency = Math.max(1, k);
    }),

  poolEnqueue: (units) =>
    set((state) => {
      const p = state.engagementPool;
      for (const unit of units) {
        // A unit is enqueued at most once per run (dedupe by id).
        if (p.knownUnits[unit.id]) continue;
        p.knownUnits[unit.id] = unit;
        p.queue.push(unit);
      }
    }),

  poolDequeue: () => {
    const p = (get() as EngagementPoolState).engagementPool;
    const next = p.queue[0] ?? null;
    if (next) {
      set((state) => {
        state.engagementPool.queue.shift();
      });
    }
    return next;
  },

  poolMarkRunning: (unit, convId) =>
    set((state) => {
      state.engagementPool.running[unit.id] = {
        unit,
        convId,
        startedAt: Date.now(),
      };
    }),

  poolMarkOutcome: (outcome) =>
    set((state) => {
      const p = state.engagementPool;
      delete p.running[outcome.unitId];
      p.outcomes[outcome.unitId] = outcome;
    }),

  poolSetPhase: (phase) =>
    set((state) => {
      state.engagementPool.phase = phase;
    }),

  poolRequestStop: () =>
    set((state) => {
      if (state.engagementPool.phase === "running") {
        state.engagementPool.phase = "stopping";
      }
    }),

  poolReset: () =>
    set((state) => {
      state.engagementPool = { ...initialEngagementPoolState.engagementPool };
    }),
});

// Selectors
export const selectPoolActiveCount = <T extends EngagementPoolState>(state: T): number =>
  Object.keys(state.engagementPool.running).length;

export const selectPoolQueueLength = <T extends EngagementPoolState>(state: T): number =>
  state.engagementPool.queue.length;
