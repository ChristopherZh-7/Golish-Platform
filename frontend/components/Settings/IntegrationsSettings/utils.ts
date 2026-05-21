/**
 * Shared helpers for the IntegrationsSettings tree.
 */

import type { TFunction } from "i18next";

/**
 * Sanitise a `tool_id` for use as an i18next path segment.
 *
 * i18next splits keys on `.` (and now also on a few other characters in
 * recent versions), so an id like `"0.zone"` would otherwise be parsed
 * as the nested path `0 → zone`. We replace anything that isn't
 * `[a-zA-Z0-9]` with `_` so e.g. `"0.zone"` → `"0_zone"`.
 */
export function safeI18nId(id: string): string {
  return id.replace(/[^a-zA-Z0-9]/g, "_");
}

/**
 * Wrapper around `t(key, { defaultValue })` that returns the
 * fallback string verbatim when the key is missing. Keeps the call
 * sites compact and explicit about the contract:
 *
 *   tWithDefault(t, "integrations.tool.zone.display_name", "0.zone（零零信安）")
 *
 * The Rust schema's `display_name` is taken as the de-facto default,
 * so unlocalised installs (no i18n key for that tool) keep working.
 */
export function tWithDefault(t: TFunction, key: string, fallback: string): string {
  return t(key, { defaultValue: fallback });
}
