/**
 * State hook for one credential-capture session.
 *
 * Lifecycle:
 *
 *   idle ──start(tool,group)──► confirm dialog open
 *                                  │
 *                  user clicks ────┴──► captureStart() → waiting_login
 *                  cancel               │
 *                  │                    ├─ navigating ─► extracting ─► captured/partial
 *                  ▼                    │
 *                idle               (timeout/failed/cancelled)
 *
 * The hook owns:
 *   - The `pendingRequest` between user click and dialog confirm.
 *   - Live `session` snapshot, refreshed from the
 *     `"integration-capture"` Tauri event listener.
 *   - The 1s countdown driving `remainingSecs`.
 *
 * Consumers wire the returned `start` to the ⚡ button, render
 * `<CaptureConfirmDialog>` controlled by `confirmOpen` /
 * `proceedAfterConfirm`, and render `<CaptureStatusToast>` keyed by
 * `session` / `remainingSecs`.
 *
 * Pass an `onTerminal` callback to refresh the parent form when the
 * session ends in `captured` / `partial` — the parent form's snapshot
 * is what surfaces the new "已配置 / configured" badge in the UI.
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  type CaptureEventPayload,
  type CaptureSessionInfo,
  type CaptureState,
  captureCancel,
  captureStart,
} from "@/lib/api/integrations";

const CAPTURE_EVENT = "integration-capture" as const;

export interface UseCaptureSessionOptions {
  /**
   * Called whenever the session transitions into `captured` or
   * `partial`. Parents pass `useIntegrationGroup`'s `refresh` here so
   * the form picks up the freshly written field values.
   */
  onTerminalSuccess?: () => void;
}

export interface UseCaptureSessionResult {
  /** Live `state`. `null` when no session has been started yet. */
  state: CaptureState | null;
  /** Full session snapshot from the last `_start` / event tick. */
  session: CaptureSessionInfo | null;
  /** Seconds remaining until TTL expires (UI countdown). */
  remainingSecs: number;
  /** Latest event payload received (used by parent components to
   *  detect transition edges like "just landed in captured"). */
  lastEvent: CaptureEventPayload | null;
  /** Error from the most recent IPC call. Cleared on next `start`. */
  startError: string | null;
  /** Whether to render the confirm dialog. */
  confirmOpen: boolean;
  setConfirmOpen: (v: boolean) => void;
  /** Captured between `start()` and user-confirm. `null` after fire. */
  pendingRequest: { toolId: string; groupId: string } | null;
  /** Open the confirm dialog for `(toolId, groupId)`. */
  start: (toolId: string, groupId: string) => void;
  /** Actually fire `captureStart` after user confirms. */
  proceedAfterConfirm: () => Promise<void>;
  /** Cancel an in-flight session. No-op when no session is active. */
  cancel: () => Promise<void>;
}

export function useCaptureSession(opts: UseCaptureSessionOptions = {}): UseCaptureSessionResult {
  const { onTerminalSuccess } = opts;

  const [session, setSession] = useState<CaptureSessionInfo | null>(null);
  const [lastEvent, setLastEvent] = useState<CaptureEventPayload | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [pendingRequest, setPendingRequest] = useState<{
    toolId: string;
    groupId: string;
  } | null>(null);
  const [remainingSecs, setRemainingSecs] = useState(0);

  // Latest session id stored in a ref so the global event listener
  // can compare without re-subscribing on every state change.
  const sessionIdRef = useRef<string | null>(null);
  useEffect(() => {
    sessionIdRef.current = session?.session_id ?? null;
  }, [session]);

  // Stable ref for the parent's onTerminalSuccess callback so changing
  // it (e.g. due to useIntegrationGroup re-render) doesn't re-subscribe
  // the event listener.
  const onTerminalRef = useRef(onTerminalSuccess);
  useEffect(() => {
    onTerminalRef.current = onTerminalSuccess;
  }, [onTerminalSuccess]);

  const start = useCallback((toolId: string, groupId: string) => {
    setStartError(null);
    setPendingRequest({ toolId, groupId });
    setConfirmOpen(true);
  }, []);

  const proceedAfterConfirm = useCallback(async () => {
    if (!pendingRequest) return;
    setConfirmOpen(false);
    try {
      const info = await captureStart(pendingRequest);
      setSession(info);
      setLastEvent(null);
      setStartError(null);
    } catch (err) {
      setSession(null);
      setStartError(err instanceof Error ? err.message : String(err));
    } finally {
      setPendingRequest(null);
    }
  }, [pendingRequest]);

  const cancel = useCallback(async () => {
    if (!session) {
      setConfirmOpen(false);
      setPendingRequest(null);
      return;
    }
    try {
      await captureCancel({ sessionId: session.session_id });
    } catch {
      // Cancel failure isn't fatal — the engine may have already
      // transitioned us into a terminal state via the TTL watcher,
      // and the next event will reflect that.
    }
  }, [session]);

  // Subscribe to the global `integration-capture` event. We listen
  // once per mount; the ref check inside the handler filters out
  // events for sessions that aren't ours.
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    (async () => {
      try {
        unlisten = await listen<CaptureEventPayload>(CAPTURE_EVENT, (evt) => {
          if (cancelled) return;
          const sid = sessionIdRef.current;
          if (!sid || evt.payload.session_id !== sid) return;
          setLastEvent(evt.payload);
          setSession((prev) =>
            prev
              ? {
                  ...prev,
                  state: evt.payload.state,
                  captured_fields: evt.payload.captured_fields ?? [],
                  failed_rules: evt.payload.failed_rules ?? [],
                  error_message: evt.payload.error_message,
                  updated_at: Date.now(),
                }
              : prev
          );
          if (
            (evt.payload.state === "captured" || evt.payload.state === "partial") &&
            onTerminalRef.current
          ) {
            onTerminalRef.current();
          }
        });
      } catch {
        // If the event listener fails to register (e.g. running in a
        // non-Tauri context like Vitest), the hook still functions —
        // the user just won't get push updates. Status polling via
        // `captureStatus` remains available as a fallback.
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  // 1Hz countdown timer driven off the live `expires_at`.
  useEffect(() => {
    if (!session || !session.expires_at) {
      setRemainingSecs(0);
      return;
    }
    const tick = () => {
      const expires = session.expires_at as number;
      const remaining = Math.max(0, Math.ceil((expires - Date.now()) / 1000));
      setRemainingSecs(remaining);
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [session]);

  return {
    state: session?.state ?? lastEvent?.state ?? null,
    session,
    remainingSecs,
    lastEvent,
    startError,
    confirmOpen,
    setConfirmOpen,
    pendingRequest,
    start,
    proceedAfterConfirm,
    cancel,
  };
}
