/**
 * Agent Tree Selector
 *
 * 把会话内的 sub-agent 调用关系组织成一棵树，供 SubAgentTreeView 渲染。
 *
 * 树结构：
 *   - root（main agent）的子节点是所有 depth=1 的 sub-agent
 *   - 每个 sub-agent 节点的子节点 = 它直接运行的 tool calls + 它生出来的下一级 sub-agent
 *   - 嵌套 sub-agent 通过 `parentRequestId` 反查上一级 sub-agent 的 toolCalls.id 关联
 *
 * 关于 depth：`ActiveSubAgent.depth` 由后端写入；如果 backend 还没填，
 * 退化处理为：parentRequestId 在 timeline 主 tool call 中能找到 → depth=1，
 * 否则按 toolCalls 反查归并。
 */
import type { ActiveSubAgent, SubAgentToolCall } from "../index";
import type { AnchorMap } from "./anchors";
import { selectAnchorMap } from "./anchors";
import { useStore } from "../index";

export type AgentTreeStatus = "running" | "completed" | "error" | "interrupted" | "pending";

export interface AgentTreeToolNode {
  kind: "tool";
  /** sub-agent.toolCalls 的 id（也是该 tool call 的 requestId 候选） */
  id: string;
  toolName: string;
  primary: string | null;
  status: AgentTreeStatus;
  durationMs?: number;
  /** anchor 编号（T# / P# / A#，主要是 T#） —— 仅当出现在主 timeline 时有 */
  anchor: string | null;
}

export interface AgentTreeAgentNode {
  kind: "agent";
  /** 该 sub-agent 的 parentRequestId（也是 anchor 查询 key） */
  id: string;
  agentId: string;
  agentName: string;
  task: string;
  status: AgentTreeStatus;
  durationMs?: number;
  anchor: string | null;
  /** 嵌套深度（main=0，第一层 sub-agent=1） */
  depth: number;
  children: Array<AgentTreeAgentNode | AgentTreeToolNode>;
}

export interface AgentTree {
  root: AgentTreeAgentNode;
  totalAgents: number;
  totalTools: number;
}

const EMPTY_AGENTS: ActiveSubAgent[] = [];

/* ──────────────────────────────────────────────────────────────────────── */
/*                                Cache                                     */
/* ──────────────────────────────────────────────────────────────────────── */

interface CacheEntry {
  subAgentsRef: unknown;
  anchorRef: AnchorMap;
  result: AgentTree;
}

const cache = new Map<string, CacheEntry>();

export function clearAgentTreeCache(sessionId?: string) {
  if (sessionId == null) cache.clear();
  else cache.delete(sessionId);
}

/* ──────────────────────────────────────────────────────────────────────── */
/*                              Compute                                     */
/* ──────────────────────────────────────────────────────────────────────── */

function primaryArgFromTool(tc: SubAgentToolCall): string | null {
  const args = tc.args;
  if (typeof args === "object" && args !== null) {
    const o = args as Record<string, unknown>;
    if (typeof o.command === "string") return o.command;
    if (typeof o.path === "string") return o.path;
    if (typeof o.file_path === "string") return o.file_path;
    if (typeof o.pattern === "string") return o.pattern;
    if (typeof o.url === "string") return o.url;
    if (typeof o.query === "string") return o.query;
  }
  return null;
}

function toolStatus(tc: SubAgentToolCall): AgentTreeStatus {
  const s = tc.status as string | undefined;
  if (s === "completed") return "completed";
  if (s === "error") return "error";
  if (s === "interrupted") return "interrupted";
  return "running";
}

function buildTree(
  subAgents: ActiveSubAgent[],
  anchors: AnchorMap,
): AgentTree {
  // 1) 反向索引：tool call id → 拥有它的 sub-agent
  //    这样我们能从一个嵌套 sub-agent 的 parentRequestId 反查它的 owner
  const ownerByToolId = new Map<string, ActiveSubAgent>();
  for (const sa of subAgents) {
    for (const tc of sa.toolCalls ?? []) {
      ownerByToolId.set(tc.id, sa);
    }
  }

  // 2) 哪些 tool calls 是 sub-agent 调用？需要从最终展示中排除（因为它们会以 agent 节点的形式出现）
  const subAgentInvocationToolIds = new Set<string>();
  for (const sa of subAgents) {
    if (sa.parentRequestId) {
      // sa.parentRequestId 可能是另一个 sub-agent 的 toolCall.id
      subAgentInvocationToolIds.add(sa.parentRequestId);
    }
  }

  // 3) 把每个 sub-agent 转换成节点（不带 children）
  const nodeByRequestId = new Map<string, AgentTreeAgentNode>();
  for (const sa of subAgents) {
    nodeByRequestId.set(sa.parentRequestId, {
      kind: "agent",
      id: sa.parentRequestId,
      agentId: sa.agentId,
      agentName: sa.agentName || sa.agentId || "subagent",
      task: sa.task ?? "",
      status: sa.status,
      durationMs: sa.durationMs,
      anchor: anchors.byRequestId.get(sa.parentRequestId) ?? null,
      depth: sa.depth ?? 1,
      children: [],
    });
  }

  // 4) 对每个 sub-agent，把它的 toolCalls 作为 children 挂上去；
  //    跳过那些"是用来调起子 sub-agent"的 tool call
  for (const sa of subAgents) {
    const node = nodeByRequestId.get(sa.parentRequestId)!;
    for (const tc of sa.toolCalls ?? []) {
      if (subAgentInvocationToolIds.has(tc.id)) {
        // 这是一个 sub-agent invocation —— 找到对应 sub-agent 节点挂上来
        const childAgent = nodeByRequestId.get(tc.id);
        if (childAgent) node.children.push(childAgent);
        // 否则丢弃（孤儿 invocation，可能 sub-agent 还没创建）
        continue;
      }
      node.children.push({
        kind: "tool",
        id: tc.id,
        toolName: tc.name,
        primary: primaryArgFromTool(tc),
        status: toolStatus(tc),
        durationMs:
          tc.completedAt && tc.startedAt
            ? new Date(tc.completedAt).getTime() - new Date(tc.startedAt).getTime()
            : undefined,
        anchor: anchors.byRequestId.get(tc.id) ?? null,
      });
    }
  }

  // 5) 找出顶层 sub-agent —— 它们的 parentRequestId 不是任何其他 sub-agent 的 toolCall.id
  const topLevel: AgentTreeAgentNode[] = [];
  for (const sa of subAgents) {
    const isNested = ownerByToolId.has(sa.parentRequestId);
    if (!isNested) {
      const node = nodeByRequestId.get(sa.parentRequestId);
      if (node) topLevel.push(node);
    }
  }

  // 6) 主 agent 根节点
  const root: AgentTreeAgentNode = {
    kind: "agent",
    id: "__main__",
    agentId: "main",
    agentName: "main agent",
    task: "",
    status: subAgents.some((a) => a.status === "running") ? "running" : "completed",
    anchor: null,
    depth: 0,
    children: topLevel,
  };

  // 7) 计数（递归）
  let totalAgents = 0;
  let totalTools = 0;
  const visit = (n: AgentTreeAgentNode | AgentTreeToolNode) => {
    if (n.kind === "agent") {
      if (n.id !== "__main__") totalAgents++;
      for (const c of n.children) visit(c);
    } else {
      totalTools++;
    }
  };
  visit(root);

  return { root, totalAgents, totalTools };
}

const EMPTY_TREE: AgentTree = {
  root: {
    kind: "agent",
    id: "__main__",
    agentId: "main",
    agentName: "main agent",
    task: "",
    status: "completed",
    anchor: null,
    depth: 0,
    children: [],
  },
  totalAgents: 0,
  totalTools: 0,
};

/* ──────────────────────────────────────────────────────────────────────── */
/*                            Public API                                    */
/* ──────────────────────────────────────────────────────────────────────── */

export function selectAgentTree(
  state: ReturnType<typeof useStore.getState>,
  sessionId: string,
): AgentTree {
  const subAgents = state.activeSubAgents[sessionId] ?? EMPTY_AGENTS;
  if (subAgents.length === 0) return EMPTY_TREE;

  const anchors = selectAnchorMap(state, sessionId);
  const cached = cache.get(sessionId);
  if (cached && cached.subAgentsRef === subAgents && cached.anchorRef === anchors) {
    return cached.result;
  }

  const result = buildTree(subAgents, anchors);
  cache.set(sessionId, {
    subAgentsRef: subAgents,
    anchorRef: anchors,
    result,
  });
  return result;
}

export function useAgentTree(sessionId: string | null | undefined): AgentTree {
  return useStore((state) => {
    if (!sessionId) return EMPTY_TREE;
    return selectAgentTree(state, sessionId);
  });
}
