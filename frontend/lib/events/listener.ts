/**
 * Type-safe event listener and emitter that enforces correct payload
 * types based on the channel name.
 *
 * Subscribe / emit operations are tagged with a short hex `sub` id,
 * logged at `debug` level on the subscribe / emit action and at
 * `error` level on failure. Per-event logging is intentionally omitted
 * to avoid flooding the console on hot channels (terminal_output,
 * ai-event, terminal output).
 */

import { emit as rawEmit, type UnlistenFn } from "@tauri-apps/api/event";
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

/**
 * Emit an event on a channel registered in `EventPayloadMap` with a
 * compile-time payload-type check. The `payload` argument is required
 * and must match the channel's declared payload shape.
 *
 * ```ts
 * await sendEvent("detached-window-closed", { session_id: "abc" });
 * ```
 */
export async function sendEvent<K extends keyof EventPayloadMap>(
  channel: K,
  payload: EventPayloadMap[K]
): Promise<void> {
  const sub = generateSubId();
  try {
    logger.debug(`[events] emit channel=${channel} sub=${sub}`);
    await rawEmit(channel, payload);
  } catch (err) {
    logger.error(`[events] emit failed channel=${channel} sub=${sub}:`, err);
    throw err;
  }
}

/**
 * Emit an event on an untyped channel. Prefer `sendEvent` for channels
 * defined in `EventPayloadMap`. `payload` is optional; pass `undefined`
 * or omit for payload-less signalling events like `targets-changed`.
 */
export async function sendCustomEvent<T = unknown>(channel: string, payload?: T): Promise<void> {
  const sub = generateSubId();
  try {
    logger.debug(`[events] emit channel=${channel} sub=${sub} (custom)`);
    await rawEmit(channel, payload);
  } catch (err) {
    logger.error(`[events] emit failed channel=${channel} sub=${sub} (custom):`, err);
    throw err;
  }
}
