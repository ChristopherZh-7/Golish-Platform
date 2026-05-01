/**
 * Transport Layer — single entry point for all backend IPC communication.
 *
 * Provides:
 * - `invoke()`     Enhanced Tauri invoke with timeout, abort, retry
 * - `dedupInvoke()` Same as invoke but deduplicates concurrent identical calls
 * - `listen()`      Typed event listener wrapper
 * - Error types     TransportError, TimeoutError, AbortedError
 */

export { dedupInvoke } from "./dedup";
export {
  AbortedError,
  TimeoutError,
  TransportError,
  isTimeoutError,
  isTransportError,
  type TransportErrorCode,
} from "./errors";
export { getInflightCommands, invoke, type InvokeOptions } from "./invoke";
export { listen, type UnlistenFn } from "./listen";
