/**
 * Project configuration IPC wrappers.
 *
 * Projects are stored as directories in `~/.golish/projects/<slug>/`,
 * each containing config.toml and workspace.json. This facade is the
 * **store-independent** IPC layer; the store-aware helpers
 * (`getProjectPath`, `ppParam`) live in `frontend/lib/projects.ts`
 * which re-exports everything from here for compatibility.
 *
 * See ADR-0009 Phase 2 / 2D-2.
 */

import { invoke } from "./client";

// ============================================================================
// Types
// ============================================================================

export interface ProjectFormData {
  name: string;
  rootPath: string;
  /** Optional initial targets (domains, IPs, CIDRs, URLs) for auto-recon */
  targets?: string[];
}

export interface ProjectData {
  name: string;
  rootPath: string;
}

export interface ScopeConfig {
  inScope: string[];
  outOfScope: string[];
}

export interface ProxyConfig {
  zapApiUrl?: string;
  zapApiKey?: string;
}

export interface CaptureConfig {
  autoSaveJs: boolean;
  autoSaveHtml: boolean;
  autoSaveToolOutput: boolean;
  maxFileSizeMb: number;
}

export interface PentestProjectConfig {
  name: string;
  createdAt: string;
  scope: ScopeConfig;
  proxy: ProxyConfig;
  capture: CaptureConfig;
  hostMap: Record<string, string[]>;
  notes: string;
}

export interface HostCaptures {
  host: string;
  ports: number[];
}

export interface CaptureOverview {
  hosts: HostCaptures[];
  toolOutputs: string[];
}

// ============================================================================
// Project lifecycle
// ============================================================================

export async function saveProject(form: ProjectFormData): Promise<void> {
  await invoke("save_project", { form: form as unknown as Record<string, unknown> });
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

// ============================================================================
// Pentest project config & file storage
// ============================================================================

export async function getPentestConfig(projectName: string): Promise<PentestProjectConfig | null> {
  return invoke<PentestProjectConfig | null>("get_pentest_config", {
    projectName,
  });
}

export async function savePentestConfig(
  projectName: string,
  config: PentestProjectConfig
): Promise<void> {
  await invoke("save_pentest_config", {
    projectName,
    config: config as unknown as Record<string, unknown>,
  });
}

export async function listCaptures(projectName: string): Promise<CaptureOverview> {
  return invoke<CaptureOverview>("list_captures", { projectName });
}

export async function listCaptureFiles(
  projectName: string,
  host: string,
  port: number,
  fileType: string
): Promise<string[]> {
  return invoke<string[]>("list_capture_files", {
    projectName,
    host,
    port,
    fileType,
  });
}

export async function readProjectFile(projectName: string, relPath: string): Promise<string> {
  return invoke<string>("read_project_file", { projectName, relPath });
}

export async function initProjectStructure(projectName: string): Promise<void> {
  await invoke("init_project_structure", { projectName });
}

export async function cleanProjectTemp(projectName: string): Promise<number> {
  return invoke<number>("clean_project_temp", { projectName });
}
