/**
 * Type-safe event listener that enforces correct payload types
 * based on the channel name.
 *
 * Each subscription is tagged with a short hex `sub` id, logged at
 * `debug` level on subscribe / unsubscribe and at `error` level on
 * subscribe failure. Per-event logging is intentionally omitted to
 * avoid flooding the console on hot channels (terminal_output, ai-event).
 */

import type { UnlistenFn } from "@tauri-apps/api/event";
import { logger } from "@/lib/logger";
import { listen as rawListen } from "../tauri-listen";
import type { EventPayloadMap } from "./payloads";

/** Generate a short hex subscription id for log correlation. */
function generateSubId(): string {
  return Math.floor(Math.random() * 0xffffffff)
    .toString(16)
    .padStart(8, "0");
}

/**
 * Subscribe to a Tauri event channel with compile-time payload validation.
 *
 * ```ts
 * const unlisten = await onEvent("terminal_output", (payload) => {
 *   console.log(payload.session_id, payload.data);
 * });
 * ```
 *
 * Optional `guard` enables runtime payload validation (added in QW6 ·
 * 2026-05). When provided, payloads that fail the guard are dropped
 * silently and a `logger.warn` is emitted with the offending payload —
 * the handler is **not** invoked. This protects callers from
 * malformed backend payloads (e.g. backend rolled out a new field
 * that broke the wire format).
 *
 * Type guards for the three hot channels (`ai-event`, `pipeline-event`,
 * `sidecar-event`) live in `./payloads.ts`:
 * `isAiEvent` / `isPipelineEventPayload` / `isSidecarEventPayload`.
 *
 * ```ts
 * import { onEvent } from "@/lib/events";
 * import { isAiEvent } from "@/lib/events/payloads";
 *
 * const unlisten = await onEvent("ai-event", handleEvent, isAiEvent);
 * ```
 */
export async function onEvent<K extends keyof EventPayloadMap>(
  channel: K,
  handler: (payload: EventPayloadMap[K]) => void,
  guard?: (v: unknown) => v is EventPayloadMap[K]
): Promise<UnlistenFn> {
  const sub = generateSubId();
  try {
    const unlisten = await rawListen<EventPayloadMap[K]>(channel, (event) => {
      if (guard && !guard(event.payload)) {
        logger.warn(`[events] payload guard rejected channel=${channel} sub=${sub}`, event.payload);
        return;
      }
      handler(event.payload);
    });
    logger.debug(`[events] subscribed channel=${channel} sub=${sub}`);
    return () => {
      logger.debug(`[events] unsubscribed channel=${channel} sub=${sub}`);
      unlisten();
    };
  } catch (err) {
    logger.error(`[events] subscribe failed channel=${channel} sub=${sub}:`, err);
    throw err;
  }
}

/**
 * Subscribe to a custom / untyped event channel.
 * Prefer `onEvent` for well-known channels defined in `EventPayloadMap`.
 */
export async function onCustomEvent<T = unknown>(
  channel: string,
  handler: (payload: T) => void
): Promise<UnlistenFn> {
  const sub = generateSubId();
  try {
    const unlisten = await rawListen<T>(channel, (event) => handler(event.payload));
    logger.debug(`[events] subscribed channel=${channel} sub=${sub} (custom)`);
    return () => {
      logger.debug(`[events] unsubscribed channel=${channel} sub=${sub} (custom)`);
      unlisten();
    };
  } catch (err) {
    logger.error(`[events] subscribe failed channel=${channel} sub=${sub} (custom):`, err);
    throw err;
  }
}
