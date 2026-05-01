/**
 * UI-State Layer — view models that transform service data
 * into shapes optimized for React rendering and Zustand consumption.
 *
 * Usage:
 *   import { buildToolTree, deriveAgentStatus } from "@/lib/ui-state";
 */

export {
  type AgentActivityStatus,
  type TokenUsageSummary,
  computeTokenUsage,
  deriveAgentStatus,
} from "./ai.viewmodel";
export {
  type ToolSummary,
  type ToolTreeNode,
  buildToolTree,
  computeToolSummary,
} from "./pentest.viewmodel";
export {
  type ProviderCard,
  countConfiguredProviders,
  deriveProviderCards,
} from "./settings.viewmodel";
