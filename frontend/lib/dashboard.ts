import { invoke } from "@/lib/api/client";
import type { Target as PentestTarget } from "@/lib/pentest/types";

export interface TokenUsageStats {
  total_tokens_in: number;
  total_tokens_out: number;
  total_cost_in: number;
  total_cost_out: number;
}

export interface AgentUsage {
  agent: string;
  total_tokens_in: number;
  total_tokens_out: number;
  total_cost: number;
}

export interface ToolCallStat {
  name: string;
  total_count: number;
  total_duration_ms: number;
  avg_duration_ms: number;
}

export interface TargetStore {
  targets: PentestTarget[];
}

export interface ProjectMethodology {
  id: string;
  project_name: string;
  template_id: string;
  phases: Array<{ id: string; name: string; items: Array<{ id: string; checked: boolean }> }>;
  created_at: string;
  updated_at: string;
}

export interface VaultEntry {
  id: string;
  name: string;
  entry_type: string;
}

export interface AuditEntry {
  id: number;
  action: string;
  category: string;
  details: string;
  source: string;
  status: string;
  createdAt: number;
  targetId?: string | null;
  entityType?: string | null;
}

export const dashboardApi = {
  // AI stats
  getTokenUsageStats: () => invoke<TokenUsageStats>("get_db_token_usage_stats"),
  getUsageByAgent: () => invoke<AgentUsage[]>("get_usage_by_agent"),
  getToolCallStats: () => invoke<ToolCallStat[]>("get_tool_call_stats", {}),
  getMemoryCount: () => invoke<number>("get_memory_count"),

  // Project data
  targetList: (projectPath: string) => invoke<TargetStore>("target_list", { projectPath }),
  methodListProjects: (projectPath: string) =>
    invoke<ProjectMethodology[]>("method_list_projects", { projectPath }),
  vaultList: (projectPath: string) => invoke<VaultEntry[]>("vault_list", { projectPath }),
  oplogList: (projectPath: string, limit: number) =>
    invoke<AuditEntry[]>("oplog_list", { projectPath, limit }),
};
