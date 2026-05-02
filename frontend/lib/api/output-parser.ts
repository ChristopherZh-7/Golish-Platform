/**
 * Tool output parsing IPC wrappers.
 *
 * Backed by the Tauri commands `output_detect_tool` and
 * `output_parse` (see commands_facade::workspace). The richer
 * `output_parse_and_store` (which writes back to the DB) lives in
 * `lib/pentest/api.ts` because it's tightly coupled to the
 * Tool Manager flow.
 */

import { invoke } from "./client";

export interface OutputConfig {
  format: string;
  produces: string[];
  patterns: unknown[];
  fields: Record<string, string>;
  detect?: string;
}

export interface DetectedTool {
  tool_id: string;
  tool_name: string;
  output_config: OutputConfig;
}

export interface ParsedItem {
  data_type: string;
  fields: Record<string, string>;
}

/**
 * Detect which tool generated a given (command, output) pair, returning
 * its parser config or null if no signature matched.
 */
export async function detectTool(command: string, rawOutput: string): Promise<DetectedTool | null> {
  return invoke<DetectedTool | null>("output_detect_tool", { command, rawOutput });
}

/**
 * Parse raw tool output into structured items using the supplied config.
 */
export async function parse(args: {
  rawOutput: string;
  config: OutputConfig;
  toolId: string;
  toolName: string;
}): Promise<{ items: ParsedItem[] }> {
  return invoke<{ items: ParsedItem[] }>(
    "output_parse",
    args as unknown as Record<string, unknown>
  );
}
