import type { AgentMessage, AiToolExecution, CommandBlock } from "./message";
import type { ActiveSubAgent } from "./sub-agent";
import type { ToolCall } from "./tool-call";

export type UnifiedBlock =
  | {
      id: string;
      type: "command";
      timestamp: string;
      data: CommandBlock & { source?: "manual" };
    }
  | {
      id: string;
      type: "agent_message";
      timestamp: string;
      data: AgentMessage;
    }
  | {
      id: string;
      type: "system_hook";
      timestamp: string;
      data: { hooks: string[] };
    }
  | {
      id: string;
      type: "agent_streaming";
      timestamp: string;
      data: { content: string; toolCalls?: ToolCall[] };
    }
  | {
      id: string;
      type: "sub_agent_activity";
      timestamp: string;
      data: ActiveSubAgent;
      planStepIndex?: number;
      batchId?: string;
    }
  | {
      id: string;
      type: "ai_tool_execution";
      timestamp: string;
      data: AiToolExecution;
    };
