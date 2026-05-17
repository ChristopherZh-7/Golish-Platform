/**
 * Wires DOM keyboard / paste / composition events on a GridTerminal
 * container to PTY writes.
 *
 * Splits responsibilities with [`keymap.ts`] — that module knows what
 * bytes a given key should produce; this hook knows when / where to
 * listen and how to call `ptyWrite`. Keeping them apart means the
 * byte-mapping table is unit-tested without any DOM mocking.
 *
 * The hook does *not* attach listeners directly to `window` — it
 * attaches to the container ref, so multiple GridTerminals in
 * split panes don't clobber each other when the focus shifts.
 */

import { type RefObject, useCallback, useEffect, useRef } from "react";
import { ptyWrite } from "@/lib/api/pty";
import { logger } from "@/lib/logger";
import { bracketedPaste, reactKeyEventToAnsiBytes } from "./keymap";

interface UseGridKeyboardOptions {
  sessionId: string;
  containerRef: RefObject<HTMLElement | null>;
  /**
   * Whether the underlying terminal is in application-cursor mode
   * (DEC mode 1). Arrow keys send `ESC O <X>` instead of `ESC [ <X>`
   * when this is true. Sourced from
   * `TerminalGridUpdatePayload.app_cursor_mode`.
   */
  appCursorMode: boolean;
  /** Set to `false` to detach all listeners (e.g. when the parent
   *  pane is not the focused pane). Defaults to `true`. */
  enabled?: boolean;
}

export function useGridKeyboard({
  sessionId,
  containerRef,
  appCursorMode,
  enabled = true,
}: UseGridKeyboardOptions): void {
  // Latest values that the closure-captured listeners reach for. The
  // listeners themselves are attached once per `enabled`/`sessionId`
  // change so a mid-stream `appCursorMode` flip doesn't tear them
  // down (which would lose any in-flight composition state).
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;
  const appCursorRef = useRef(appCursorMode);
  appCursorRef.current = appCursorMode;

  const sendBytes = useCallback((bytes: string) => {
    if (bytes.length === 0) return;
    ptyWrite(sessionIdRef.current, bytes).catch((err) => {
      logger.warn("[GridTerminal] ptyWrite failed:", err);
    });
  }, []);

  useEffect(() => {
    if (!enabled) return;
    const el = containerRef.current;
    if (!el) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      // We synthesise a React-like view here so the keymap doesn't
      // care which event flavour came through. `nativeEvent` is the
      // native event itself in both cases — the test bench just
      // passes a plain object instead.
      const bytes = reactKeyEventToAnsiBytes(
        {
          key: event.key,
          code: event.code,
          ctrlKey: event.ctrlKey,
          altKey: event.altKey,
          shiftKey: event.shiftKey,
          metaKey: event.metaKey,
          nativeEvent: event,
        } as unknown as React.KeyboardEvent,
        appCursorRef.current
      );
      // eslint-disable-next-line no-console
      console.info(
        "[grid-debug][keydown]",
        JSON.stringify({
          key: event.key,
          ctrl: event.ctrlKey,
          alt: event.altKey,
          meta: event.metaKey,
          mapped: bytes === null ? null : bytes.length,
          willSend: bytes !== null,
        })
      );
      // #region agent log
      fetch("http://127.0.0.1:7440/ingest/f9f2cacd-c1f1-479f-8225-b4a5be2ee53c", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-Debug-Session-Id": "900b3a" },
        body: JSON.stringify({
          sessionId: "900b3a",
          location: "useGridKeyboard.ts:handleKeyDown",
          message: "grid-keydown",
          data: {
            ptySessionId: sessionIdRef.current,
            key: event.key,
            code: event.code,
            ctrl: event.ctrlKey,
            alt: event.altKey,
            meta: event.metaKey,
            mappedLen: bytes === null ? null : bytes.length,
            mappedFirstByte: bytes && bytes.length > 0 ? bytes.charCodeAt(0) : null,
            willSend: bytes !== null,
            activeElementTag: (document.activeElement as HTMLElement | null)?.tagName ?? null,
            activeIsGtRoot:
              (document.activeElement as HTMLElement | null)?.classList?.contains("gt-root") ??
              false,
            target: (event.target as HTMLElement | null)?.tagName ?? null,
          },
          timestamp: Date.now(),
          hypothesisId: "F1,F2,F3",
        }),
      }).catch(() => {});
      // #endregion
      if (bytes === null) return;
      event.preventDefault();
      event.stopPropagation();
      sendBytes(bytes);
    };

    const handlePaste = (event: ClipboardEvent) => {
      const text = event.clipboardData?.getData("text") ?? "";
      if (!text) return;
      event.preventDefault();
      event.stopPropagation();
      sendBytes(bracketedPaste(text));
    };

    const handleCompositionEnd = (event: CompositionEvent) => {
      const finalised = event.data;
      if (!finalised) return;
      sendBytes(finalised);
    };

    el.addEventListener("keydown", handleKeyDown);
    el.addEventListener("paste", handlePaste);
    el.addEventListener("compositionend", handleCompositionEnd);

    return () => {
      el.removeEventListener("keydown", handleKeyDown);
      el.removeEventListener("paste", handlePaste);
      el.removeEventListener("compositionend", handleCompositionEnd);
    };
  }, [enabled, containerRef, sendBytes]);
}
