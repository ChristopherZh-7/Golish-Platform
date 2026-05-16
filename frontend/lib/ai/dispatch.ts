/**
 * Sub-agent dispatch queries.
 *
 * Backed by the Tauri command `list_running_sub_agent_dispatches`
 * (P0-4). Reads the lifecycle table populated by every
 * `execute_sub_agent_with_client` call and tagged with the
 * stale-cleanup reaper on every read.
 *
 * Writes are intentionally NOT exposed — the agent runtime owns the
 * lifecycle (`record_start` / `record_finish`).
 */

import { invoke } from "@/lib/api/client";

export interface RunningSubAgentDispatch {
  id: string;
  parent_dispatch_id: string | null;
  agent_id: string;
  tool_call_id: string | null;
  depth: number;
  args: Record<string, unknown>;
  started_at: string;
}

/**
 * Return every dispatch row tagged `running` for the given session.
 * Stale rows (>24h since started_at) are auto-cancelled by the
 * backend on each call.
 */
export async function listRunningSubAgentDispatches(
  sessionId: string
): Promise<RunningSubAgentDispatch[]> {
  return invoke("list_running_sub_agent_dispatches", { sessionId });
}
