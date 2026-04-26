import type { ToolSource } from "@/lib/ai/types";
import type { ToolCallSource } from "@/store";

/** Convert AI event source to store source (snake_case to camelCase) */
export function convertToolSource(source?: ToolSource): ToolCallSource | undefined {
  if (!source) return undefined;
  if (source.type === "main") return { type: "main" };
  if (source.type === "sub_agent") {
    return {
      type: "sub_agent",
      agentId: source.agent_id,
      agentName: source.agent_name,
    };
  }
  if (source.type === "workflow") {
    return {
      type: "workflow",
      workflowId: source.workflow_id,
      workflowName: source.workflow_name,
      stepName: source.step_name,
      stepIndex: source.step_index,
    };
  }
  return undefined;
}
