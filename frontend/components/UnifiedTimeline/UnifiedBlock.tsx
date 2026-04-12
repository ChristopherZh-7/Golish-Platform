import { memo, useCallback } from "react";
import { CommandBlock } from "@/components/CommandBlock/CommandBlock";
import { PipelineProgressBlock } from "@/components/PipelineProgressBlock";
import { SubAgentCard, SubAgentGroup } from "@/components/SubAgentCard";
import { ToolExecutionCard } from "@/components/ToolExecutionCard";
import type { UnifiedBlock as UnifiedBlockType, ActiveSubAgent } from "@/store";
import { useStore } from "@/store";

interface UnifiedBlockProps {
  block: UnifiedBlockType;
  sessionId: string;
  workingDirectory: string;
  /** Adjacent sub-agent blocks to group together (passed by timeline renderer) */
  groupedAgents?: ActiveSubAgent[];
}

const getToggleBlockCollapse = () => useStore.getState().toggleBlockCollapse;

export const UnifiedBlock = memo(function UnifiedBlock({ block, sessionId, groupedAgents }: UnifiedBlockProps) {
  const toggleBlockCollapse = useCallback(
    (blockId: string) => getToggleBlockCollapse()(blockId),
    []
  );

  if (block.type === "command") {
    return (
      <CommandBlock
        block={block.data}
        sessionId={sessionId}
        onToggleCollapse={toggleBlockCollapse}
        source={block.data.source}
      />
    );
  }

  if (block.type === "pipeline_progress") {
    return <PipelineProgressBlock execution={block.data} />;
  }

  if (block.type === "sub_agent_activity") {
    if (groupedAgents && groupedAgents.length > 1) {
      return <SubAgentGroup agents={groupedAgents} />;
    }
    return <SubAgentCard subAgent={block.data} />;
  }

  if (block.type === "ai_tool_execution") {
    return <ToolExecutionCard execution={block.data} />;
  }

  return null;
});
