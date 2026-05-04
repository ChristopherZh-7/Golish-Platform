/**
 * Transport Layer — single entry point for all backend IPC communication.
 *
 * Provides:
 * - `invoke()`      Enhanced Tauri invoke with timeout, abort, retry
 * - `dedupInvoke()` Same as invoke but deduplicates concurrent identical calls
 * - Error types     TransportError, TimeoutError, AbortedError
 *
 * For event subscriptions, use `onEvent` / `onCustomEvent` from `@/lib/events`.
 */

export { dedupInvoke } from "./dedup";
export {
  AbortedError,
  isTimeoutError,
  isTransportError,
  TimeoutError,
  TransportError,
  type TransportErrorCode,
} from "./errors";
export { getInflightCommands, type InvokeOptions, invoke } from "./invoke";
