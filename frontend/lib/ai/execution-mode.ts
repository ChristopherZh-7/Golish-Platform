import { invoke } from "@/lib/api/client";

/**
 * Descriptor for one execution mode policy registered on the backend.
 *
 * Field names match the camelCase wire format produced by the Tauri
 * `list_execution_modes` command (see
 * `backend/crates/golish/src/ai/commands/mode.rs::ExecutionModeDescriptor`,
 * which uses `#[serde(rename_all = "camelCase")]`).
 */
export interface ExecutionModeDescriptor {
  /** Stable lookup id, e.g. "chat" / "task" / future "plan". */
  id: string;
  /** Human-readable label rendered in the picker. */
  displayName: string;
  /**
   * Lucide icon name. The picker maps known names to React components
   * and falls back to `MessageSquare` for unknown values, so adding a
   * new mode with an unmapped icon string is non-fatal.
   */
  icon: string;
  /**
   * Free-form CSS theme key (e.g. `"muted"`, `"magenta"`). The picker
   * applies a small allow-list of background classes; unknown values
   * gracefully fall back to the default neutral style.
   */
  badgeColor: string;
  /** Tooltip / help text shown under the option. */
  description: string;
  /**
   * `true` if the mode allows the LLM to dispatch sub-agents. Used by
   * the picker to enable / disable the "Sub-Agents" toggle row.
   */
  allowsSubAgents: boolean;
}

/**
 * Fetch the list of execution modes registered on the backend.
 *
 * The call hits the cheap `list_execution_modes` Tauri command which
 * iterates `ExecutionModeRegistry::default()` synchronously, so it is
 * safe to invoke on mount without debouncing.
 */
export async function listExecutionModes(): Promise<ExecutionModeDescriptor[]> {
  return invoke<ExecutionModeDescriptor[]>("list_execution_modes");
}
