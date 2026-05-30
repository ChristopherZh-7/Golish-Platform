import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export interface GolishErrorShape {
  code: string;
  message: string;
}

/**
 * Tauri rejects command promises with whatever the Rust side serialized.
 * After P0-1, `GolishError` serializes to `{ code, message }`; legacy commands
 * that still return `Result<T, String>` reject with a bare string, and JS
 * runtime failures reject with an `Error`. Normalize all three into
 * `{ code, message }` so callers can branch on a stable `code`.
 */
export function parseGolishError(cause: unknown): GolishErrorShape {
  if (cause && typeof cause === "object" && "code" in cause && "message" in cause) {
    const c = cause as Record<string, unknown>;
    if (typeof c.code === "string" && typeof c.message === "string") {
      return { code: c.code, message: c.message };
    }
  }
  const message = cause instanceof Error ? cause.message : String(cause);
  return { code: "UNKNOWN", message };
}

/**
 * `ApiError` carries the typed Tauri command name AND the auto-
 * generated frontend trace id so that errors can be threaded back
 * through `lib/logger` to `~/.golish/frontend.log` and grep'd by
 * trace id later. It also exposes the backend error `code` (see
 * `lib/api/error-codes.ts`) so callers can branch + translate.
 */
export class ApiError extends Error {
  /** Stable backend error code (see lib/api/error-codes.ts), or "UNKNOWN". */
  public readonly code: string;

  constructor(
    public readonly command: string,
    public readonly cause: unknown,
    public readonly traceId: string
  ) {
    const { code, message } = parseGolishError(cause);
    super(`[API trace=${traceId}] ${command}: ${message}`);
    this.name = "ApiError";
    this.code = code;
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

// `listen` was removed in QW6 (2026-05) — it had zero call sites after
// `lib/ai/session.ts::onAiEvent` migrated to the typed `onEvent` from
// `@/lib/events`. New event subscriptions should use `onEvent` for
// compile-time channel typing + structured subscription logs.
