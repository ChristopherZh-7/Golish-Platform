import { invoke } from "./client";

/**
 * Vault entry create/update/delete/list/status IPC wrappers.
 *
 * Field set kept in sync with the Rust `vault_*` Tauri commands in
 * `backend/crates/golish/src/tools/vault.rs`. When the backend
 * signature changes, update both this file and the Rust handler
 * argument list (Tauri auto-converts snake_case to camelCase across
 * the IPC boundary).
 */

/** Result row returned by `vault_list` and `vault_add`. */
export interface VaultEntrySafe {
  id: string;
  name: string;
  entryType: string;
  username: string | null;
  notes: string | null;
  project: string | null;
  tags: string[];
  status: string;
  sourceUrl: string | null;
  lastValidatedAt: number | null;
  createdAt: number;
  updatedAt: number;
}

export interface AddVaultEntryParams {
  name: string;
  /** "password" | "token" | "api_key" | "cookie" | "other" — keep in sync with `VaultEntryType` enum. */
  entryType: string;
  value: string;
  username?: string | null;
  notes?: string | null;
  /**
   * Host/project the credential belongs to (e.g. `evil.example.com`).
   * Stored as the `project` column in `vault_entries`.
   */
  project?: string | null;
  /** Free-form tags shown in the credential list. */
  tags?: string[] | null;
  /** Origin URL the credential was captured from. */
  sourceUrl?: string | null;
  projectPath: string | null;
}

export async function addVaultEntry(params: AddVaultEntryParams): Promise<VaultEntrySafe> {
  return invoke<VaultEntrySafe>("vault_add", params as unknown as Record<string, unknown>);
}

export async function listVaultEntries(projectPath: string | null): Promise<VaultEntrySafe[]> {
  return invoke<VaultEntrySafe[]>("vault_list", { projectPath });
}

/**
 * Run validation on a stored credential (e.g. attempt a probe with
 * the stored value). Returns a human-readable status string.
 */
export async function validateVaultEntry(id: string, projectPath: string | null): Promise<string> {
  return invoke<string>("vault_validate", { id, projectPath });
}

/**
 * Fetch the plaintext credential value for an entry (one-shot).
 * Triggers an audit log entry server-side.
 */
export async function getVaultValue(id: string, projectPath: string | null): Promise<string> {
  return invoke<string>("vault_get_value", { id, projectPath });
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
