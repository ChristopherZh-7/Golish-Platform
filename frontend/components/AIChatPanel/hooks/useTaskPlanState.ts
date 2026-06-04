import { type MutableRefObject, useCallback, useEffect, useMemo } from "react";
import { getPlan } from "@/lib/ai";
import { collectStageMarkers, writeStageMarkers } from "@/lib/stage-marker-persistence";
import { type ChatMessage, useStore } from "@/store";
import { readStagePlans, writeStagePlans } from "../stagePlanPersistence";
import type { StagePlansViewModel, TaskPlanViewModel } from "../TaskPlan";

const EMPTY_RETIRED: any[] = [];

export function useTaskPlanState(
  messages: ChatMessage[],
  planMessageIdRef: MutableRefObject<string | null>
) {
  const activeAiSessionId = useStore((s) => {
    if (!s.activeConversationId) return null;
    return s.conversations[s.activeConversationId]?.aiSessionId ?? null;
  });

  // Stable across reloads (the conversation DB restores by id) — used as the
  // localStorage key for per-stage roadmap persistence below.
  const activeConversationId = useStore((s) => s.activeConversationId);
  // The terminal/PTY session id the live event path writes per-stage state
  // under (setStagePlan / markStagePassed). Restore must target the SAME id.
  const activeTermId = useStore((s) =>
    s.activeConversationId ? (s.conversationTerminals[s.activeConversationId]?.[0] ?? null) : null
  );

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

  // Per-stage plan buckets (task mode, design 2026-06-04). Selected as separate
  // stable refs (Immer gives a fresh ref only when that stage's data changes),
  // then combined in a memo so unrelated state changes don't churn.
  const storeStageOrder = useStore((s) => {
    if (!s.activeConversationId) return null;
    const sid = s.conversations[s.activeConversationId]?.aiSessionId;
    if (sid && s.sessions[sid]?.stageOrder?.length) return s.sessions[sid].stageOrder ?? null;
    const termId = s.conversationTerminals[s.activeConversationId]?.[0];
    if (termId && s.sessions[termId]?.stageOrder?.length)
      return s.sessions[termId].stageOrder ?? null;
    return null;
  });
  const storePlansByStage = useStore((s) => {
    if (!s.activeConversationId) return null;
    const sid = s.conversations[s.activeConversationId]?.aiSessionId;
    if (sid && s.sessions[sid]?.plansByStage) return s.sessions[sid].plansByStage ?? null;
    const termId = s.conversationTerminals[s.activeConversationId]?.[0];
    if (termId && s.sessions[termId]?.plansByStage) return s.sessions[termId].plansByStage ?? null;
    return null;
  });
  const storePassedStages = useStore((s) => {
    if (!s.activeConversationId) return null;
    const sid = s.conversations[s.activeConversationId]?.aiSessionId;
    if (sid && s.sessions[sid]?.passedStages?.length) return s.sessions[sid].passedStages ?? null;
    const termId = s.conversationTerminals[s.activeConversationId]?.[0];
    if (termId && s.sessions[termId]?.passedStages?.length)
      return s.sessions[termId].passedStages ?? null;
    return null;
  });

  const stagePlans = useMemo<StagePlansViewModel | null>(() => {
    if (!storeStageOrder?.length || !storePlansByStage) return null;
    const plansByStage: Record<string, TaskPlanViewModel> = {};
    for (const sid of storeStageOrder) {
      const p = storePlansByStage[sid];
      if (p) plansByStage[sid] = { version: p.version, steps: p.steps, summary: p.summary };
    }
    if (Object.keys(plansByStage).length === 0) return null;
    return { stageOrder: storeStageOrder, plansByStage, passedStages: storePassedStages ?? [] };
  }, [storeStageOrder, storePlansByStage, storePassedStages]);

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
    // Per-stage plans present → don't pull the legacy single snapshot; it would
    // re-introduce a merged single card alongside the per-stage cards.
    if (storeStageOrder?.length) return;

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
  }, [activeAiSessionId, storePlan, storeStageOrder]);

  // Persist the per-stage roadmap so it survives a webview refresh / app restart
  // (design 2026-06-04). Unlike the single `plan` (DB-backed via
  // `terminal_state.planJson`), the per-stage buckets live only in memory and are
  // rebuilt from streamed events that don't replay on restore — so snapshot them
  // to localStorage (keyed by the stable conversation id), mirroring
  // `contextUsagePersistence`. `writeStagePlans` no-ops on an empty `stageOrder`,
  // so an uninitialized store can never clobber a saved snapshot.
  useEffect(() => {
    if (!activeConversationId || !storeStageOrder?.length || !storePlansByStage) return;
    writeStagePlans(activeConversationId, {
      stageOrder: storeStageOrder,
      plansByStage: storePlansByStage,
      passedStages: storePassedStages ?? [],
    });
  }, [activeConversationId, storeStageOrder, storePlansByStage, storePassedStages]);

  // Restore the persisted per-stage roadmap into the store when a conversation
  // activates with no in-memory per-stage state (e.g. right after a refresh).
  // Writes under the resolved terminal id — the same id the live event path uses
  // — so the cards read it back and a later live update supersedes it cleanly.
  useEffect(() => {
    if (!activeConversationId || !activeTermId) return;
    if (storeStageOrder?.length) return; // live/restored state already present
    const persisted = readStagePlans(activeConversationId);
    if (!persisted || persisted.stageOrder.length === 0) return;
    const st = useStore.getState();
    for (const stageId of persisted.stageOrder) {
      const plan = persisted.plansByStage[stageId];
      if (plan) st.setStagePlan(activeTermId, stageId, plan);
    }
    for (const stageId of persisted.passedStages) {
      st.markStagePassed(activeTermId, stageId);
    }
  }, [activeConversationId, activeTermId, storeStageOrder]);

  // Persist task-mode stage dividers ("Stage/Step complete" bubbles). They're
  // runtime-only `role:"system"` messages dropped by the conversation DB
  // (`isPersistableMessage`), so snapshot them to localStorage keyed by the
  // conversation id; `conversation-db-sync` re-splices them on reload. The string
  // signature gates writes so streaming text deltas don't thrash localStorage.
  const stageMarkers = useMemo(() => collectStageMarkers(messages), [messages]);
  const stageMarkerSig = useMemo(
    () =>
      stageMarkers.map((p) => `${p.anchorId ?? ""}>${p.marker.kind}:${p.marker.label}`).join("|"),
    [stageMarkers]
  );
  useEffect(() => {
    if (activeConversationId && stageMarkers.length > 0) {
      writeStageMarkers(activeConversationId, stageMarkers);
    }
    // Gated on the marker signature only — `stageMarkers` ref churns on every
    // streaming delta, but its contents (and thus the snapshot) change rarely.
  }, [activeConversationId, stageMarkerSig]);

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
    const hasStagePlanCards = !!stagePlans && stagePlans.stageOrder.length > 0;
    const msgId = storePlanMessageId ?? planMessageIdRef.current;
    if (msgId) {
      const idx = messages.findIndex((m) => m.id === msgId);
      if (idx >= 0) return idx;
    }
    // Per-stage (harness) mode: the roadmap renders ALL stages in one stack and
    // must stay anchored at a STABLE spot — the first assistant message of the
    // run. Chasing the first `update_plan` toolCall made the whole roadmap jump
    // to a later stage's message (and vanish from its original spot) the moment
    // that stage planned, which read as "the previous stage's card disappeared".
    // The toolCall anchor is therefore reserved for the legacy single chat-plan
    // card (chat mode), where the plan belongs to the message that emitted it.
    if (!hasStagePlanCards) {
      for (let i = 0; i < messages.length; i++) {
        if (
          messages[i].role === "assistant" &&
          messages[i].toolCalls?.some((tc) => tc.name === "update_plan")
        )
          return i;
      }
    }
    if (taskPlan || hasStagePlanCards) {
      const firstAssistant = messages.findIndex((m) => m.role === "assistant");
      if (firstAssistant >= 0) return firstAssistant;
    }
    return -1;
  }, [messages, taskPlan, stagePlans, storePlanMessageId, planMessageIdRef.current]);

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
    stagePlans,
    planTargetIdx,
    retiredPlansByMsg,
  };
}
