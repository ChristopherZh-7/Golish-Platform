/**
 * Unified transport-level error types.
 *
 * All IPC errors (Tauri invoke failures, timeouts, network issues)
 * are normalized into these types so services can handle them
 * without knowing the underlying transport.
 */

export class TransportError extends Error {
  readonly code: TransportErrorCode;

  constructor(
    code: TransportErrorCode,
    message: string,
    public readonly command: string,
    public readonly cause?: unknown,
  ) {
    super(message);
    this.name = "TransportError";
    this.code = code;
  }
}

export type TransportErrorCode =
  | "INVOKE_FAILED"
  | "TIMEOUT"
  | "ABORTED"
  | "BACKEND_UNREACHABLE";

export class TimeoutError extends TransportError {
  constructor(command: string, timeoutMs: number) {
    super("TIMEOUT", `[Transport] ${command}: timed out after ${timeoutMs}ms`, command);
    this.name = "TimeoutError";
  }
}

export class AbortedError extends TransportError {
  constructor(command: string) {
    super("ABORTED", `[Transport] ${command}: request aborted`, command);
    this.name = "AbortedError";
  }
}

export function isTransportError(err: unknown): err is TransportError {
  return err instanceof TransportError;
}

export function isTimeoutError(err: unknown): err is TimeoutError {
  return err instanceof TimeoutError;
}
