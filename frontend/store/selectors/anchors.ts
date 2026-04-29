/**
 * Anchor Map Selector
 *
 * 给会话内的 tool / sub-agent / pipeline 分配单调递增的"锚点编号"，
 * 让用户能在 ChatPanel 的小卡和 DetailView 的大卡之间一眼对上号。
 *
 * 编号规则（按 timeline 顺序遍历）：
 *   - 普通 tool call           → T1, T2, T3, …
 *   - sub-agent invocation     → A1, A2, A3, …
 *   - pipeline run             → P1, P2, P3, …
 *
 * 排除：
 *   - update_plan：不算 tool（属于 plan 维度）
 *
 * 锚点不写进 store —— 每次渲染由 timeline 顺序推导出来。timeline 是 append-only
 * 的，正常情况下 requestId → anchor 的映射稳定，除非 store 整体重置。
 *
 * 为什么不存到 store：
 *   1. 不需要改 schema / 迁移历史数据
 *   2. 单一事实来源 = timeline 顺序
 *   3. 性能可接受：timeline 通常 < 数百条，且按 sessionId 缓存
 */
import { useStore } from "../index";

export interface AnchorMap {
  /** requestId → "T3" / "A2" / "P1" */
  byRequestId: Map<string, string>;
  /** anchor 字符串 → requestId（反查用） */
  byAnchor: Map<string, string>;
}

const EMPTY_MAP: AnchorMap = {
  byRequestId: new Map(),
  byAnchor: new Map(),
};

/* ──────────────────────────────────────────────────────────────────────── */
/*                              Cache                                       */
/* ──────────────────────────────────────────────────────────────────────── */

interface CacheEntry {
  /** timeline 引用（用于命中检测） */
  timelineRef: unknown;
  result: AnchorMap;
}

const cache = new Map<string, CacheEntry>();

/** 测试 / hot-reload 用 */
export function clearAnchorCache(sessionId?: string) {
  if (sessionId == null) cache.clear();
  else cache.delete(sessionId);
}

/* ──────────────────────────────────────────────────────────────────────── */
/*                           Compute                                        */
/* ──────────────────────────────────────────────────────────────────────── */

function computeAnchorMap(
  state: ReturnType<typeof useStore.getState>,
  sessionId: string,
): AnchorMap {
  const timeline = state.timelines[sessionId];
  if (!timeline || timeline.length === 0) return EMPTY_MAP;

  const byRequestId = new Map<string, string>();
  const byAnchor = new Map<string, string>();
  let toolCount = 0;
  let agentCount = 0;
  let pipelineCount = 0;

  for (const block of timeline) {
    if (block.type === "ai_tool_execution") {
      const name = block.data.toolName;
      const requestId = block.data.requestId;
      if (!requestId) continue;
      if (byRequestId.has(requestId)) continue;

      if (name === "update_plan") continue;

      let anchor: string;
      if (name.startsWith("sub_agent_")) {
        agentCount += 1;
        anchor = `A${agentCount}`;
      } else if (name === "run_pipeline") {
        pipelineCount += 1;
        anchor = `P${pipelineCount}`;
      } else {
        toolCount += 1;
        anchor = `T${toolCount}`;
      }
      byRequestId.set(requestId, anchor);
      byAnchor.set(anchor, requestId);
    } else if (block.type === "sub_agent_activity") {
      // sub-agent activity 块的 key 是 parentRequestId（即调起这个 sub-agent 的那次工具调用 id）
      const requestId = block.data.parentRequestId;
      if (!requestId || byRequestId.has(requestId)) continue;
      agentCount += 1;
      const anchor = `A${agentCount}`;
      byRequestId.set(requestId, anchor);
      byAnchor.set(anchor, requestId);
    } else if (block.type === "pipeline_progress") {
      const id = block.id;
      if (!id || byRequestId.has(id)) continue;
      pipelineCount += 1;
      const anchor = `P${pipelineCount}`;
      byRequestId.set(id, anchor);
      byAnchor.set(anchor, id);
    }
  }

  return { byRequestId, byAnchor };
}

/* ──────────────────────────────────────────────────────────────────────── */
/*                            Public API                                    */
/* ──────────────────────────────────────────────────────────────────────── */

/**
 * 选 anchor map（带 sessionId 维度的缓存）。可以脱开 hook 直接传 state 用。
 */
export function selectAnchorMap(
  state: ReturnType<typeof useStore.getState>,
  sessionId: string,
): AnchorMap {
  const timeline = state.timelines[sessionId];
  const cached = cache.get(sessionId);
  if (cached && cached.timelineRef === timeline) return cached.result;

  const result = computeAnchorMap(state, sessionId);
  cache.set(sessionId, { timelineRef: timeline, result });
  return result;
}

/**
 * Hook 形式：自动订阅 timeline 变更。
 *
 * 用法：
 *   const anchors = useAnchorMap(sessionId);
 *   const id = anchors.byRequestId.get(tc.requestId); // "T3"
 */
export function useAnchorMap(sessionId: string | null | undefined): AnchorMap {
  return useStore((state) => {
    if (!sessionId) return EMPTY_MAP;
    return selectAnchorMap(state, sessionId);
  });
}

/**
 * 只取单个 anchor —— 适合在叶子组件里用，避免每次都拿整个 Map。
 */
export function useAnchorFor(
  sessionId: string | null | undefined,
  requestId: string | null | undefined,
): string | null {
  return useStore((state) => {
    if (!sessionId || !requestId) return null;
    return selectAnchorMap(state, sessionId).byRequestId.get(requestId) ?? null;
  });
}
