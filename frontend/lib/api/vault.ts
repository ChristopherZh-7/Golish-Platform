import { invoke } from "./client";

/**
 * Vault entry create/update/delete/status IPC wrappers.
 *
 * Field set kept in sync with the Rust `vault_*` Tauri commands
 * (see `backend/crates/golish-pentest/src/vault.rs`). When the
 * backend signature changes, update both this file and the
 * Rust command's `#[derive(serde::Deserialize)]` struct.
 */

export interface AddVaultEntryParams {
  name: string;
  entryType: string;
  value: string;
  username?: string | null;
  host?: string | null;
  notes?: string | null;
  projectPath: string | null;
}

export async function addVaultEntry(params: AddVaultEntryParams): Promise<{ id: string }> {
  return invoke<{ id: string }>("vault_add", params as unknown as Record<string, unknown>);
}

export async function deleteVaultEntry(id: string, projectPath: string | null): Promise<void> {
  await invoke("vault_delete", { id, projectPath });
}

export interface UpdateVaultEntryParams {
  id: string;
  /** New plaintext credential value (only required when rotating the secret). */
  value?: string | null;
  /** Updated username (e.g. when a duplicate-host credential changes its login). */
  username?: string | null;
  /** Free-form notes shown in the credential vault UI. */
  notes?: string | null;
  projectPath: string | null;
}

export async function updateVaultEntry(params: UpdateVaultEntryParams): Promise<void> {
  await invoke("vault_update", params as unknown as Record<string, unknown>);
}

export async function updateVaultStatus(
  id: string,
  status: string,
  projectPath: string | null
): Promise<void> {
  await invoke("vault_update_status", { id, status, projectPath });
}
