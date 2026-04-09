/**
 * Project configuration API.
 *
 * Projects are stored as directories in ~/.golish/projects/<slug>/.
 * Each directory contains config.toml and workspace.json.
 */

import { invoke } from "@tauri-apps/api/core";
import { useStore } from "@/store";

/** Get the current project's root path from the store. */
export function getProjectPath(): string | null {
  return useStore.getState().currentProjectPath;
}

/** Helper to create a projectPath param for backend invoke calls. */
export function ppParam() {
  return { projectPath: getProjectPath() };
}

/** Project form data for creating/updating a project. */
export interface ProjectFormData {
  name: string;
  rootPath: string;
  /** Optional initial targets (domains, IPs, CIDRs, URLs) for auto-recon */
  targets?: string[];
}

/** Project data returned from the backend. */
export interface ProjectData {
  name: string;
  rootPath: string;
}

export async function saveProject(form: ProjectFormData): Promise<void> {
  await invoke("save_project", { form });
}

export async function deleteProject(name: string): Promise<boolean> {
  return invoke<boolean>("delete_project_config", { name });
}

export async function listProjectConfigs(): Promise<ProjectData[]> {
  return invoke<ProjectData[]>("list_project_configs");
}

export async function getProjectConfig(name: string): Promise<ProjectData | null> {
  return invoke<ProjectData | null>("get_project_config", { name });
}

/** Save workspace state JSON for a project. */
export async function saveProjectWorkspace(projectName: string, stateJson: string): Promise<void> {
  await invoke("save_project_workspace", { projectName, stateJson });
}

/** Load workspace state JSON for a project. Returns null if none exists. */
export async function loadProjectWorkspace(projectName: string): Promise<string | null> {
  return invoke<string | null>("load_project_workspace", { projectName });
}
