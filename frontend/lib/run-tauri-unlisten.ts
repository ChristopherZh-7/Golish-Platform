import type { UnlistenFn } from "@tauri-apps/api/event";

/**
 * Tauri's unlisten is implemented as an async function; `UnlistenFn` is typed as `() => void` but
 * calling it returns a Promise. Dropping that Promise shows up as an unhandled rejection when
 * `unregisterListener` throws (e.g. double cleanup / strict mode).
 */
export function runTauriUnlistenFromPromise(promise: Promise<UnlistenFn>): void {
  void promise
    .then((unlisten) => Promise.resolve(unlisten()).catch(() => undefined))
    .catch(() => undefined);
}

/**
 * For `await listen(...)` call sites that hold a bare `UnlistenFn`.
 */
export function runTauriUnlistenFn(unlisten: UnlistenFn | null | undefined): void {
  if (!unlisten) {
    return;
  }
  void Promise.resolve(unlisten()).catch(() => undefined);
}
