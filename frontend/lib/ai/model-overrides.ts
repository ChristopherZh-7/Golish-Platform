/**
 * Helpers for reading per-`(provider, model)` overrides out of settings and
 * shaping them for the wire format expected by the backend.
 *
 * Mirror conventions of `backend/crates/golish-settings/.../ModelOverride`
 * and the `model_override` field on every `ProviderConfig` variant.
 */

import type { GolishSettings, ModelOverride } from "@/lib/settings";

import type { ProviderModelOverride } from "./types";

/**
 * Storage key used inside `settings.ai.model_overrides`. Keep in lockstep
 * with the backend key convention so a TOML written by either side reads
 * the same.
 */
export function modelOverrideKey(provider: string, model: string): string {
  return `${provider}::${model}`;
}

/**
 * Look up the user's saved override for `(provider, model)`. Returns
 * `undefined` when no override exists; safe to call against an older
 * settings file that pre-dates `model_overrides`.
 */
export function getModelOverride(
  settings: GolishSettings,
  provider: string,
  model: string
): ModelOverride | undefined {
  return settings.ai.model_overrides?.[modelOverrideKey(provider, model)];
}

/**
 * Convert a settings-level {@link ModelOverride} to the transport-shaped
 * {@link ProviderModelOverride} carried on every `ProviderConfig` variant.
 *
 * Returns `undefined` when the override has no meaningful fields so the
 * backend can treat it as "no override".
 */
export function toProviderOverride(
  override: ModelOverride | undefined
): ProviderModelOverride | undefined {
  if (!override) return undefined;
  const out: ProviderModelOverride = {};
  if (override.thinking !== undefined) out.thinking = override.thinking;
  if (override.reasoning_effort !== undefined) out.reasoning_effort = override.reasoning_effort;
  if (override.max_tokens !== undefined) out.max_tokens = override.max_tokens;
  if (override.context_window !== undefined) out.context_window = override.context_window;
  if (override.stream_debug !== undefined) out.stream_debug = override.stream_debug;
  return Object.keys(out).length > 0 ? out : undefined;
}

/**
 * Convenience: read settings, return the shaped wire-level override (or
 * `undefined`). Equivalent to
 * `toProviderOverride(getModelOverride(settings, provider, model))`.
 */
export function resolveProviderOverride(
  settings: GolishSettings,
  provider: string,
  model: string
): ProviderModelOverride | undefined {
  return toProviderOverride(getModelOverride(settings, provider, model));
}

// ---------------------------------------------------------------------------
// Live override change subscribers
// ---------------------------------------------------------------------------
//
// A lightweight pub/sub used to refresh UI fragments (e.g. the thinking
// indicator badge next to the model name) the moment the user toggles a
// switch in `ModelSettingsPopover`, without dragging the entire settings
// blob through Zustand or React Context.
//
// `notifyModelOverrideChanged` is invoked from the popover after a
// successful persist; subscribers re-read `settings.ai.model_overrides`
// via `getSettingsCached()` and update their derived state.

type OverrideChangeListener = (key: string) => void;

const overrideChangeListeners = new Set<OverrideChangeListener>();

export function subscribeToModelOverrideChanges(listener: OverrideChangeListener): () => void {
  overrideChangeListeners.add(listener);
  return () => {
    overrideChangeListeners.delete(listener);
  };
}

export function notifyModelOverrideChanged(key: string): void {
  for (const listener of overrideChangeListeners) {
    try {
      listener(key);
    } catch (err) {
      console.warn("[model-overrides] listener threw", err);
    }
  }
}
