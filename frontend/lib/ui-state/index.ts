/**
 * UI-State Layer — view models that transform service data
 * into shapes optimized for React rendering and Zustand consumption.
 *
 * Usage:
 *   import { buildToolTree, deriveAgentStatus } from "@/lib/ui-state";
 */

export {
  type AgentActivityStatus,
  computeTokenUsage,
  deriveAgentStatus,
  type TokenUsageSummary,
} from "./ai.viewmodel";
export {
  buildToolTree,
  computeToolSummary,
  type ToolSummary,
  type ToolTreeNode,
} from "./pentest.viewmodel";
export {
  countConfiguredProviders,
  deriveProviderCards,
  type ProviderCard,
} from "./settings.viewmodel";
