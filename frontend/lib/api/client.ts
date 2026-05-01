import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { listen as tauriListen } from "../tauri-listen";

export class ApiError extends Error {
  constructor(
    public readonly command: string,
    public readonly cause: unknown,
  ) {
    const msg =
      cause instanceof Error ? cause.message : String(cause);
    super(`[API] ${command}: ${msg}`);
    this.name = "ApiError";
  }
}

let requestCounter = 0;
const inflightCommands = new Map<number, { command: string; startedAt: number }>();

export function getInflightCommands(): ReadonlyMap<number, { command: string; startedAt: number }> {
  return inflightCommands;
}

export async function invoke<T = void>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const id = ++requestCounter;
  inflightCommands.set(id, { command, startedAt: Date.now() });
  try {
    return await tauriInvoke<T>(command, args);
  } catch (err) {
    throw new ApiError(command, err);
  } finally {
    inflightCommands.delete(id);
  }
}

export function listen<T>(
  channel: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  return tauriListen<T>(channel, (event) => handler(event.payload));
}
