import { invoke } from "./client";

export interface MethodologyItem {
  id: string;
  title: string;
  description: string;
  checked: boolean;
  notes: string;
  tools: string[];
}

export interface MethodologyPhase {
  id: string;
  name: string;
  description: string;
  items: MethodologyItem[];
}

export interface MethodologyTemplate {
  id: string;
  name: string;
  description: string;
  phases: MethodologyPhase[];
}

export interface ProjectMethodology {
  id: string;
  template_id: string;
  template_name: string;
  project_name: string;
  phases: MethodologyPhase[];
  created_at: string;
  updated_at: string;
}

export async function listTemplates(): Promise<MethodologyTemplate[]> {
  const t = await invoke<MethodologyTemplate[]>("method_list_templates");
  return Array.isArray(t) ? t : [];
}

export async function listProjects(projectPath: string | null): Promise<ProjectMethodology[]> {
  const p = await invoke<ProjectMethodology[]>("method_list_projects", { projectPath });
  return Array.isArray(p) ? p : [];
}

export async function startProject(params: {
  templateId: string;
  projectName: string;
  projectPath: string | null;
}): Promise<ProjectMethodology> {
  return invoke<ProjectMethodology>("method_start_project", params);
}

export async function loadProject(
  id: string,
  projectPath: string | null
): Promise<ProjectMethodology> {
  return invoke<ProjectMethodology>("method_load_project", { id, projectPath });
}

export async function deleteProject(id: string, projectPath: string | null): Promise<void> {
  await invoke("method_delete_project", { id, projectPath });
}

export async function updateItem(params: {
  projectId: string;
  phaseId: string;
  itemId: string;
  checked: boolean | null;
  notes: string | null;
  projectPath: string | null;
}): Promise<void> {
  await invoke("method_update_item", params);
}
