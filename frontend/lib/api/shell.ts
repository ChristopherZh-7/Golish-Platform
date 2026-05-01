import { invoke } from "./client";

export interface IntegrationStatus {
  type: "NotInstalled" | "Installed" | "Outdated";
  version?: string;
  current?: string;
  latest?: string;
}

export async function shellIntegrationStatus(): Promise<IntegrationStatus> {
  return invoke<IntegrationStatus>("shell_integration_status");
}

export async function shellIntegrationInstall(): Promise<void> {
  return invoke("shell_integration_install");
}

export async function shellIntegrationUninstall(): Promise<void> {
  return invoke("shell_integration_uninstall");
}

export interface ClassifyResult {
  route: "terminal" | "agent";
  detected_command: string | null;
}

export async function classifyInput(input: string): Promise<ClassifyResult> {
  return invoke<ClassifyResult>("classify_input", { input });
}

export type PathEntryType = "file" | "directory" | "symlink";

export interface PathCompletion {
  name: string;
  insert_text: string;
  entry_type: PathEntryType;
  score: number;
  match_indices: number[];
}

export interface PathCompletionResponse {
  completions: PathCompletion[];
  total_count: number;
}

export async function listPathCompletions(
  sessionId: string,
  partialPath: string,
  limit?: number,
): Promise<PathCompletionResponse> {
  return invoke<PathCompletionResponse>("list_path_completions", {
    sessionId,
    partialPath,
    limit,
  });
}

export async function imeGetSource(): Promise<string | null> {
  return invoke<string | null>("ime_get_source");
}

export async function imeSetSource(sourceId: string): Promise<boolean> {
  return invoke<boolean>("ime_set_source", { sourceId });
}
