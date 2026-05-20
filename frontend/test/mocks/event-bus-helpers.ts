import { vi } from "vitest";

/**
 * In-memory replacement for `@tauri-apps/api/event` used in unit
 * tests. The real module talks to the Tauri IPC bridge that
 * doesn't exist under jsdom / happy-dom; without this shim every
 * `useTauriEvents` / `useAiEvents` / `onEvent` consumer would crash
 * with `listen is not a function` the moment a hook mounted.
 *
 * `vitest.config.ts` aliases `@tauri-apps/api/event` to this file,
 * so any production code that does
 *
 *   import { listen, emit } from "@tauri-apps/api/event";
 *
 * receives the named exports below at module-eval time. Tests that
 * need to drive events use the helpers (`emitMockEvent`,
 * `clearMockListeners`, `getListenerCount`) imported directly from
 * this file (not through the alias).
 *
 * Both surfaces share the same `listeners` array so an
 * `emitMockEvent("foo", payload)` call delivers to listeners that
 * were registered through the aliased `listen()`. Keeping the state
 * at module scope works because Vitest's module graph caches this
 * file once per worker, so listen-time and emit-time both reach
 * the same array.
 */

type EventCallback<T> = (event: { payload: T }) => void;
type UnlistenFn = () => void;

interface EventListener<T = unknown> {
  eventName: string;
  callback: EventCallback<T>;
}

const listeners: EventListener[] = [];

/**
 * Aliased into `@tauri-apps/api/event#listen` — production code
 * calls this when subscribing to backend events.
 */
export async function listen<T>(
  eventName: string,
  callback: EventCallback<T>
): Promise<UnlistenFn> {
  const listener: EventListener<T> = { eventName, callback };
  listeners.push(listener as EventListener);
  return () => {
    const index = listeners.indexOf(listener as EventListener);
    if (index > -1) {
      listeners.splice(index, 1);
    }
  };
}

/**
 * Aliased into `@tauri-apps/api/event#emit` — production code uses
 * this to fan-out frontend → backend events. We default to a noop
 * spy so call sites that assert on it work; tests that care about
 * emit calls can re-spy with `vi.spyOn` if they want richer
 * behaviour.
 */
export const emit = vi.fn().mockResolvedValue(undefined);

/** Aliased into `@tauri-apps/api/event#emitTo` (multi-window emit). */
export const emitTo = vi.fn().mockResolvedValue(undefined);

/**
 * Aliased into `@tauri-apps/api/event#once` — same shape as
 * `listen` but auto-unsubscribes after the first delivery.
 */
export async function once<T>(eventName: string, callback: EventCallback<T>): Promise<UnlistenFn> {
  const unlisten = await listen<T>(eventName, (event) => {
    unlisten();
    callback(event);
  });
  return unlisten;
}

/**
 * Aliased into `@tauri-apps/api/event#TauriEvent` enum — the real
 * Tauri module exports an enum of well-known channels. Production
 * code in this codebase only references it for type narrowing, so
 * an empty object is enough to keep imports resolving.
 */
export const TauriEvent = {} as const;

// ---- Test helpers (imported directly, not via the alias) --------

/** Drive a listener registered through the aliased `listen()`. */
export function emitMockEvent<T>(eventName: string, payload: T): void {
  // Snapshot so handlers can unsubscribe without invalidating the
  // ongoing iteration.
  const snapshot = [...listeners];
  for (const listener of snapshot) {
    if (listener.eventName === eventName) {
      listener.callback({ payload });
    }
  }
}

/** Reset between tests so listeners don't leak across cases. */
export function clearMockListeners(): void {
  listeners.length = 0;
}

/** Inspect how many handlers are registered for a channel. */
export function getListenerCount(eventName: string): number {
  return listeners.filter((l) => l.eventName === eventName).length;
}

/** Back-compat alias; `mockListen` was the legacy name before we
 *  exposed the real `listen` export above. */
export const mockListen = vi.fn(listen);
