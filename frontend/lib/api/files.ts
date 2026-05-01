import { invoke } from "./client";

export interface PromptInfo {
  name: string;
  path: string;
  source: "global" | "local";
}

export async function listPrompts(workingDirectory?: string): Promise<PromptInfo[]> {
  return invoke<PromptInfo[]>("list_prompts", { workingDirectory });
}

export async function readPrompt(path: string): Promise<string> {
  return invoke<string>("read_prompt", { path });
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
  return invoke<SkillInfo[]>("list_skills", { workingDirectory });
}

export async function readSkill(path: string): Promise<string> {
  return invoke<string>("read_skill", { path });
}

export async function readSkillBody(path: string): Promise<string> {
  return invoke<string>("read_skill_body", { path });
}

export async function listSkillFiles(skillPath: string, subdir: string): Promise<SkillFileInfo[]> {
  return invoke<SkillFileInfo[]>("list_skill_files", { skillPath, subdir });
}

export async function readSkillFile(skillPath: string, relativePath: string): Promise<string> {
  return invoke<string>("read_skill_file", { skillPath, relativePath });
}

export interface FileInfo {
  name: string;
  relative_path: string;
}

export async function listWorkspaceFiles(
  workingDirectory: string,
  query?: string,
  limit?: number,
): Promise<FileInfo[]> {
  return invoke<FileInfo[]>("list_workspace_files", { workingDirectory, query, limit });
}

export async function readFileAsBase64(path: string): Promise<string> {
  return invoke<string>("read_file_as_base64", { path });
}

export async function readTextFile(
  workingDirectory: string,
  relativePath: string,
): Promise<string> {
  const fullPath = `${workingDirectory}/${relativePath}`;
  return invoke<string>("read_prompt", { path: fullPath });
}

export async function listDirectory(
  workingDirectory: string,
  path: string,
): Promise<FileInfo[]> {
  return invoke<FileInfo[]>("list_directory", { workingDirectory, path });
}

export async function readWorkspaceFile(
  workingDirectory: string,
  relativePath: string,
): Promise<string> {
  return invoke<string>("read_workspace_file", { workingDirectory, relativePath });
}

export async function writeWorkspaceFile(
  workingDirectory: string,
  relativePath: string,
  content: string,
): Promise<void> {
  return invoke("write_workspace_file", { workingDirectory, relativePath, content });
}

export interface FileStat {
  exists: boolean;
  is_file: boolean;
  is_directory: boolean;
  size: number | null;
  modified_at: string | null;
}

export async function statWorkspaceFile(
  workingDirectory: string,
  relativePath: string,
): Promise<FileStat> {
  return invoke<FileStat>("stat_workspace_file", { workingDirectory, relativePath });
}

export async function watchFile(
  workingDirectory: string,
  relativePath: string,
): Promise<void> {
  return invoke("watch_file", { workingDirectory, relativePath });
}

export async function unwatchFile(
  workingDirectory: string,
  relativePath: string,
): Promise<void> {
  return invoke("unwatch_file", { workingDirectory, relativePath });
}

export async function unwatchAllFiles(): Promise<void> {
  return invoke("unwatch_all_files");
}
