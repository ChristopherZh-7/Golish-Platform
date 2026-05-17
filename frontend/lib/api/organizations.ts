import { invoke } from "./client";

/**
 * Multi-level organization tree introduced in §S3. The tree is reconstructed
 * client-side from the flat list returned by `list()` (each node has
 * `parent_id` pointing at its container; root nodes have `parent_id === null`).
 */
export interface Organization {
  id: string;
  project_path: string;
  name: string;
  parent_id: string | null;
  description: string;
  owner: string;
  sort_order: number;
  created_at: number;
  updated_at: number;
}

export async function listOrganizations(
  projectPath: string | null
): Promise<Organization[]> {
  return invoke<Organization[]>("organization_list", { projectPath });
}

export async function createOrganization(params: {
  projectPath: string | null;
  name: string;
  parentId?: string;
  description?: string;
  owner?: string;
}): Promise<Organization> {
  return invoke<Organization>("organization_create", params);
}

export async function updateOrganization(params: {
  id: string;
  name?: string;
  description?: string;
  owner?: string;
  sortOrder?: number;
}): Promise<Organization> {
  return invoke<Organization>("organization_update", params);
}

export async function moveOrganization(params: {
  id: string;
  newParentId: string | null;
}): Promise<void> {
  await invoke("organization_move", params);
}

export async function deleteOrganization(id: string): Promise<void> {
  await invoke("organization_delete", { id });
}
