/**
 * Enhanced Tauri invoke wrapper with timeout, abort, and retry support.
 *
 * This is the single exit point for all backend IPC calls.
 * Higher-level service code should use this instead of raw `@tauri-apps/api/core`.
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { AbortedError, TimeoutError, TransportError } from "./errors";

export interface InvokeOptions {
  /** Abort signal — cancel the request from the caller side. */
  signal?: AbortSignal;
  /** Per-call timeout in ms (default: no timeout). */
  timeoutMs?: number;
  /**
   * Retry count on transient failures (default: 0 = no retry).
   * Only retries on INVOKE_FAILED, never on TIMEOUT or ABORTED.
   */
  retries?: number;
  /** Base delay between retries in ms (exponential backoff). */
  retryDelayMs?: number;
}

const DEFAULT_RETRY_DELAY_MS = 300;

let requestCounter = 0;
const inflightCommands = new Map<number, { command: string; startedAt: number }>();

export function getInflightCommands(): ReadonlyMap<
  number,
  { command: string; startedAt: number }
> {
  return inflightCommands;
}

export async function invoke<T = void>(
  command: string,
  args?: Record<string, unknown>,
  opts?: InvokeOptions,
): Promise<T> {
  const { signal, timeoutMs, retries = 0, retryDelayMs = DEFAULT_RETRY_DELAY_MS } = opts ?? {};

  let lastError: unknown;

  for (let attempt = 0; attempt <= retries; attempt++) {
    if (signal?.aborted) throw new AbortedError(command);

    try {
      return await invokeOnce<T>(command, args, signal, timeoutMs);
    } catch (err) {
      lastError = err;
      if (err instanceof AbortedError || err instanceof TimeoutError) throw err;
      if (attempt < retries) {
        await sleep(retryDelayMs * 2 ** attempt);
      }
    }
  }

  throw lastError;
}

async function invokeOnce<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  signal: AbortSignal | undefined,
  timeoutMs: number | undefined,
): Promise<T> {
  const id = ++requestCounter;
  inflightCommands.set(id, { command, startedAt: Date.now() });

  try {
    const raceEntries: Promise<T>[] = [
      tauriInvoke<T>(command, args).catch((err) => {
        throw new TransportError(
          "INVOKE_FAILED",
          `[Transport] ${command}: ${err instanceof Error ? err.message : String(err)}`,
          command,
          err,
        );
      }),
    ];

    if (timeoutMs !== undefined) {
      raceEntries.push(
        new Promise<never>((_, reject) => {
          setTimeout(() => reject(new TimeoutError(command, timeoutMs)), timeoutMs);
        }),
      );
    }

    if (signal) {
      raceEntries.push(
        new Promise<never>((_, reject) => {
          const onAbort = () => reject(new AbortedError(command));
          if (signal.aborted) {
            onAbort();
            return;
          }
          signal.addEventListener("abort", onAbort, { once: true });
        }),
      );
    }

    return await Promise.race(raceEntries);
  } finally {
    inflightCommands.delete(id);
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
