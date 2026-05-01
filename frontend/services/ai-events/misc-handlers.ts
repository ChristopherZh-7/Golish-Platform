/**
 * Miscellaneous AI event handlers.
 *
 * Handles: plan_updated, server_tool_started, web_search_result, web_fetch_result.
 */

import { logger } from "@/lib/logger";
import type { EventHandler } from "./types";

/**
 * Handle plan updated event.
 * Updates the task plan for a session.
 */
export const handlePlanUpdated: EventHandler<{
  type: "plan_updated";
  version: number;
  summary: { total: number; completed: number; in_progress: number; pending: number };
  steps: Array<{ id?: string; step: string; status: "pending" | "in_progress" | "completed" | "cancelled" | "failed" }>;
  explanation: string | null;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const plan = {
    version: event.version,
    summary: event.summary,
    steps: event.steps.map((s) => ({
      id: s.id,
      step: s.step,
      status: s.status,
    })),
    explanation: event.explanation,
    updated_at: new Date().toISOString(),
  };
  const state = ctx.getState();
  state.setPlan(ctx.sessionId, plan);
};

/**
 * Handle server tool started event.
 * Logs server tool start for debugging.
 */
export const handleServerToolStarted: EventHandler<{
  type: "server_tool_started";
  request_id: string;
  tool_name: string;
  input: unknown;
  session_id: string;
  seq?: number;
}> = (event, _ctx) => {
  logger.info(`[Server Tool] ${event.tool_name} started (${event.request_id})`);
};

/**
 * Handle web search result event.
 * Logs web search results for debugging.
 */
export const handleWebSearchResult: EventHandler<{
  type: "web_search_result";
  request_id: string;
  results: unknown;
  session_id: string;
  seq?: number;
}> = (event, _ctx) => {
  logger.info(`[Server Tool] Web search completed (${event.request_id}):`, event.results);
};

/**
 * Handle web fetch result event.
 * Logs web fetch results for debugging.
 */
export const handleWebFetchResult: EventHandler<{
  type: "web_fetch_result";
  request_id: string;
  url: string;
  content_preview: string;
  session_id: string;
  seq?: number;
}> = (event, _ctx) => {
  logger.info(
    `[Server Tool] Web fetch completed for ${event.url} (${event.request_id}):`,
    event.content_preview
  );
};
