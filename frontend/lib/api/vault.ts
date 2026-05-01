import { invoke } from "./client";

export async function addVaultEntry(params: {
  name: string;
  entryType: string;
  value: string;
  host?: string;
  notes?: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("vault_add", params);
}

export async function deleteVaultEntry(id: string, projectPath: string | null): Promise<void> {
  await invoke("vault_delete", { id, projectPath });
}

export async function updateVaultEntry(params: {
  id: string;
  notes?: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("vault_update", params);
}

export async function updateVaultStatus(id: string, status: string, projectPath: string | null): Promise<void> {
  await invoke("vault_update_status", { id, status, projectPath });
}
