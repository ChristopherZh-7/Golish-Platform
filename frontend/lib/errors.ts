import i18n from "@/lib/i18n";

/**
 * Sentinel prefix that the Rust `GolishError::I18n` variant emits when
 * serialized over the Tauri IPC boundary. Keep this in sync with
 * `backend/crates/golish/src/error.rs::I18N_ERROR_PREFIX`.
 */
const I18N_PREFIX = "[i18n]";

/**
 * Take the raw error value caught around a Tauri `invoke` (or any Rust →
 * frontend boundary) and return a *user-facing, language-aware* string.
 *
 * Format produced by Rust:
 *   "[i18n]<dotted.code>|<json-params>"
 *
 * Anything that does not start with the sentinel is returned untouched —
 * we still preserve the original "errors are passthrough strings" contract
 * that pre-existing code relies on.
 */
export function localizeBackendError(err: unknown): string {
  const raw = stringifyError(err);
  if (!raw.startsWith(I18N_PREFIX)) return raw;
  const body = raw.slice(I18N_PREFIX.length);
  const sep = body.indexOf("|");
  if (sep < 0) return raw;
  const code = body.slice(0, sep);
  const paramsRaw = body.slice(sep + 1);
  let params: Record<string, unknown> = {};
  try {
    const parsed = JSON.parse(paramsRaw);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      params = parsed as Record<string, unknown>;
    }
  } catch {
    /* fall through with empty params */
  }
  // i18next will use fallbackLng=en when a key is missing, then fall through
  // to defaultValue. We surface the raw English code as last resort so the
  // user always gets *something* readable instead of the sentinel-prefixed
  // payload.
  return i18n.t(`backend.errors.${code}`, { ...params, defaultValue: code });
}

function stringifyError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  if (err && typeof err === "object" && "toString" in err) {
    try {
      return String(err);
    } catch {
      return JSON.stringify(err);
    }
  }
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

/**
 * Tiny helper that wraps a promise (typically `invoke<T>(...)`) and rethrows
 * with a localized error message. Use this when you would otherwise write
 * `try { ... } catch (e) { setError(String(e)); }`.
 */
export async function localizeBackendCall<T>(p: Promise<T>): Promise<T> {
  try {
    return await p;
  } catch (e) {
    throw new Error(localizeBackendError(e));
  }
}
