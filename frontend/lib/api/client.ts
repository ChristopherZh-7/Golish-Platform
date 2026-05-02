import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { listen as tauriListen } from "../tauri-listen";

/**
 * `ApiError` carries the typed Tauri command name AND the auto-
 * generated frontend trace id so that errors can be threaded back
 * through `lib/logger` to `~/.golish/frontend.log` and grep'd by
 * trace id later.
 */
export class ApiError extends Error {
  constructor(
    public readonly command: string,
    public readonly cause: unknown,
    public readonly traceId: string
  ) {
    const msg = cause instanceof Error ? cause.message : String(cause);
    super(`[API trace=${traceId}] ${command}: ${msg}`);
    this.name = "ApiError";
  }
}

let requestCounter = 0;
const inflightCommands = new Map<number, { command: string; startedAt: number; traceId: string }>();

export function getInflightCommands(): ReadonlyMap<
  number,
  { command: string; startedAt: number; traceId: string }
> {
  return inflightCommands;
}

/**
 * Generate a short hex trace id for one IPC call. 8 chars is enough
 * to be locally unique across an interactive session (the Tauri
 * shell process is single-user) while staying greppable.
 */
function generateTraceId(): string {
  const a = Math.floor(Math.random() * 0xffffffff)
    .toString(16)
    .padStart(8, "0");
  return a;
}

/**
 * Lightweight thread-local store of "currently active" trace ids,
 * exposed so that `lib/logger.ts` (or any other observer) can
 * attach the trace id of the most recent in-flight IPC to its log
 * payload without explicit plumbing through every callsite.
 */
let lastTraceId: string | null = null;

export function getLastTraceId(): string | null {
  return lastTraceId;
}

export async function invoke<T = void>(
  command: string,
  args?: Record<string, unknown>
): Promise<T> {
  const id = ++requestCounter;
  const traceId = generateTraceId();
  lastTraceId = traceId;
  inflightCommands.set(id, { command, startedAt: Date.now(), traceId });
  try {
    return await tauriInvoke<T>(command, args);
  } catch (err) {
    throw new ApiError(command, err, traceId);
  } finally {
    inflightCommands.delete(id);
  }
}

export function listen<T>(channel: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return tauriListen<T>(channel, (event) => handler(event.payload));
}
