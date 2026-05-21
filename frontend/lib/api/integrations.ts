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
  /**
   * Optional auto-capture recipe. When present, the Settings UI renders
   * a ⚡ "Auto-fill" button next to the group header that opens an
   * isolated webview and harvests credentials after the user logs in.
   * Absent (the default) ⇒ no ⚡ button; the user fills the form by hand.
   */
  capture?: CaptureRecipe;
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
// Capture (mirrors golish-integrations/src/schema.rs CaptureRecipe +
//          src/types.rs CaptureState / CaptureSessionInfo)
//
// Wire-format snake_case to match serde defaults. Internal-tag
// `type` discriminator on CaptureRule matches Rust's
// `#[serde(tag = "type", rename_all = "snake_case")]`.
// ────────────────────────────────────────────────────────────────────────

/**
 * Schema-declared recipe describing how to harvest credentials after
 * the user logs in. One per [`IntegrationGroup`]; absent ⇒ feature off.
 */
export interface CaptureRecipe {
  /** First URL loaded in the capture webview (typically the login page). */
  login_url: string;
  /**
   * Regex; when the webview navigates to a URL matching this pattern
   * the engine treats login as complete and proceeds to extraction.
   * If unset the user must click "Capture now" manually.
   */
  success_url_pattern?: string;
  /**
   * Optional URL to navigate to *after* success but before extraction
   * (e.g. open a dashboard page that sets the cookie of interest).
   */
  visit_url?: string;
  /** Markdown shown above the webview to guide the user. */
  instructions?: string;
  /** Hard timeout for the whole session. Engine clamps to [30, 900]. */
  timeout_secs: number;
  /** Ordered list of extraction rules. */
  rules: CaptureRule[];
}

/**
 * Single extraction step. Each variant has its own input shape but
 * always writes to one target field via `target_field`. `required`
 * defaults to false on the wire (serde `default`); the UI surfaces
 * required failures as "session failed" but tolerates optional ones.
 */
export type CaptureRule =
  | {
      type: "cookie";
      domain: string;
      name: string;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "cookie_joined";
      domain: string;
      names: string[];
      /** Separator between joined values; default `; `. */
      sep?: string;
      /** Per-name format string; default `"{name}={value}"`. */
      fmt?: string;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "local_storage";
      key: string;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "session_storage";
      key: string;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "page_content";
      /** CSS selector (e.g. `meta[name=csrf-token]`). */
      selector: string;
      /** When set, read this attribute instead of `textContent`. */
      attribute?: string;
      /** Wait this many ms for the node to appear before failing. */
      wait_ms?: number;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "url_query";
      name: string;
      target_field: string;
      required?: boolean;
    };

/**
 * State of a capture session. Mirrors `CaptureState` in
 * `golish-integrations/src/types.rs`. Terminal states (Rust:
 * `CaptureState::is_terminal`):
 *   captured, partial, failed, timeout, cancelled.
 */
export type CaptureState =
  | "waiting_login"
  | "navigating"
  | "extracting"
  | "captured"
  | "partial"
  | "failed"
  | "timeout"
  | "cancelled";

/** Per-rule failure detail; `rule_index` is 0-based. */
export interface FailedRule {
  rule_index: number;
  reason: string;
}

/**
 * Snapshot of a capture session — returned by
 * `integrations_capture_start` / `_status` and emitted (minus
 * `expires_at`) on the `"integration-capture"` event channel as
 * [`CaptureEventPayload`]. Timestamps are Unix milliseconds so the UI
 * countdown can compare against `Date.now()` directly.
 */
export interface CaptureSessionInfo {
  session_id: string;
  tool_id: string;
  group_id: string;
  state: CaptureState;
  login_url: string;
  /** `target_field` values from the recipe rules, in declaration order. */
  expected_fields: string[];
  /** Subset of `expected_fields` actually written to vault. */
  captured_fields?: string[];
  failed_rules?: FailedRule[];
  error_message?: string;
  /** Unix milliseconds; absent for already-terminal sessions. */
  expires_at?: number;
  /** Unix milliseconds when state last transitioned. */
  updated_at: number;
}

/**
 * Payload emitted on the `"integration-capture"` Tauri event channel.
 * Subset of [`CaptureSessionInfo`] — the frontend already has
 * `expires_at` + `login_url` from the `_start` response and doesn't
 * need them re-delivered on every transition.
 */
export interface CaptureEventPayload {
  session_id: string;
  tool_id: string;
  group_id: string;
  state: CaptureState;
  captured_fields?: string[];
  failed_rules?: FailedRule[];
  error_message?: string;
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
