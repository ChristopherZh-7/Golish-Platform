/**
 * Phase B GridTerminal — renders the virtual terminal grid streamed by
 * the backend without touching xterm.js. See
 * `docs/design/2026-05-15-grid-terminal-phase-b.md` for the
 * end-to-end architecture; this component is the React side.
 *
 * Responsibilities (D3 scope):
 *  - Subscribe to `terminal_grid_update` for the active session via
 *    [`useGridState`].
 *  - Render the cached grid as `<div class="gt-row">` × `<span>` per
 *    cell.
 *  - Show a "Waiting for first frame…" placeholder until the backend
 *    has shipped a baseline (alt-screen sessions only emit a grid; if
 *    you mount this component for a non-alt session you'll sit at the
 *    placeholder indefinitely, which is intentional — non-alt
 *    sessions belong in `UnifiedTimeline`).
 *
 * Out of scope for D3 (lands in D4):
 *  - Keyboard event → ANSI byte translation
 *  - ResizeObserver → `pty_resize_grid` integration
 *  - Focus / blur reporting
 */

import { memo, useCallback, useEffect, useRef, useState } from "react";
import "@/styles/grid-terminal.css";
import { imeGetSource, imeSetSource } from "@/lib/api/shell";
import {
  clearTerminalAutoFocusSuppression,
  isTerminalAutoFocusSuppressed,
} from "@/lib/terminal/terminalAutoFocus";
import { GridRow } from "./GridRow";
import { useGridKeyboard } from "./useGridKeyboard";
import { useGridResize } from "./useGridResize";
import { useGridState } from "./useGridState";

interface GridTerminalProps {
  sessionId: string;
  /**
   * Disable the grid subscription + keyboard / resize handlers.
   * Useful when the parent already knows the session is not on
   * alt-screen and wants the GridTerminal mount silent. Defaults to
   * `true`.
   */
  enabled?: boolean;
  /**
   * Should we autofocus the container on mount? Defaults to `true` so
   * the first key after `vim` launches is captured immediately. Pass
   * `false` for tests / popovers where stealing focus would hurt.
   */
  autoFocus?: boolean;
}

export const GridTerminal = memo(function GridTerminal({
  sessionId,
  enabled = true,
  autoFocus = true,
}: GridTerminalProps) {
  const snapshot = useGridState(sessionId, enabled);
  const containerRef = useRef<HTMLDivElement>(null);
  const [isFocused, setIsFocused] = useState(false);

  useGridKeyboard({
    sessionId,
    containerRef,
    appCursorMode: snapshot.appCursorMode,
    enabled,
  });

  useGridResize({ sessionId, containerRef, enabled });

  // Pull focus to the container on mount so a freshly launched `vim`
  // doesn't drop the first few keystrokes into the surrounding chrome.
  // Re-runs if the session changes (split panes / tab switch) but not
  // on every grid update.
  useEffect(() => {
    if (!autoFocus || !enabled) return;
    if (isTerminalAutoFocusSuppressed(sessionId)) return;
    const el = containerRef.current;
    if (!el) return;
    // Defer a frame so React's commit + browser focus race lands in
    // the right order; otherwise auto-focus loses to a parent
    // focusing itself on mount.
    const handle = requestAnimationFrame(() => {
      el.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(handle);
  }, [autoFocus, enabled, sessionId]);

  // While a non-ABC IME (e.g. macOS pinyin) is active, every printable
  // ASCII keystroke gets transformed before it reaches the DOM —
  // notably `:` becomes `：` (U+FF1A) under pinyin, which vim then
  // doesn't recognise as the command-mode trigger. Mirror the
  // `useUnifiedInputState.handleFocus/handleBlur` strategy: stash the
  // current input source, flip to ABC on focus, restore on blur.
  const prevImeSourceRef = useRef<string | null>(null);
  const handleFocus = useCallback(() => {
    setIsFocused(true);
    clearTerminalAutoFocusSuppression(sessionId);
    imeGetSource()
      .then((src) => {
        if (src && src !== "com.apple.keylayout.ABC") {
          prevImeSourceRef.current = src;
          imeSetSource("com.apple.keylayout.ABC").catch(() => {});
        }
      })
      .catch(() => {});
  }, [sessionId]);
  const handleBlur = useCallback(() => {
    setIsFocused(false);
    if (prevImeSourceRef.current) {
      imeSetSource(prevImeSourceRef.current).catch(() => {});
      prevImeSourceRef.current = null;
    }
  }, []);
  // Clicking anywhere in the grid should focus the container so the
  // keymap hook starts seeing keystrokes.
  const handleMouseDown = useCallback(() => {
    clearTerminalAutoFocusSuppression(sessionId);
    containerRef.current?.focus({ preventScroll: true });
  }, [sessionId]);

  if (snapshot.rowCount === 0) {
    return (
      <div
        ref={containerRef}
        className="gt-root gt-empty"
        role="application"
        aria-label="Terminal grid (initialising)"
        data-testid="grid-terminal-empty"
        // biome-ignore lint/a11y/noNoninteractiveTabindex: terminal grid is a self-managed keyboard surface (role=application); focus is required for keystroke capture
        tabIndex={0}
        onFocus={handleFocus}
        onBlur={handleBlur}
        onMouseDown={handleMouseDown}
      >
        <span className="gt-empty-hint">Waiting for first frame…</span>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className="gt-root"
      role="application"
      aria-label="Terminal grid"
      data-testid="grid-terminal"
      data-rev={snapshot.serverRev}
      data-cols={snapshot.cols}
      data-rows={snapshot.rowCount}
      data-focused={isFocused ? "true" : "false"}
      data-app-cursor={snapshot.appCursorMode ? "true" : "false"}
      // biome-ignore lint/a11y/noNoninteractiveTabindex: terminal grid is a self-managed keyboard surface (role=application); focus is required for keystroke capture
      tabIndex={0}
      onFocus={handleFocus}
      onBlur={handleBlur}
      onMouseDown={handleMouseDown}
    >
      {snapshot.rows.map((cells, y) => (
        <GridRow
          // y is stable across the lifetime of the grid (always
          // 0..rowCount), so it's a safe key.
          key={y}
          cells={cells}
          cursorX={snapshot.cursor.y === y ? snapshot.cursor.x : -1}
          cursorVisible={snapshot.cursor.visible}
        />
      ))}
    </div>
  );
});
