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

/**
 * Minimal vault projection used by the dashboard summary.
 *
 * Mirrors the `id` / `name` / `type` fields of the Rust `VaultEntrySafe`
 * struct (`golish-core/src/vault.rs`) over IPC. The `type` enum string
 * literals are kept aligned with `golish-core/src/vault.rs::VaultEntryType`
 * — when the Rust enum gains a variant, add it here too.
 */
type VaultEntry = {
  id: string;
  name: string;
  type: "password" | "token" | "ssh_key" | "api_key" | "cookie" | "certificate" | "other";
};

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
  vaultList: (projectPath: string) => invoke<VaultEntry[]>("vault_list", { projectPath }),
  oplogList: (projectPath: string, limit: number) =>
    invoke<AuditEntry[]>("oplog_list", { projectPath, limit }),
};
