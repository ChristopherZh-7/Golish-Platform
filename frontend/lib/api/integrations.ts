import { invoke } from "./client";

/**
 * Integrations IPC wrappers.
 *
 * Schema-driven external-service credentials. The backend lives in
 * `backend/crates/golish-integrations` (types) +
 * `backend/crates/golish/src/tools/integrations/commands.rs` (IPC).
 *
 * The wire-format types here mirror the Rust types one-to-one. Field
 * names are snake_case to match serde defaults. **Do not** rename
 * fields without updating the corresponding Rust `#[serde]` attribute.
 *
 * See `docs/design/2026-05-21-integrations.md` for the architecture.
 */

// ────────────────────────────────────────────────────────────────────────
// Schema (mirrors golish-integrations/src/schema.rs)
// ────────────────────────────────────────────────────────────────────────

/**
 * Renderer hint for a single field. The Settings UI has one
 * dedicated component per variant:
 *
 * - `secret_text`     → `<SecretInput>`     (password input + reveal)
 * - `secret_textarea` → `<SecretTextarea>`  (multi-line + reveal)
 * - `text`            → plain `<input>`
 * - `url`             → `<UrlInput>`
 * - `port`            → numeric `<input type="number">`
 * - `select`          → `<SelectField>` (uses `options[]`)
 * - `boolean`         → checkbox / toggle
 * - `proxy`           → composite proxy editor
 */
export type FieldType =
  | "secret_text"
  | "secret_textarea"
  | "text"
  | "url"
  | "port"
  | "select"
  | "boolean"
  | "proxy";

export interface SelectOption {
  value: string;
  label: string;
}

export interface Field {
  /** Dotted key path (e.g. `"cookies.aqc"`). */
  key: string;
  /** Human-readable label rendered next to the input. */
  label: string;
  /** Renderer hint, see [`FieldType`]. */
  type: FieldType;
  placeholder?: string;
  /** When true the form rejects blank values client-side. */
  required?: boolean;
  /** Optional rows hint for `secret_textarea`. */
  rows?: number;
  /** Options for `select`-type fields. */
  options?: SelectOption[];
  /** Optional regex (server-side authoritative). */
  pattern?: string;
}

export interface IntegrationGroup {
  /** Stable id, e.g. `"default"` / `"aqc"` / `"tyc"`. */
  id: string;
  /** Display name in the card sub-header. */
  name: string;
  description?: string;
  icon?: string;
  /** Group-specific help URL (overrides schema-level when set). */
  help_url?: string;
  fields: Field[];
  test?: TestKind;
}

/**
 * The three connectivity-test recipes a schema can declare.
 *
 * - `builtin`: defer to the provider's own `test_connection` (Phase 5)
 * - `exec`:    spawn a command, match stdout against regex
 * - `http`:    fire an HTTP request, check status code range
 */
export type TestKind =
  | { kind: "builtin" }
  | {
      kind: "exec";
      cmd: string;
      ok_regex: string;
      fail_regex?: string;
      timeout_secs?: number;
    }
  | {
      kind: "http";
      method: string;
      url: string;
      headers?: Record<string, string>;
      /** Inclusive [lo, hi]. */
      ok_status_range?: [number, number];
      timeout_secs?: number;
    };

export interface VaultStorageDescriptor {
  extra_tags?: string[];
}

export interface ExternalFileStorageDescriptor {
  path: string;
  format?: "yaml" | "json";
  preserve_unknown_keys?: boolean;
  backup_on_write?: boolean;
}

export interface SettingsStorageDescriptor {
  key: string;
}

export type Storage =
  | { type: "vault"; vault?: VaultStorageDescriptor }
  | { type: "external_file"; external_file: ExternalFileStorageDescriptor }
  | { type: "settings"; settings: SettingsStorageDescriptor };

export interface IntegrationSchema {
  category: string;
  display_name: string;
  description?: string;
  storage: Storage;
  groups: IntegrationGroup[];
  help_url?: string;
}

export interface ResolvedIntegration {
  tool_id: string;
  schema: IntegrationSchema;
}

// ────────────────────────────────────────────────────────────────────────
// Runtime values (mirrors golish-integrations/src/types.rs)
// ────────────────────────────────────────────────────────────────────────

export interface FieldValue {
  /** True when this field has been written before. */
  has_value: boolean;
  /** Plaintext for non-secret fields; always `null` for secrets. */
  value?: string | null;
  /** Short obfuscated preview (e.g. `"AKIA****WXYZ"`). */
  display_hint?: string | null;
  /** RFC3339 timestamp string of last update. */
  updated_at?: string | null;
}

export type HealthStatus = "healthy" | "invalid" | "expired" | "rate_limited" | "unknown";

export interface IntegrationHealth {
  status: HealthStatus;
  /** Human-readable message (never includes the credential value). */
  message: string;
  tested_at: string;
}

// ────────────────────────────────────────────────────────────────────────
// IPC wrappers
// ────────────────────────────────────────────────────────────────────────

/**
 * List every integration schema known to this Golish install.
 *
 * Sources merged server-side:
 *  - `resources/toolsconfig/*.json` files declaring an `integration` block
 *  - In-code declarations from `IntelProvider::meta().integration_schema`
 *  - Hardcoded `core_integrations.json` (Phase 5)
 *
 * Returned in alphabetical order by `tool_id`.
 */
export async function listSchemas(): Promise<ResolvedIntegration[]> {
  return invoke<ResolvedIntegration[]>("integrations_list_schemas");
}

/**
 * Read the current field values for one group.
 *
 * Secret-typed fields surface with `has_value: true, value: null` so
 * the UI can show a "configured" badge without ever holding the
 * secret in memory longer than necessary. Non-secret fields surface
 * with `value` populated.
 */
export async function get(args: {
  toolId: string;
  groupId: string;
}): Promise<Record<string, FieldValue>> {
  return invoke<Record<string, FieldValue>>("integrations_get", {
    toolId: args.toolId,
    groupId: args.groupId,
  });
}

/**
 * Persist the user-edited field values for one group.
 *
 * The backend validates required-fields and unknown-keys server-side;
 * client-side validation is purely a UX optimization. Backend errors
 * surface as `ApiError` with the underlying `GolishError` variant in
 * the message.
 */
export async function set(args: {
  toolId: string;
  groupId: string;
  fields: Record<string, string>;
}): Promise<void> {
  return invoke<void>("integrations_set", {
    toolId: args.toolId,
    groupId: args.groupId,
    fields: args.fields,
  });
}

/**
 * Delete every field belonging to a group.
 *
 * For `vault` storage this drops both the per-field rows and the
 * legacy single-key row used by the old `IntelProvidersSettings` UI
 * (so "Clear" really empties everything across the compatibility
 * boundary). For `external_file` only schema-declared keys are
 * removed; user-added keys outside the schema are preserved.
 */
export async function clear(args: { toolId: string; groupId: string }): Promise<void> {
  return invoke<void>("integrations_clear", {
    toolId: args.toolId,
    groupId: args.groupId,
  });
}

/**
 * Run the schema-declared connectivity test against the currently
 * stored credentials. Returns one of five health statuses
 * ([`HealthStatus`]) with a short human-readable message.
 *
 * Phase 3 note: schemas declaring `TestKind { kind: "builtin" }`
 * return `unknown` until the Phase 5 dispatch hook lands. Schemas
 * declaring `exec` / `http` are fully supported in Phase 3.
 */
export async function test(args: { toolId: string; groupId: string }): Promise<IntegrationHealth> {
  return invoke<IntegrationHealth>("integrations_test", {
    toolId: args.toolId,
    groupId: args.groupId,
  });
}
