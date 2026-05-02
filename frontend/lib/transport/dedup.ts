/**
 * Request deduplication for transport-level invoke calls.
 *
 * When multiple components request the same command+args simultaneously,
 * only one actual IPC call is made; all callers share the same Promise.
 */

import { type InvokeOptions, invoke } from "./invoke";

const inflight = new Map<string, Promise<unknown>>();

function buildKey(command: string, args?: Record<string, unknown>): string {
  return args ? `${command}::${JSON.stringify(args)}` : command;
}

/**
 * Deduplicated invoke — concurrent calls with the same command+args
 * piggyback on a single in-flight request.
 */
export async function dedupInvoke<T = void>(
  command: string,
  args?: Record<string, unknown>,
  opts?: InvokeOptions
): Promise<T> {
  const key = buildKey(command, args);

  const existing = inflight.get(key);
  if (existing) return existing as Promise<T>;

  const promise = invoke<T>(command, args, opts).finally(() => {
    inflight.delete(key);
  });

  inflight.set(key, promise);
  return promise;
}
