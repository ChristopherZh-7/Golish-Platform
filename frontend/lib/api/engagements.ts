import { invoke } from "./client";

/**
 * Engagement = HVV / red-team project metadata.
 * Time fields are unix seconds (matching backend `u64`). Convert to/from
 * ISO 8601 at the form layer.
 */
export interface Engagement {
  project_path: string;
  hvv_name: string;
  team_members: string[];
  start_at: number | null;
  end_at: number | null;
  notes: string;
  created_at: number;
  updated_at: number;
}

export async function getEngagement(projectPath: string): Promise<Engagement | null> {
  return invoke<Engagement | null>("engagement_get", { projectPath });
}

export async function saveEngagement(params: {
  projectPath: string;
  hvvName?: string;
  teamMembers?: string[];
  startAt?: string;
  endAt?: string;
  notes?: string;
}): Promise<Engagement> {
  return invoke<Engagement>("engagement_save", params);
}

export async function deleteEngagement(projectPath: string): Promise<void> {
  await invoke("engagement_delete", { projectPath });
}
