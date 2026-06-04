/**
 * Tests for the P0-1 plan-restore fallback in `useTaskPlanState`.
 *
 * The hook normally surfaces plans pushed by the backend via
 * `plan_updated` events. When a conversation activates after an app
 * restart, that broadcast may have already fired (and been missed) or
 * may be in flight; the fallback effect calls `getPlan(sessionId)` so
 * the previously persisted plan shows up regardless.
 *
 * See docs/design/2026-05-17-plan-restore-on-restart.md (Task 8).
 */

import { renderHook, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as ai from "@/lib/ai";
import { useStore } from "@/store";
import { writeStagePlans } from "../stagePlanPersistence";
import { useTaskPlanState } from "./useTaskPlanState";

vi.mock("@/lib/ai", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ai")>();
  return {
    ...actual,
    getPlan: vi.fn(),
  };
});

const SID = "ai-sid-1";
const CID = "conv-1";

function makeStubPlan(overrides: Partial<ReturnType<typeof basePlan>> = {}) {
  return { ...basePlan(), ...overrides };
}

function basePlan() {
  return {
    explanation: "Restored",
    steps: [
      { id: "s1", step: "do thing", status: "completed" as const },
      { id: "s2", step: "do other thing", status: "in_progress" as const },
    ],
    summary: { total: 2, completed: 1, in_progress: 1, pending: 0 },
    version: 3,
    updated_at: new Date().toISOString(),
  };
}

function resetStoreWithConversation(opts: {
  aiSessionId: string | null;
  storedPlanSessionId?: string | null;
  storedPlanSteps?: number;
} = { aiSessionId: SID }) {
  const sessions: Record<string, any> = {};
  if (opts.storedPlanSessionId) {
    sessions[opts.storedPlanSessionId] = {
      plan: {
        ...basePlan(),
        version: 99,
        steps: Array.from({ length: opts.storedPlanSteps ?? 1 }, (_, i) => ({
          id: `existing-${i}`,
          step: `existing step ${i}`,
          status: "in_progress" as const,
        })),
      },
    };
  }
  useStore.setState({
    activeConversationId: CID,
    conversations: {
      [CID]: {
        aiSessionId: opts.aiSessionId,
        messages: [],
      } as any,
    },
    conversationTerminals: {},
    sessions,
  } as any);
}

function renderHookEmpty() {
  return renderHook(() => {
    const ref = useRef<string | null>(null);
    return useTaskPlanState([], ref);
  });
}

describe("useTaskPlanState · P0-1 fallback fetch", () => {
  beforeEach(() => {
    (ai.getPlan as unknown as ReturnType<typeof vi.fn>).mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("calls getPlan when store has no plan for the active aiSessionId", async () => {
    (ai.getPlan as any).mockResolvedValue(makeStubPlan());
    resetStoreWithConversation({ aiSessionId: SID });

    renderHookEmpty();

    await waitFor(() => {
      expect(ai.getPlan).toHaveBeenCalledTimes(1);
    });
    expect(ai.getPlan).toHaveBeenCalledWith(SID);

    await waitFor(() => {
      const plan = useStore.getState().sessions[SID]?.plan;
      expect(plan?.version).toBe(3);
      expect(plan?.steps).toHaveLength(2);
    });
  });

  it("does NOT call getPlan when store already has a non-empty plan", async () => {
    (ai.getPlan as any).mockResolvedValue(makeStubPlan());
    resetStoreWithConversation({
      aiSessionId: SID,
      storedPlanSessionId: SID,
      storedPlanSteps: 2,
    });

    renderHookEmpty();

    await new Promise((r) => setTimeout(r, 20));
    expect(ai.getPlan).not.toHaveBeenCalled();

    // existing plan untouched
    const plan = useStore.getState().sessions[SID]?.plan;
    expect(plan?.version).toBe(99);
  });

  it("does NOT call getPlan when activeAiSessionId is null", async () => {
    (ai.getPlan as any).mockResolvedValue(makeStubPlan());
    resetStoreWithConversation({ aiSessionId: null });

    renderHookEmpty();

    await new Promise((r) => setTimeout(r, 20));
    expect(ai.getPlan).not.toHaveBeenCalled();
  });

  it("does NOT overwrite a newer plan that arrived via plan_updated during the fetch", async () => {
    let resolveFn: (p: ReturnType<typeof makeStubPlan>) => void;
    (ai.getPlan as any).mockImplementation(
      () =>
        new Promise((r) => {
          resolveFn = r;
        })
    );

    resetStoreWithConversation({ aiSessionId: SID });
    renderHookEmpty();

    await waitFor(() => {
      expect(ai.getPlan).toHaveBeenCalledTimes(1);
    });

    // Simulate a plan_updated event arriving while getPlan is pending.
    useStore.setState((s: any) => ({
      sessions: {
        ...s.sessions,
        [SID]: {
          plan: makeStubPlan({
            version: 42,
            steps: [{ id: "live", step: "live step", status: "in_progress" }],
            summary: { total: 1, completed: 0, in_progress: 1, pending: 0 },
          }),
        },
      },
    }));

    // Now resolve the slower getPlan with the OLDER snapshot.
    resolveFn!(makeStubPlan({ version: 1 }));

    await new Promise((r) => setTimeout(r, 20));

    const plan = useStore.getState().sessions[SID]?.plan;
    expect(plan?.version).toBe(42);
    expect(plan?.steps?.[0].step).toBe("live step");
  });

  it("silently swallows getPlan rejections", async () => {
    (ai.getPlan as any).mockRejectedValue(new Error("backend boom"));
    resetStoreWithConversation({ aiSessionId: SID });

    const consoleSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    renderHookEmpty();
    await waitFor(() => {
      expect(ai.getPlan).toHaveBeenCalled();
    });

    await new Promise((r) => setTimeout(r, 20));

    // Store still has no plan
    expect(useStore.getState().sessions[SID]?.plan).toBeUndefined();
    // Warning logged
    expect(consoleSpy).toHaveBeenCalled();
    consoleSpy.mockRestore();
  });

  it("treats an empty plan (version 0) from an uninitialized session as a silent no-op", async () => {
    // P2 · `get_plan` now returns an empty plan (version 0) instead of throwing
    // `ai_session_not_initialized` when the session bridge isn't registered yet.
    // The fallback must treat that as "no plan": no store write AND — crucially —
    // NO console.warn (the old throw path warned on every early restore).
    (ai.getPlan as any).mockResolvedValue(
      makeStubPlan({
        version: 0,
        steps: [],
        summary: { total: 0, completed: 0, in_progress: 0, pending: 0 },
      })
    );
    resetStoreWithConversation({ aiSessionId: SID });

    const consoleSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    renderHookEmpty();
    await waitFor(() => {
      expect(ai.getPlan).toHaveBeenCalledTimes(1);
    });
    await new Promise((r) => setTimeout(r, 20));

    // No plan written, and no noisy warning (unlike the rejection path above).
    expect(useStore.getState().sessions[SID]?.plan).toBeUndefined();
    expect(consoleSpy).not.toHaveBeenCalled();
    consoleSpy.mockRestore();
  });
});

function stubStagePlan(version: number, status: "completed" | "in_progress") {
  return {
    version,
    explanation: null,
    updated_at: "2026-06-04T00:00:00.000Z",
    steps: [{ id: `s${version}`, step: "step", status }],
    summary: {
      total: 1,
      completed: status === "completed" ? 1 : 0,
      in_progress: status === "in_progress" ? 1 : 0,
      pending: 0,
    },
  };
}

describe("useTaskPlanState · per-stage roadmap persistence (refresh restore)", () => {
  beforeEach(() => {
    // P0-1 fallback calls getPlan; resolve to a harmless empty value here.
    (ai.getPlan as unknown as ReturnType<typeof vi.fn>).mockResolvedValue(undefined as never);
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
  });

  it("restores stageOrder, plansByStage AND passedStages (cards + completions + floating-bar source)", async () => {
    const CONV = "conv-persist";
    const TERM = "term-persist";
    useStore.setState({
      activeConversationId: CONV,
      conversations: { [CONV]: { aiSessionId: "ai-persist", messages: [] } as any },
      conversationTerminals: { [CONV]: [TERM] },
      sessions: { [TERM]: { id: TERM } as any },
    } as any);

    writeStagePlans(CONV, {
      stageOrder: ["scoping", "target_intel"],
      plansByStage: {
        scoping: stubStagePlan(1, "completed"),
        target_intel: stubStagePlan(2, "in_progress"),
      },
      passedStages: ["scoping"],
    });

    renderHookEmpty();

    await waitFor(() => {
      const sess = useStore.getState().sessions[TERM] as any;
      // Cards + run order
      expect(sess?.stageOrder).toEqual(["scoping", "target_intel"]);
      expect(sess?.plansByStage?.target_intel?.version).toBe(2);
      // "阶段完成" (drives the green check + the floating bar's current-stage calc)
      expect(sess?.passedStages).toEqual(["scoping"]);
    });
  });

  it("does NOT clobber newer in-memory per-stage state with a stale snapshot", async () => {
    const CONV = "conv-live";
    const TERM = "term-live";
    useStore.setState({
      activeConversationId: CONV,
      conversations: { [CONV]: { aiSessionId: "ai-live", messages: [] } as any },
      conversationTerminals: { [CONV]: [TERM] },
      sessions: {
        [TERM]: {
          id: TERM,
          stageOrder: ["recon"],
          plansByStage: { recon: stubStagePlan(5, "in_progress") },
          passedStages: [],
        } as any,
      },
    } as any);

    writeStagePlans(CONV, {
      stageOrder: ["scoping"],
      plansByStage: { scoping: stubStagePlan(1, "completed") },
      passedStages: ["scoping"],
    });

    renderHookEmpty();
    await new Promise((r) => setTimeout(r, 20));

    const sess = useStore.getState().sessions[TERM] as any;
    expect(sess?.stageOrder).toEqual(["recon"]);
    expect(sess?.passedStages).toEqual([]);
  });
});

describe("useTaskPlanState · roadmap anchor stability (per-stage)", () => {
  const CID2 = "conv-anchor";
  const SID2 = "ai-anchor";

  beforeEach(() => {
    (ai.getPlan as unknown as ReturnType<typeof vi.fn>).mockResolvedValue(undefined as never);
    localStorage.clear();
    useStore.setState({
      activeConversationId: CID2,
      conversations: { [CID2]: { aiSessionId: SID2, messages: [] } as any },
      conversationTerminals: {},
      sessions: {
        [SID2]: {
          id: SID2,
          stageOrder: ["scoping", "target_intel"],
          plansByStage: {
            scoping: stubStagePlan(0, "completed"),
            target_intel: stubStagePlan(1, "in_progress"),
          },
          passedStages: ["scoping"],
        } as any,
      },
    } as any);
  });

  afterEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
  });

  it("anchors the roadmap at the first assistant message, not a later update_plan message", () => {
    // Scoping (confirm-only) never calls update_plan; stage 2 does. The roadmap
    // must stay at the first assistant message so it doesn't appear to vanish
    // when stage 2 plans.
    const messages = [
      { id: "u1", role: "user", content: "go", timestamp: 0 },
      { id: "a1", role: "assistant", content: "thinking", timestamp: 1 },
      {
        id: "a2",
        role: "assistant",
        content: "planning target intel",
        timestamp: 2,
        toolCalls: [{ name: "update_plan", args: "{}", requestId: "r1" }],
      },
    ] as never[];
    const { result } = renderHook(() => {
      const ref = useRef<string | null>(null);
      return useTaskPlanState(messages, ref);
    });
    expect(result.current.stagePlans).not.toBeNull();
    expect(result.current.planTargetIdx).toBe(1);
  });
});
