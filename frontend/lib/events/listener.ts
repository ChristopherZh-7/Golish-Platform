/**
 * Type-safe event listener that enforces correct payload types
 * based on the channel name.
 */

import type { UnlistenFn } from "@tauri-apps/api/event";
import { listen as rawListen } from "../tauri-listen";
import type { EventPayloadMap } from "./payloads";

/**
 * Subscribe to a Tauri event channel with compile-time payload validation.
 *
 * ```ts
 * const unlisten = await onEvent("terminal_output", (payload) => {
 *   // payload is automatically typed as TerminalOutputPayload
 *   console.log(payload.session_id, payload.data);
 * });
 * ```
 */
export async function onEvent<K extends keyof EventPayloadMap>(
  channel: K,
  handler: (payload: EventPayloadMap[K]) => void,
): Promise<UnlistenFn> {
  return rawListen<EventPayloadMap[K]>(channel, (event) => handler(event.payload));
}

/**
 * Subscribe to a custom/untyped event channel.
 * Prefer `onEvent` for well-known channels.
 */
export async function onCustomEvent<T = unknown>(
  channel: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  return rawListen<T>(channel, (event) => handler(event.payload));
}
