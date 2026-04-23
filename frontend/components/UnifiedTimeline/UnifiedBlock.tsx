import { memo, useCallback } from "react";
import { CommandBlock } from "@/components/CommandBlock/CommandBlock";
import type { UnifiedBlock as UnifiedBlockType } from "@/store";
import { useStore } from "@/store";

interface UnifiedBlockProps {
  block: UnifiedBlockType;
  sessionId: string;
  workingDirectory: string;
}

const getToggleBlockCollapse = () => useStore.getState().toggleBlockCollapse;

export const UnifiedBlock = memo(function UnifiedBlock({ block, sessionId }: UnifiedBlockProps) {
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

  return null;
});
