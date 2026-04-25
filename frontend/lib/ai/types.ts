import type { RiskLevel } from "../tools";

export type { RiskLevel };

export type AiProvider =
  | "vertex_ai"
  | "vertex_gemini"
  | "openrouter"
  | "openai"
  | "anthropic"
  | "ollama"
  | "gemini"
  | "groq"
  | "xai"
  | "zai_sdk"
  | "nvidia";

export interface ProjectSettings {
  provider: AiProvider | null;
  model: string | null;
  agent_mode: AgentMode | null;
}

export interface AiConfig {
  workspace: string;
  provider: AiProvider;
  model: string;
  apiKey: string;
}

export type ProviderConfig =
  | {
      provider: "vertex_ai";
      workspace: string;
      model: string;
      credentials_path: string | null;
      project_id: string;
      location: string;
    }
  | {
      provider: "vertex_gemini";
      workspace: string;
      model: string;
      credentials_path: string | null;
      project_id: string;
      location: string;
    }
  | {
      provider: "openrouter";
      workspace: string;
      model: string;
      api_key: string;
      provider_preferences?: Record<string, unknown>;
    }
  | {
      provider: "openai";
      workspace: string;
      model: string;
      api_key: string;
      base_url?: string;
      reasoning_effort?: string;
    }
  | {
      provider: "anthropic";
      workspace: string;
      model: string;
      api_key: string;
    }
  | {
      provider: "ollama";
      workspace: string;
      model: string;
      base_url?: string;
    }
  | {
      provider: "gemini";
      workspace: string;
      model: string;
      api_key: string;
    }
  | {
      provider: "groq";
      workspace: string;
      model: string;
      api_key: string;
    }
  | {
      provider: "xai";
      workspace: string;
      model: string;
      api_key: string;
    }
  | {
      provider: "zai_sdk";
      workspace: string;
      model: string;
      api_key: string;
      base_url?: string;
    }
  | {
      provider: "nvidia";
      workspace: string;
      model: string;
      api_key: string;
      base_url?: string;
    };

export interface ApprovalPattern {
  tool_name: string;
  total_requests: number;
  approvals: number;
  denials: number;
  always_allow: boolean;
  last_updated: string;
  justifications: string[];
}

export interface ProviderRequestStats {
  requests: number;
  last_sent_at: number | null;
  last_received_at: number | null;
}

export interface ApiRequestStatsSnapshot {
  providers: Record<string, ProviderRequestStats>;
}

export type ToolSource =
  | { type: "main" }
  | { type: "sub_agent"; agent_id: string; agent_name: string }
  | {
      type: "workflow";
      workflow_id: string;
      workflow_name: string;
      step_name?: string;
      step_index?: number;
    };

interface AiEventBase {
  session_id: string;
  seq?: number;
  ts?: string;
}

export type AiEvent = AiEventBase &
  (
    | { type: "started"; turn_id: string }
    | { type: "system_hooks_injected"; hooks: string[] }
    | { type: "text_delta"; delta: string; accumulated: string }
    | {
        type: "tool_request";
        tool_name: string;
        args: unknown;
        request_id: string;
        source?: ToolSource;
      }
    | {
        type: "tool_approval_request";
        request_id: string;
        tool_name: string;
        args: unknown;
        stats: ApprovalPattern | null;
        risk_level: RiskLevel;
        can_learn: boolean;
        suggestion: string | null;
        source?: ToolSource;
      }
    | {
        type: "tool_auto_approved";
        request_id: string;
        tool_name: string;
        args: unknown;
        reason: string;
        source?: ToolSource;
      }
    | {
        type: "tool_result";
        tool_name: string;
        result: unknown;
        success: boolean;
        request_id: string;
        source?: ToolSource;
      }
    | {
        type: "tool_output_chunk";
        request_id: string;
        tool_name: string;
        chunk: string;
        stream: string;
        source?: ToolSource;
      }
    | { type: "reasoning"; content: string }
    | {
        type: "completed";
        response: string;
        reasoning?: string;
        input_tokens?: number;
        output_tokens?: number;
        duration_ms?: number;
      }
    | { type: "error"; message: string; error_type: string }
    | {
        type: "sub_agent_started";
        agent_id: string;
        agent_name: string;
        task: string;
        depth: number;
        parent_request_id: string;
      }
    | {
        type: "sub_agent_tool_request";
        agent_id: string;
        tool_name: string;
        args: unknown;
        request_id: string;
        parent_request_id: string;
      }
    | {
        type: "sub_agent_tool_result";
        agent_id: string;
        tool_name: string;
        success: boolean;
        result: unknown;
        request_id: string;
        parent_request_id: string;
      }
    | {
        type: "sub_agent_completed";
        agent_id: string;
        response: string;
        duration_ms: number;
        parent_request_id: string;
      }
    | {
        type: "sub_agent_text_delta";
        agent_id: string;
        delta: string;
        accumulated: string;
        parent_request_id: string;
      }
    | {
        type: "sub_agent_error";
        agent_id: string;
        error: string;
        parent_request_id: string;
      }
    | {
        type: "prompt_generation_started";
        agent_id: string;
        parent_request_id: string;
        architect_system_prompt: string;
        architect_user_message: string;
      }
    | {
        type: "prompt_generation_completed";
        agent_id: string;
        parent_request_id: string;
        generated_prompt?: string;
        success: boolean;
        duration_ms: number;
      }
    | {
        type: "workflow_started";
        workflow_id: string;
        workflow_name: string;
        session_id: string;
      }
    | {
        type: "workflow_step_started";
        workflow_id: string;
        step_name: string;
        step_index: number;
        total_steps: number;
      }
    | {
        type: "workflow_step_completed";
        workflow_id: string;
        step_name: string;
        output: string | null;
        duration_ms: number;
      }
    | {
        type: "workflow_completed";
        workflow_id: string;
        final_output: string;
        total_duration_ms: number;
      }
    | {
        type: "workflow_error";
        workflow_id: string;
        step_name: string | null;
        error: string;
      }
    | {
        type: "plan_updated";
        version: number;
        summary: {
          total: number;
          completed: number;
          in_progress: number;
          pending: number;
        };
        steps: Array<{
          id?: string;
          step: string;
          status: "pending" | "in_progress" | "completed" | "cancelled" | "failed";
        }>;
        explanation: string | null;
      }
    | {
        type: "context_warning";
        utilization: number;
        total_tokens: number;
        max_tokens: number;
      }
    | {
        type: "compaction_started";
        tokens_before: number;
        messages_before: number;
      }
    | {
        type: "compaction_completed";
        tokens_before: number;
        messages_before: number;
        messages_after: number;
        summary_length: number;
        summary?: string;
        summarizer_input?: string;
      }
    | {
        type: "compaction_failed";
        tokens_before: number;
        messages_before: number;
        error: string;
        summarizer_input?: string;
      }
    | {
        type: "tool_response_truncated";
        tool_name: string;
        original_tokens: number;
        truncated_tokens: number;
      }
    | {
        type: "server_tool_started";
        request_id: string;
        tool_name: string;
        input: unknown;
      }
    | {
        type: "web_search_result";
        request_id: string;
        results: unknown;
      }
    | {
        type: "web_fetch_result";
        request_id: string;
        url: string;
        content_preview: string;
      }
    | {
        type: "warning";
        message: string;
      }
    | {
        type: "ask_human_request";
        request_id: string;
        question: string;
        input_type: string;
        options: string[];
        context: string;
      }
    | {
        type: "ask_human_response";
        request_id: string;
        response: string;
        skipped: boolean;
      }
    | {
        type: "task_progress";
        task_id: string;
        status: string;
        message: string;
      }
    | {
        type: "subtask_created";
        task_id: string;
        subtask_id: string;
        title: string;
        agent: string | null;
      }
    | {
        type: "subtask_completed";
        task_id: string;
        subtask_id: string;
        title: string;
        result: string;
      }
    | {
        type: "subtask_waiting_for_input";
        task_id: string;
        subtask_id: string;
        title: string;
        prompt: string;
      }
    | {
        type: "subtask_user_input";
        task_id: string;
        subtask_id: string;
        input: string;
      }
    | {
        type: "task_resumed";
        task_id: string;
        subtask_index: number;
        total_subtasks: number;
      }
    | {
        type: "enricher_result";
        task_id: string;
        subtask_id: string;
        context_added: string;
      }
  );

export interface ToolDefinition {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
}

export interface WorkflowInfo {
  name: string;
  description: string;
}

export interface SubAgentInfo {
  id: string;
  name: string;
  description: string;
  model_override: [string, string] | null;
}

export interface SubAgentModelOverride {
  provider: AiProvider;
  model: string;
}

export interface SessionAiConfigInfo {
  provider: string;
  model: string;
}

export interface ToolStatus {
  name: string;
  installed: boolean;
}

export interface ReconToolCheck {
  tools: ToolStatus[];
  all_ready: boolean;
  missing: string[];
}

export interface VertexAiEnvConfig {
  credentials_path: string | null;
  project_id: string | null;
  location: string | null;
}

export interface VertexAiConfig {
  workspace: string;
  credentialsPath: string;
  projectId: string;
  location: string;
  model: string;
}

export interface OpenAiConfig {
  workspace: string;
  model: string;
  apiKey: string;
  baseUrl?: string;
  reasoningEffort?: ReasoningEffort;
}

export type ReasoningEffort = "low" | "medium" | "high" | "extra_high";

export type AgentMode = "default" | "auto-approve" | "planning";

export type SessionMessageRole = "user" | "assistant" | "system" | "tool";

export interface SessionMessage {
  role: SessionMessageRole;
  content: string;
  tool_call_id?: string;
  tool_name?: string;
}

export interface SessionListingInfo {
  identifier: string;
  path: string;
  workspace_label: string;
  workspace_path: string;
  model: string;
  provider: string;
  started_at: string;
  ended_at: string;
  total_messages: number;
  distinct_tools: string[];
  first_prompt_preview?: string;
  first_reply_preview?: string;
  status?: "active" | "completed" | "abandoned";
  title?: string;
}

export interface SessionSnapshot {
  workspace_label: string;
  workspace_path: string;
  model: string;
  provider: string;
  started_at: string;
  ended_at: string;
  total_messages: number;
  distinct_tools: string[];
  transcript: string[];
  messages: SessionMessage[];
  agent_mode?: string;
}

export interface ToolApprovalConfig {
  always_allow: string[];
  always_require_approval: string[];
  pattern_learning_enabled: boolean;
  min_approvals: number;
  approval_threshold: number;
}

export interface ApprovalDecision {
  request_id: string;
  approved: boolean;
  reason?: string;
  remember: boolean;
  always_allow: boolean;
}

export interface TaskPlan {
  explanation: string | null;
  steps: Array<{
    step: string;
    status: "pending" | "in_progress" | "completed" | "cancelled" | "failed";
  }>;
  summary: {
    total: number;
    completed: number;
    in_progress: number;
    pending: number;
  };
  version: number;
  updated_at: string;
}

export interface VisionCapabilities {
  supports_vision: boolean;
  max_image_size_bytes: number;
  supported_formats: string[];
}

export interface TextPart {
  type: "text";
  text: string;
}

export interface ImagePart {
  type: "image";
  data: string;
  media_type?: string;
  filename?: string;
}

export type PromptPart = TextPart | ImagePart;

export interface PromptPayload {
  parts: PromptPart[];
}

export interface CommitMessageResponse {
  summary: string;
  description: string;
}

export interface ToolCallStats {
  name: string;
  total_count: number;
  total_duration_ms: number;
  avg_duration_ms: number;
}

export interface DbTokenUsageStats {
  total_tokens_in: number;
  total_tokens_out: number;
  total_cost_in: number;
  total_cost_out: number;
}

export interface AuditEntry {
  id: number;
  action: string;
  category: string;
  details: string;
  entity_type: string | null;
  entity_id: string | null;
  project_path: string | null;
  created_at: string;
}

export interface MemoryEntry {
  id: string;
  content: string;
  mem_type: string;
  metadata: Record<string, unknown> | null;
  created_at: string;
}

export interface AgentFileInfo {
  id: string;
  name: string;
  description: string;
  path: string;
  source: "built-in" | "file";
  scope: "global" | "project" | "built-in";
  is_system: boolean;
  model: string | null;
  allowed_tools: string[];
  max_iterations: number;
  timeout_secs: number | null;
  idle_timeout_secs: number | null;
  readonly: boolean;
  is_background: boolean;
  temperature: number | null;
  max_tokens: number | null;
  top_p: number | null;
}

export interface SkillInfo {
  name: string;
  path: string;
  source: "global" | "project";
  description: string;
  license: string | null;
  compatibility: string | null;
  metadata: Record<string, string> | null;
  allowed_tools: string[] | null;
  has_scripts: boolean;
  has_references: boolean;
  has_assets: boolean;
}

export interface RuleInfo {
  name: string;
  path: string;
  source: "global" | "project";
  description: string;
  globs: string | null;
  always_apply: boolean;
}

export const DEFAULT_AI_CONFIG = {
  provider: "openrouter" as AiProvider,
  model: "anthropic/claude-opus-4.5",
};
