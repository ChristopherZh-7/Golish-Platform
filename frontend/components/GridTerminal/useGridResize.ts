/**
 * Keeps the backend GridTerminal (and the underlying PTY) in sync
 * with the visible viewport size of the React container.
 *
 * Strategy:
 *  1. Measure the rendered cell size once per mount by stashing a
 *     hidden probe `<span>X</span>` into the container, reading
 *     `getBoundingClientRect`, then removing it. We don't keep the
 *     probe around because alacritty's column count is "how many
 *     glyphs fit" and a permanent invisible span risks bumping the
 *     baseline.
 *  2. On every `ResizeObserver` callback, compute `cols × rows` from
 *     the container's current client size.
 *  3. Debounce by `RESIZE_DEBOUNCE_MS` so rapid layout shifts during
 *     pane splits / window resizes don't spam the IPC bridge.
 *  4. Call `ptyResize` (drives the shell's SIGWINCH so `tput cols`
 *     returns the right thing) and `ptyResizeGrid` (re-shapes the
 *     virtual grid in alacritty_terminal) together — they target
 *     different layers but always move in lockstep.
 */

import { type RefObject, useEffect, useRef } from "react";
import { ptyResize, ptyResizeGrid } from "@/lib/api/pty";
import { logger } from "@/lib/logger";

interface UseGridResizeOptions {
  sessionId: string;
  containerRef: RefObject<HTMLElement | null>;
  /** Skip all measurement / notification work. Defaults to `true`. */
  enabled?: boolean;
}

const RESIZE_DEBOUNCE_MS = 100;
const MIN_COLS = 2;
const MIN_ROWS = 1;
/** Don't let the container's first-paint zero size send a `cols=0`
 *  resize to the PTY (Alacritty clamps but `ptyResize` does not). */
const MAX_RESONABLE_COLS = 2000;
const MAX_RESONABLE_ROWS = 1000;

interface CellMetrics {
  width: number;
  height: number;
}

function measureCell(host: HTMLElement): CellMetrics | null {
  // We use `M` because it tends to be the widest glyph in any
  // monospace font and avoids the kerning surprises of digits / `i`.
  const probe = document.createElement("span");
  probe.textContent = "M";
  probe.style.position = "absolute";
  probe.style.visibility = "hidden";
  probe.style.pointerEvents = "none";
  probe.style.whiteSpace = "pre";
  host.appendChild(probe);
  const rect = probe.getBoundingClientRect();
  host.removeChild(probe);
  if (rect.width <= 0 || rect.height <= 0) return null;
  return { width: rect.width, height: rect.height };
}

export function useGridResize({
  sessionId,
  containerRef,
  enabled = true,
}: UseGridResizeOptions): void {
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastDimsRef = useRef<{ cols: number; rows: number } | null>(null);

  useEffect(() => {
    if (!enabled) return;
    const el = containerRef.current;
    if (!el) return;
    if (typeof ResizeObserver === "undefined") return;

    const flush = (trigger: "mount" | "observer") => {
      const target = containerRef.current;
      if (!target) return;
      const metrics = measureCell(target);
      if (!metrics) return;

      const clientWidth = target.clientWidth;
      const clientHeight = target.clientHeight;
      const cols = Math.max(
        MIN_COLS,
        Math.min(MAX_RESONABLE_COLS, Math.floor(clientWidth / metrics.width))
      );
      const rows = Math.max(
        MIN_ROWS,
        Math.min(MAX_RESONABLE_ROWS, Math.floor(clientHeight / metrics.height))
      );

      // #region agent log
      const debugPayload = {
        sessionId,
        trigger,
        containerClassName: target.className,
        clientWidth,
        clientHeight,
        cellWidth: metrics.width,
        cellHeight: metrics.height,
        cols,
        rows,
      };
      // eslint-disable-next-line no-console
      console.info("[grid-debug][resize-flush]", JSON.stringify(debugPayload));
      fetch("http://127.0.0.1:7440/ingest/f9f2cacd-c1f1-479f-8225-b4a5be2ee53c", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-Debug-Session-Id": "900b3a" },
        body: JSON.stringify({
          sessionId: "900b3a",
          location: "useGridResize.ts:flush",
          message: "resize-flush",
          data: debugPayload,
          timestamp: Date.now(),
          hypothesisId: "2",
        }),
      }).catch(() => {});
      // #endregion

      const prev = lastDimsRef.current;
      if (prev && prev.cols === cols && prev.rows === rows) return;
      lastDimsRef.current = { cols, rows };

      // `ptyResize` uses (rows, cols) — preserves the legacy
      // signature so we don't churn every existing caller. The
      // GridTerminal counterpart takes (cols, rows) because that
      // mirrors `cargo` / CSS conventions.
      ptyResize(sessionId, rows, cols).catch((err) => {
        logger.warn("[GridTerminal] ptyResize failed:", err);
      });
      ptyResizeGrid(sessionId, cols, rows).catch((err) => {
        logger.warn("[GridTerminal] ptyResizeGrid failed:", err);
      });
    };

    const observer = new ResizeObserver(() => {
      if (debounceRef.current !== null) clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => {
        debounceRef.current = null;
        flush("observer");
      }, RESIZE_DEBOUNCE_MS);
    });
    observer.observe(el);

    // Kick a baseline measurement on mount so the very first frame
    // sees the right dimensions even before any resize happens.
    flush("mount");

    return () => {
      observer.disconnect();
      if (debounceRef.current !== null) {
        clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
    };
  }, [sessionId, containerRef, enabled]);
}
