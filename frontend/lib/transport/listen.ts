/**
 * Typed event listener wrapper for Tauri backend events.
 *
 * Re-exports the existing listen helper through the transport layer
 * so services don't import from @tauri-apps/api directly.
 */

import type { UnlistenFn } from "@tauri-apps/api/event";
import { listen as tauriListen } from "../tauri-listen";

export type { UnlistenFn };

export function listen<T>(
  channel: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  return tauriListen<T>(channel, (event) => handler(event.payload));
}
