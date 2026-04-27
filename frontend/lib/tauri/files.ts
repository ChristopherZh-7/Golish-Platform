import { invoke } from "@tauri-apps/api/core";

export interface PromptInfo {
  name: string;
  path: string;
  source: "global" | "local";
}

export async function listPrompts(workingDirectory?: string): Promise<PromptInfo[]> {
  return invoke("list_prompts", { workingDirectory });
}

export async function readPrompt(path: string): Promise<string> {
  return invoke("read_prompt", { path });
}

export interface SkillInfo {
  name: string;
  path: string;
  source: "global" | "local";
  description: string;
  license?: string;
  compatibility?: string;
  metadata?: Record<string, string>;
  allowed_tools?: string[];
  has_scripts: boolean;
  has_references: boolean;
  has_assets: boolean;
}

export interface SkillFileInfo {
  name: string;
  relative_path: string;
  is_directory: boolean;
}

export async function listSkills(workingDirectory?: string): Promise<SkillInfo[]> {
  return invoke("list_skills", { workingDirectory });
}

export async function readSkill(path: string): Promise<string> {
  return invoke("read_skill", { path });
}

export async function readSkillBody(path: string): Promise<string> {
  return invoke("read_skill_body", { path });
}

export async function listSkillFiles(skillPath: string, subdir: string): Promise<SkillFileInfo[]> {
  return invoke("list_skill_files", { skillPath, subdir });
}

export async function readSkillFile(skillPath: string, relativePath: string): Promise<string> {
  return invoke("read_skill_file", { skillPath, relativePath });
}

export interface FileInfo {
  name: string;
  relative_path: string;
}

export async function listWorkspaceFiles(
  workingDirectory: string,
  query?: string,
  limit?: number
): Promise<FileInfo[]> {
  return invoke("list_workspace_files", { workingDirectory, query, limit });
}

export async function readFileAsBase64(path: string): Promise<string> {
  return invoke("read_file_as_base64", { path });
}

export async function readTextFile(
  workingDirectory: string,
  relativePath: string
): Promise<string> {
  const fullPath = `${workingDirectory}/${relativePath}`;
  return invoke("read_prompt", { path: fullPath });
}
