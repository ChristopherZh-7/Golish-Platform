/**
 * SubAgentTreeView
 *
 * "Sub-Agent 调用树"视图。展示当前会话下所有 sub-agent 的调用层次：
 *   - 主 agent 是树根（虚拟）
 *   - 每个 sub-agent 显示其状态、AnchorChip ([A#])、task 摘要
 *   - 每个 sub-agent 的 tool calls 作为子节点（带 [T#] / 命令预览 / 状态）
 *   - 嵌套 sub-agent 通过 `parentRequestId` 反查上一级的 toolCalls 关联起来
 *
 * 数据来源：useAgentTree(sessionId) selector。
 */
import {
  ArrowLeft,
  Bot,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  Loader2,
  Maximize2,
} from "lucide-react";
import { Fragment, memo, useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SubAgentDetailsModal } from "@/components/SubAgentCard/SubAgentDetailsModal";
import { AnchorChip } from "@/components/ui/AnchorChip";
import { getAgentColor, getAgentIcon } from "@/lib/sub-agent-theme";
import { formatDurationShort } from "@/lib/time";
import { getToolColor, getToolIcon } from "@/lib/tools";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent } from "@/store";
import { useStore } from "@/store";
import {
  type AgentTreeAgentNode,
  type AgentTreeStatus,
  type AgentTreeToolNode,
  useAgentTree,
} from "@/store/selectors";

interface SubAgentTreeViewProps {
  sessionId: string;
}

interface FlatRow {
  node: AgentTreeAgentNode | AgentTreeToolNode;
  depth: number;
  ancestorIsLast: boolean[];
  isLast: boolean;
  hasChildren: boolean;
  collapsed: boolean;
  parentId: string | null;
}

function flatten(
  root: AgentTreeAgentNode,
  collapsedIds: Set<string>,
): FlatRow[] {
  const out: FlatRow[] = [];
  const walk = (
    node: AgentTreeAgentNode | AgentTreeToolNode,
    depth: number,
    ancestorIsLast: boolean[],
    isLast: boolean,
    parentId: string | null,
  ) => {
    const hasChildren = node.kind === "agent" && node.children.length > 0;
    const collapsed = collapsedIds.has(node.id);
    out.push({ node, depth, ancestorIsLast, isLast, hasChildren, collapsed, parentId });

    if (node.kind === "agent" && hasChildren && !collapsed) {
      const kids = node.children;
      kids.forEach((child, i) => {
        const childIsLast = i === kids.length - 1;
        walk(child, depth + 1, [...ancestorIsLast, isLast], childIsLast, node.id);
      });
    }
  };
  walk(root, 0, [], true, null);
  return out;
}

function flattenAllAgentIds(node: AgentTreeAgentNode | AgentTreeToolNode, acc: string[] = []): string[] {
  if (node.kind === "agent") {
    if (node.id !== "__main__") acc.push(node.id);
    for (const c of node.children) flattenAllAgentIds(c, acc);
  }
  return acc;
}

/* ──────────────────────────────────────────────────────────────────────── */
/*                              Atoms                                       */
/* ──────────────────────────────────────────────────────────────────────── */

function StatusGlyph({ status }: { status: AgentTreeStatus }) {
  switch (status) {
    case "running":
      return <Loader2 className="w-3 h-3 text-accent animate-spin flex-shrink-0" />;
    case "completed":
      return (
        <svg className="w-3 h-3 text-green-500 flex-shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
          <path d="M20 6L9 17l-5-5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "error":
      return (
        <svg className="w-3 h-3 text-red-400 flex-shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
          <circle cx="12" cy="12" r="9" />
          <path d="M15 9l-6 6M9 9l6 6" strokeLinecap="round" />
        </svg>
      );
    case "interrupted":
      return (
        <svg className="w-3 h-3 text-amber-400 flex-shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
          <circle cx="12" cy="12" r="9" />
          <path d="M12 8v5M12 16h.01" strokeLinecap="round" />
        </svg>
      );
    default:
      return <div className="w-3 h-3 rounded-full border-[1.5px] border-muted-foreground/25 flex-shrink-0" />;
  }
}

function DurationLabel({ ms }: { ms?: number }) {
  if (ms == null) return null;
  return (
    <span className="text-[10px] text-muted-foreground/60 tabular-nums flex-shrink-0">
      {formatDurationShort(ms)}
    </span>
  );
}

function TreeGuides({ row }: { row: FlatRow }) {
  if (row.depth === 0) return null;
  const cells: React.ReactNode[] = [];
  for (let i = 0; i < row.depth; i++) {
    const isLastAncestor = i === row.depth - 1;
    if (isLastAncestor) {
      cells.push(
        <span key={i} className="relative w-5 flex-shrink-0">
          <span
            className="absolute left-2.5 top-0 w-px bg-[var(--border-subtle)]"
            style={{ height: row.isLast ? "50%" : "100%" }}
          />
          <span className="absolute left-2.5 top-1/2 h-px w-2.5 bg-[var(--border-subtle)]" />
        </span>,
      );
    } else {
      const ancestorIsLast = row.ancestorIsLast[i + 1];
      cells.push(
        <span key={i} className="relative w-5 flex-shrink-0">
          {!ancestorIsLast && (
            <span className="absolute left-2.5 top-0 bottom-0 w-px bg-[var(--border-subtle)]" />
          )}
        </span>,
      );
    }
  }
  return <>{cells}</>;
}

/* ──────────────────────────────────────────────────────────────────────── */
/*                              Rows                                        */
/* ──────────────────────────────────────────────────────────────────────── */

function AgentRow({
  row,
  onToggle,
  onOpenDetails,
  hoveredId,
  setHoveredId,
  sessionId,
  toolsLabel,
  viewDetailsLabel,
}: {
  row: FlatRow;
  onToggle: (id: string) => void;
  onOpenDetails: (parentRequestId: string) => void;
  hoveredId: string | null;
  setHoveredId: (id: string | null) => void;
  sessionId: string;
  toolsLabel: string;
  viewDetailsLabel: string;
}) {
  const node = row.node as AgentTreeAgentNode;
  const Icon = row.depth === 0 ? Bot : getAgentIcon(node.agentName);
  const color = row.depth === 0 ? "var(--accent)" : getAgentColor(node.agentName);
  const isHovered = hoveredId === node.id;
  const isParentHovered = !!row.parentId && hoveredId === row.parentId;
  const isMain = row.depth === 0;

  const handleClick = () => {
    if (!row.hasChildren) return;
    onToggle(node.id);
  };

  const toolCount = useMemo(() => {
    let n = 0;
    const visit = (x: AgentTreeAgentNode | AgentTreeToolNode) => {
      if (x.kind === "tool") n++;
      else for (const c of x.children) visit(c);
    };
    visit(node);
    return n;
  }, [node]);

  return (
    <div
      onMouseEnter={() => setHoveredId(node.id)}
      onMouseLeave={() => setHoveredId(null)}
      className={cn(
        "group flex items-center gap-2 w-full text-left px-2 py-1.5 rounded transition-colors",
        node.status === "running" && "bg-accent/[0.04]",
        isHovered && "bg-accent/10",
        isParentHovered && "bg-accent/5",
      )}
    >
      <TreeGuides row={row} />

      <button
        type="button"
        onClick={handleClick}
        className={cn(
          "flex flex-1 items-center gap-2 min-w-0 text-left",
          row.hasChildren ? "cursor-pointer" : "cursor-default",
        )}
      >
        <span className="w-3 flex-shrink-0 text-muted-foreground/60">
          {row.hasChildren ? (
            row.collapsed ? <ChevronRight className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />
          ) : null}
        </span>

        <StatusGlyph status={node.status} />

        <Icon className="w-3.5 h-3.5 flex-shrink-0" style={{ color }} />

        <span
          className={cn(
            "text-[12px] font-medium truncate",
            node.status === "completed" && "text-foreground/80",
            node.status === "running" && "text-accent",
            node.status === "error" && "text-red-400/90",
            node.status === "interrupted" && "text-amber-400/90",
            node.status === "pending" && "text-muted-foreground/60",
          )}
        >
          {node.agentName}
        </span>

        {!isMain && (
          <AnchorChip sessionId={sessionId} requestId={node.id} anchor={node.anchor ?? undefined} />
        )}

        {node.task && (
          <span className="text-[11px] text-muted-foreground/50 truncate flex-1 min-w-0" title={node.task}>
            {node.task}
          </span>
        )}
        {!node.task && <div className="flex-1" />}

        {row.hasChildren && (
          <span className="text-[10px] text-muted-foreground/55 flex-shrink-0 tabular-nums">
            {toolCount} {toolsLabel}
          </span>
        )}

        <DurationLabel ms={node.durationMs} />
      </button>

      {/* "查看 sub-agent 全部明细" —— 打开 Modal 看 entries/response/工具调用流 */}
      {!isMain && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onOpenDetails(node.id);
          }}
          className="p-1 rounded hover:bg-accent/40 transition-colors flex-shrink-0 opacity-50 hover:opacity-100"
          title={viewDetailsLabel}
        >
          <Maximize2 className="w-3 h-3 text-muted-foreground hover:text-foreground" />
        </button>
      )}
    </div>
  );
}

function ToolRow({
  row,
  hoveredId,
  setHoveredId,
  sessionId,
}: {
  row: FlatRow;
  hoveredId: string | null;
  setHoveredId: (id: string | null) => void;
  sessionId: string;
}) {
  const node = row.node as AgentTreeToolNode;
  const Icon = getToolIcon(node.toolName);
  const color = getToolColor(node.toolName);
  const isParentHovered = !!row.parentId && hoveredId === row.parentId;
  const isHovered = hoveredId === node.id;
  const isShell = node.toolName === "run_pty_cmd" || node.toolName === "run_command";

  return (
    <div
      onMouseEnter={() => setHoveredId(node.id)}
      onMouseLeave={() => setHoveredId(null)}
      className={cn(
        "flex items-center gap-2 w-full px-2 py-1 rounded text-[11px]",
        node.status === "running" && "bg-accent/[0.04]",
        isHovered && "bg-accent/10",
        isParentHovered && "bg-accent/5",
      )}
    >
      <TreeGuides row={row} />
      <span className="w-3 flex-shrink-0" />
      <StatusGlyph status={node.status} />
      <Icon className="w-3 h-3 flex-shrink-0" style={{ color }} />
      <span
        className={cn(
          "truncate font-medium",
          node.status === "pending" && "text-muted-foreground/55",
          node.status === "error" && "text-red-400/85",
          node.status === "running" && "text-foreground/85",
          node.status === "completed" && "text-foreground/75",
          node.status === "interrupted" && "text-amber-400/85",
        )}
      >
        {node.toolName}
      </span>
      <AnchorChip sessionId={sessionId} requestId={node.id} anchor={node.anchor ?? undefined} />
      {node.primary ? (
        <span
          className={cn(
            "truncate font-mono text-[10px] flex-1 min-w-0 px-1.5 py-px rounded",
            isShell ? "bg-[var(--ansi-black)]/30 text-[var(--ansi-green)]/80" : "bg-muted/25 text-muted-foreground/75",
          )}
          title={node.primary}
        >
          {isShell && <span className="text-muted-foreground/50 mr-1">$</span>}
          {node.primary}
        </span>
      ) : (
        <div className="flex-1" />
      )}
      <DurationLabel ms={node.durationMs} />
    </div>
  );
}

/* ──────────────────────────────────────────────────────────────────────── */
/*                                Main                                      */
/* ──────────────────────────────────────────────────────────────────────── */

export const SubAgentTreeView = memo(function SubAgentTreeView({ sessionId }: SubAgentTreeViewProps) {
  const { t } = useTranslation();
  const tree = useAgentTree(sessionId);
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);
  const subAgents = useStore((s) => s.activeSubAgents[sessionId] ?? null);

  const [collapsedIds, setCollapsedIds] = useState<Set<string>>(new Set());
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [modalAgent, setModalAgent] = useState<ActiveSubAgent | null>(null);

  const openDetails = useCallback(
    (parentRequestId: string) => {
      if (!subAgents) return;
      const agent = subAgents.find((a) => a.parentRequestId === parentRequestId) ?? null;
      setModalAgent(agent);
    },
    [subAgents],
  );

  const allAgentIds = useMemo(() => flattenAllAgentIds(tree.root), [tree.root]);
  const allCollapsed = collapsedIds.size === allAgentIds.length && allAgentIds.length > 0;

  const rows = useMemo(() => flatten(tree.root, collapsedIds), [tree.root, collapsedIds]);

  const stats = useMemo(() => {
    let running = 0;
    let errored = 0;
    const visit = (n: AgentTreeAgentNode | AgentTreeToolNode) => {
      if (n.kind === "agent" && n.id !== "__main__") {
        if (n.status === "running") running++;
        if (n.status === "error") errored++;
        for (const c of n.children) visit(c);
      } else if (n.kind === "agent") {
        for (const c of n.children) visit(c);
      }
    };
    visit(tree.root);
    return { running, errored };
  }, [tree.root]);

  const toggle = useCallback((id: string) => {
    setCollapsedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const toggleAll = useCallback(() => {
    if (allCollapsed) setCollapsedIds(new Set());
    else setCollapsedIds(new Set(allAgentIds));
  }, [allCollapsed, allAgentIds]);

  const isEmpty = tree.totalAgents === 0;

  return (
    <div className="h-full flex flex-col bg-card">
      {/* Header */}
      <div className="flex items-center gap-3 px-3 py-2 border-b border-[var(--border-subtle)] flex-shrink-0">
        <button
          type="button"
          onClick={() => setDetailViewMode(sessionId, "timeline")}
          className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          <ArrowLeft className="w-3.5 h-3.5" />
          {t("ai.toolDetail.backToTerminal")}
        </button>
        <div className="flex items-center gap-2 text-[10px] text-muted-foreground/70 ml-2">
          <Bot className="w-3.5 h-3.5 text-accent" />
          <span className="tabular-nums">{tree.totalAgents} {t("ai.agentTree.agents")}</span>
          <span className="text-muted-foreground/30">·</span>
          <span className="tabular-nums">{tree.totalTools} {t("ai.agentTree.tools")}</span>
          {stats.running > 0 && (
            <>
              <span className="text-muted-foreground/30">·</span>
              <span className="text-accent tabular-nums">{stats.running} {t("ai.agentTree.running")}</span>
            </>
          )}
          {stats.errored > 0 && (
            <>
              <span className="text-muted-foreground/30">·</span>
              <span className="text-red-400/90 tabular-nums">{stats.errored} {t("ai.agentTree.failed")}</span>
            </>
          )}
        </div>
        <div className="flex-1" />
        {allAgentIds.length > 0 && (
          <button
            type="button"
            onClick={toggleAll}
            className="flex items-center gap-1 text-[11px] text-muted-foreground/70 hover:text-foreground hover:bg-[var(--bg-hover)] px-2 py-1 rounded transition-colors"
          >
            {allCollapsed ? (
              <>
                <ChevronsUpDown className="w-3 h-3" />
                {t("ai.agentTree.expandAll")}
              </>
            ) : (
              <>
                <ChevronsDownUp className="w-3 h-3" />
                {t("ai.agentTree.collapseAll")}
              </>
            )}
          </button>
        )}
      </div>

      {/* Tree */}
      <div className="flex-1 overflow-y-auto px-2 py-2">
        {isEmpty ? (
          <div className="h-full flex items-center justify-center text-[12px] text-muted-foreground/50">
            {t("ai.agentTree.empty")}
          </div>
        ) : (
          rows.map((row, i) => (
            <Fragment key={`${row.node.id}-${i}`}>
              {row.node.kind === "agent" ? (
                <AgentRow
                  row={row}
                  onToggle={toggle}
                  onOpenDetails={openDetails}
                  hoveredId={hoveredId}
                  setHoveredId={setHoveredId}
                  sessionId={sessionId}
                  toolsLabel={t("ai.agentTree.tools")}
                  viewDetailsLabel={t("ai.agentTree.viewDetails")}
                />
              ) : (
                <ToolRow
                  row={row}
                  hoveredId={hoveredId}
                  setHoveredId={setHoveredId}
                  sessionId={sessionId}
                />
              )}
            </Fragment>
          ))
        )}
      </div>

      {/* Footer */}
      {stats.running > 0 && (
        <div className="px-3 py-2 border-t border-[var(--border-subtle)] bg-accent/5 flex items-center gap-2 flex-shrink-0">
          <Loader2 className="w-3 h-3 text-accent animate-spin" />
          <span className="text-[11px] text-accent/80 tabular-nums">
            {t("ai.agentTree.agentsRunning", { count: stats.running })}
          </span>
        </div>
      )}

      {modalAgent && (
        <SubAgentDetailsModal
          subAgent={modalAgent}
          onClose={() => setModalAgent(null)}
        />
      )}
    </div>
  );
});
