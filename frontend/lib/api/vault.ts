import { invoke } from "./client";

/**
 * Vault entry create/update/delete/list/status IPC wrappers.
 *
 * The shape of `VaultEntrySafe` (and its `type` enum) mirrors
 * `golish-core/src/vault.rs::VaultEntrySafe` — the canonical Rust
 * struct returned by all `vault_*` Tauri commands. Field names are
 * snake_case to match the JSON over the wire (Rust serializes with
 * `#[serde(rename = "type")]` for `entry_type`).
 *
 * Argument types (`AddVaultEntryParams` etc.) below stay hand-written
 * because Tauri's auto camelCase→snake_case conversion only applies to
 * *arguments* (front → back), and they don't 1:1 mirror any Rust struct.
 *
 * Whenever the Rust struct changes, update the field set / enum literals
 * here in lockstep. (Previously this came from `lib/generated/` ts-rs
 * codegen which is being removed per M2.5.)
 */

export type VaultEntryType =
  | "password"
  | "token"
  | "ssh_key"
  | "api_key"
  | "cookie"
  | "certificate"
  | "other";

export interface VaultEntrySafe {
  id: string;
  name: string;
  type: VaultEntryType;
  username: string;
  notes: string;
  project: string;
  tags: string[];
  status: string;
  source_url: string;
  /**
   * Unix seconds. Nullable when the entry has never been validated.
   * Frontend arithmetic (e.g. `entry.last_validated_at * 1000`) treats
   * this as `number` (timestamps fit in `Number.MAX_SAFE_INTEGER`).
   */
  last_validated_at: number | null;
  created_at: number;
  updated_at: number;
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
