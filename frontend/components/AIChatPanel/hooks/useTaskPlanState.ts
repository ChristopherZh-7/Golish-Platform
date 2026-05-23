import { type MutableRefObject, useCallback, useEffect, useMemo } from "react";
import { getPlan } from "@/lib/ai";
import { type ChatMessage, useStore } from "@/store";
import type { TaskPlanViewModel } from "../TaskPlan";

const EMPTY_RETIRED: any[] = [];

export function useTaskPlanState(
  messages: ChatMessage[],
  planMessageIdRef: MutableRefObject<string | null>
) {
  const activeAiSessionId = useStore((s) => {
    if (!s.activeConversationId) return null;
    return s.conversations[s.activeConversationId]?.aiSessionId ?? null;
  });

  const storePlan = useStore((s) => {
    if (!s.activeConversationId) return null;
    const sid = s.conversations[s.activeConversationId]?.aiSessionId;
    if (sid) {
      const plan = s.sessions[sid]?.plan;
      if (plan) return plan;
    }
    const termIds = s.conversationTerminals[s.activeConversationId];
    const termId = termIds?.[0];
    if (termId) return s.sessions[termId]?.plan ?? null;
    return null;
  });

  const taskPlan = useMemo<TaskPlanViewModel | null>(
    () =>
      storePlan
        ? { version: storePlan.version, steps: storePlan.steps, summary: storePlan.summary }
        : null,
    [storePlan]
  );

  // P0-1 fallback fetch: when a conversation activates with an
  // `aiSessionId` but the store has no plan for it (e.g. fresh app start
  // and the `PlanUpdated` broadcast was missed), pull the latest plan
  // snapshot from the backend so the restored plan still shows up.
  //
  // Re-checks the store after the request returns to avoid clobbering a
  // newer plan that arrived via the `plan_updated` event in flight.
  useEffect(() => {
    if (!activeAiSessionId) return;
    if (storePlan?.steps && storePlan.steps.length > 0) return;

    let cancelled = false;
    const sid = activeAiSessionId;
    getPlan(sid)
      .then((plan) => {
        if (cancelled) return;
        if (!plan || plan.version === 0 || !plan.steps || plan.steps.length === 0) {
          return;
        }
        const current = useStore.getState().sessions[sid]?.plan;
        if (current?.steps && current.steps.length > 0) {
          // Event handler already populated it; honour the newer copy.
          return;
        }
        useStore.getState().setPlan(sid, plan);
      })
      .catch((err) => {
        // Non-fatal: backend may have no plan, or this is a brand-new session.
        // eslint-disable-next-line no-console
        console.warn("[useTaskPlanState] getPlan fallback failed", err);
      });

    return () => {
      cancelled = true;
    };
  }, [activeAiSessionId, storePlan]);

  const storePlanMessageId = useStore((s) => {
    if (!s.activeConversationId) return null;
    const sid = s.conversations[s.activeConversationId]?.aiSessionId;
    if (sid && s.sessions[sid]?.planMessageId) return s.sessions[sid].planMessageId;
    const termIds = s.conversationTerminals[s.activeConversationId];
    const termId = termIds?.[0];
    if (termId) return s.sessions[termId]?.planMessageId ?? null;
    return null;
  });

  const retiredPlans = useStore(
    useCallback((s: any) => {
      if (!s.activeConversationId) return EMPTY_RETIRED;
      const sid = s.conversations[s.activeConversationId]?.aiSessionId;
      if (sid && s.sessions[sid]?.retiredPlans?.length) return s.sessions[sid].retiredPlans;
      const termIds = s.conversationTerminals[s.activeConversationId];
      const termId = termIds?.[0];
      if (termId && s.sessions[termId]?.retiredPlans?.length)
        return s.sessions[termId].retiredPlans;
      return EMPTY_RETIRED;
    }, [])
  );

  const planTargetIdx = useMemo(() => {
    const msgId = storePlanMessageId ?? planMessageIdRef.current;
    if (msgId) {
      const idx = messages.findIndex((m) => m.id === msgId);
      if (idx >= 0) return idx;
    }
    for (let i = 0; i < messages.length; i++) {
      if (
        messages[i].role === "assistant" &&
        messages[i].toolCalls?.some((tc) => tc.name === "update_plan")
      )
        return i;
    }
    if (taskPlan) {
      const firstAssistant = messages.findIndex((m) => m.role === "assistant");
      if (firstAssistant >= 0) return firstAssistant;
    }
    return -1;
  }, [messages, taskPlan, storePlanMessageId, planMessageIdRef.current]);

  const retiredPlansByMsg = useMemo(() => {
    const map = new Map<string, TaskPlanViewModel[]>();
    for (const rp of retiredPlans) {
      const list = map.get(rp.messageId) ?? [];
      list.push({
        version: rp.plan.version,
        steps: rp.plan.steps,
        summary: rp.plan.summary,
        retiredAt: rp.retiredAt,
      });
      map.set(rp.messageId, list);
    }
    return map;
  }, [retiredPlans]);

  return {
    activeAiSessionId,
    taskPlan,
    planTargetIdx,
    retiredPlansByMsg,
  };
}
