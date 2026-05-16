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
});
