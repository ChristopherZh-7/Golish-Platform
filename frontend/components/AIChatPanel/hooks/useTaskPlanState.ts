import { useCallback, useMemo, type MutableRefObject } from "react";
import { type ChatMessage, useStore } from "@/store";
import type { TaskPlanViewModel } from "../TaskPlan";

const EMPTY_RETIRED: any[] = [];

export function useTaskPlanState(
  messages: ChatMessage[],
  planMessageIdRef: MutableRefObject<string | null>,
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
    () => storePlan ? { version: storePlan.version, steps: storePlan.steps, summary: storePlan.summary } : null,
    [storePlan]
  );

  const storePlanMessageId = useStore((s) => {
    if (!s.activeConversationId) return null;
    const sid = s.conversations[s.activeConversationId]?.aiSessionId;
    if (sid && s.sessions[sid]?.planMessageId) return s.sessions[sid].planMessageId;
    const termIds = s.conversationTerminals[s.activeConversationId];
    const termId = termIds?.[0];
    if (termId) return s.sessions[termId]?.planMessageId ?? null;
    return null;
  });

  const retiredPlans = useStore(useCallback((s: any) => {
    if (!s.activeConversationId) return EMPTY_RETIRED;
    const sid = s.conversations[s.activeConversationId]?.aiSessionId;
    if (sid && s.sessions[sid]?.retiredPlans?.length) return s.sessions[sid].retiredPlans;
    const termIds = s.conversationTerminals[s.activeConversationId];
    const termId = termIds?.[0];
    if (termId && s.sessions[termId]?.retiredPlans?.length) return s.sessions[termId].retiredPlans;
    return EMPTY_RETIRED;
  }, []));

  const planTargetIdx = useMemo(() => {
    const msgId = storePlanMessageId ?? planMessageIdRef.current;
    if (msgId) { const idx = messages.findIndex((m) => m.id === msgId); if (idx >= 0) return idx; }
    for (let i = 0; i < messages.length; i++) {
      if (messages[i].role === "assistant" && messages[i].toolCalls?.some((tc) => tc.name === "update_plan")) return i;
    }
    if (taskPlan) { const firstAssistant = messages.findIndex((m) => m.role === "assistant"); if (firstAssistant >= 0) return firstAssistant; }
    return -1;
  }, [messages, taskPlan, storePlanMessageId]);

  const retiredPlansByMsg = useMemo(() => {
    const map = new Map<string, TaskPlanViewModel[]>();
    for (const rp of retiredPlans) {
      const list = map.get(rp.messageId) ?? [];
      list.push({ version: rp.plan.version, steps: rp.plan.steps, summary: rp.plan.summary, retiredAt: rp.retiredAt });
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
