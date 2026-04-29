/**
 * SubAgentInlineCard
 *
 * 在 ChatPanel 的消息流中渲染一个 "已委托给 X" 卡，替代之前 `MessageBlock`
 * 里 `seg.kind === "sub_agent"` 直接 `return null` 的隐藏行为。
 *
 * 设计取自 ChatPanel.mock.tsx 的 SubAgentInlineCard：
 *   - 一个紧凑的卡片，左边状态图标 + agent 头像
 *   - 中间 agent 名 + AnchorChip + task 摘要
 *   - 右边 Duration + tool count + "详情 →"
 *   - 点击：跳到 sub-agent-detail 模式，展示该单个 sub-agent 的完整详情
 */
import { ChevronRight } from "lucide-react";
import { memo, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { AnchorChip } from "@/components/ui/AnchorChip";
import { getAgentColor, getAgentIcon } from "@/lib/sub-agent-theme";
import { formatDurationShort } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent } from "@/store";
import { useStore } from "@/store";
import type { ChatToolCall } from "@/store/slices/conversation";

interface SubAgentInlineCardProps {
  /** sub_agent_* tool call 的 requestId，用来锚点查询 + store 查找 */
  requestId: string;
  /** 原始 tool call —— 当 activeSubAgents 还没装上时用于 fallback */
  toolCall: ChatToolCall;
  /** 当前会话 id（必须） */
  sessionId?: string | null;
}

interface ResolvedSubAgent {
  agentId: string;
  agentName: string;
  task: string;
  status: ActiveSubAgent["status"];
  durationMs?: number;
  toolCount: number;
  error?: string;
}

const EMPTY_AGENTS: ActiveSubAgent[] = [];

function deriveFallback(tc: ChatToolCall): ResolvedSubAgent {
  const fallbackName = tc.name.replace(/^sub_agent_/, "") || "subagent";
  let task = "";
  if (tc.args) {
    try {
      const parsed = JSON.parse(tc.args);
      if (typeof parsed?.task === "string") task = parsed.task;
      else if (typeof parsed?.prompt === "string") task = parsed.prompt;
      else if (typeof parsed?.user_message === "string") task = parsed.user_message;
    } catch {
      /* tc.args may be partial during streaming */
    }
  }
  // tc.success: true => completed, false => error, undefined => running
  const status: ActiveSubAgent["status"] =
    tc.success === true ? "completed" : tc.success === false ? "error" : "running";
  return {
    agentId: fallbackName,
    agentName: fallbackName,
    task,
    status,
    toolCount: 0,
  };
}

function useResolvedSubAgent(
  sessionId: string | null | undefined,
  requestId: string,
  tc: ChatToolCall,
): ResolvedSubAgent {
  const activeAgent = useStore((s) => {
    if (!sessionId) return null;
    // sessionId may be aiSessionId; also check terminal session
    let list = s.activeSubAgents[sessionId] ?? EMPTY_AGENTS;
    let found = list.find((a) => a.parentRequestId === requestId) ?? null;
    if (!found) {
      const conv = s.getConversationBySessionId(sessionId);
      if (conv) {
        const termId = s.conversationTerminals[conv.id]?.[0];
        if (termId) {
          list = s.activeSubAgents[termId] ?? EMPTY_AGENTS;
          found = list.find((a) => a.parentRequestId === requestId) ?? null;
        }
      }
    }
    return found;
  });

  return useMemo(() => {
    if (activeAgent) {
      return {
        agentId: activeAgent.agentId,
        agentName: activeAgent.agentName || activeAgent.agentId,
        task: activeAgent.task ?? "",
        status: activeAgent.status,
        durationMs: activeAgent.durationMs,
        toolCount: activeAgent.toolCalls?.length ?? 0,
        error: activeAgent.error,
      };
    }
    return deriveFallback(tc);
  }, [activeAgent, tc]);
}

function StatusGlyph({ status }: { status: ActiveSubAgent["status"] }) {
  if (status === "running") {
    return (
      <svg
        className="w-3 h-3 text-accent animate-spin flex-shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
      >
        <path d="M21 12a9 9 0 1 1-6.219-8.56" strokeLinecap="round" />
      </svg>
    );
  }
  if (status === "completed") {
    return (
      <svg
        className="w-3 h-3 text-green-500 flex-shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
      >
        <path d="M20 6L9 17l-5-5" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  }
  if (status === "error") {
    return (
      <svg
        className="w-3 h-3 text-red-400 flex-shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
      >
        <circle cx="12" cy="12" r="9" />
        <path d="M15 9l-6 6M9 9l6 6" strokeLinecap="round" />
      </svg>
    );
  }
  // interrupted
  return (
    <svg
      className="w-3 h-3 text-amber-400 flex-shrink-0"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.5"
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M12 8v5M12 16h.01" strokeLinecap="round" />
    </svg>
  );
}

export const SubAgentInlineCard = memo(function SubAgentInlineCard({
  requestId,
  toolCall,
  sessionId,
}: SubAgentInlineCardProps) {
  const { t } = useTranslation();
  const resolved = useResolvedSubAgent(sessionId, requestId, toolCall);
  const Icon = getAgentIcon(resolved.agentName);
  const color = getAgentColor(resolved.agentName);

  const handleClick = () => {
    if (!sessionId) return;
    const store = useStore.getState();
    // sessionId may be the conversation's aiSessionId which differs from the
    // terminal session ID used by PaneLeaf. Resolve to the terminal session.
    let targetSid = sessionId;
    if (!store.sessions[targetSid]) {
      const conv = store.getConversationBySessionId(targetSid);
      if (conv) {
        const termId = store.conversationTerminals[conv.id]?.[0];
        if (termId && store.sessions[termId]) targetSid = termId;
      }
    }
    if (!targetSid || !store.sessions[targetSid]) {
      targetSid = store.activeSessionId ?? sessionId;
    }
    store.setToolDetailRequestIds(targetSid, [requestId]);
    store.setDetailViewMode(targetSid, "sub-agent-detail");
  };

  return (
    <button
      type="button"
      onClick={handleClick}
      className={cn(
        "rounded-lg border bg-background/50 p-2.5 transition-colors hover:border-accent/40 cursor-pointer group text-left w-full",
        resolved.status === "running" && "border-l-2 animate-[pulse-border_2s_ease-in-out_infinite]",
        resolved.status === "error" && "border-red-500/30",
        resolved.status === "completed" && "border-border/30",
        resolved.status === "interrupted" && "border-amber-500/30",
      )}
      style={resolved.status === "running" ? { borderLeftColor: color } : undefined}
    >
      <div className="flex items-center gap-2">
        <StatusGlyph status={resolved.status} />
        <Icon className="w-3.5 h-3.5 flex-shrink-0" style={{ color }} />
        <span className="text-[11px] text-muted-foreground/65">{t("ai.subAgent.delegateTo")}</span>
        <span
          className={cn(
            "text-[12px] font-semibold truncate",
            resolved.status === "completed" && "text-foreground/85",
            resolved.status === "running" && "text-accent",
            resolved.status === "error" && "text-red-400/90",
            resolved.status === "interrupted" && "text-amber-400/90",
          )}
        >
          {resolved.agentName}
        </span>
        <AnchorChip sessionId={sessionId} requestId={requestId} />
        <div className="flex-1" />
        {resolved.durationMs != null && (
          <span className="text-[10px] text-muted-foreground/60 tabular-nums flex-shrink-0">
            {formatDurationShort(resolved.durationMs)}
          </span>
        )}
        <span className="text-[10px] text-muted-foreground/55 group-hover:text-accent/70 transition-colors flex items-center gap-0.5 flex-shrink-0">
          {t("ai.subAgent.viewMore")} <ChevronRight className="w-2.5 h-2.5" />
        </span>
      </div>

      {resolved.task && (
        <div
          className="mt-1.5 ml-5 text-[11px] text-muted-foreground/70 truncate"
          title={resolved.task}
        >
          {resolved.task}
        </div>
      )}

      {(resolved.toolCount > 0 || resolved.error) && (
        <div className="mt-1 ml-5 flex items-center gap-2 text-[10px] text-muted-foreground/55">
          {resolved.toolCount > 0 && (
            <span className="tabular-nums">
              {resolved.toolCount} {t("ai.agentTree.tools")}
            </span>
          )}
          {resolved.error && resolved.toolCount > 0 && <span>·</span>}
          {resolved.error && (
            <span className="text-red-400/70 truncate">{resolved.error}</span>
          )}
        </div>
      )}
    </button>
  );
});
