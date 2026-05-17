/**
 * Subscribes to `terminal_grid_update` events for a given session and
 * maintains a local grid mirror that the `GridTerminal` React component
 * renders. See `docs/design/2026-05-15-grid-terminal-phase-b.md` §3 for
 * the wire protocol and §5.2 for the rendering strategy.
 *
 * Responsibilities:
 *  - Apply full snapshots wholesale, replace the cached grid.
 *  - Apply diff snapshots by overwriting only the rows in
 *    `dirty_rows`.
 *  - Detect non-contiguous `rev` jumps (event dropped over IPC) and
 *    request a fresh `pty_request_grid_snapshot` to recover.
 *  - Surface a stable `cols × rows` 2D array plus the cursor — no
 *    backend types leak past this hook.
 */

import { useEffect, useReducer, useRef } from "react";
import { ptyRequestGridSnapshot } from "@/lib/api/pty";
import { onEvent } from "@/lib/events";
import type {
  GridCellPayload,
  GridCursorPayload,
  TerminalGridUpdatePayload,
} from "@/lib/events/payloads";
import { logger } from "@/lib/logger";

/** Read-only snapshot the rendering component consumes. */
export interface GridSnapshot {
  /** `rows` outer length × `cols` inner length grid. */
  rows: GridCellPayload[][];
  cursor: GridCursorPayload;
  cols: number;
  rowCount: number;
  /** Bumped any time `rows` or `cursor` reference changes — used as a
   *  hint for memoised consumers (so they don't depend on the array
   *  identity directly). */
  revision: number;
  /** Last `rev` we successfully merged. `0` until the first frame. */
  serverRev: number;
  /** Whether DEC `APP_CURSOR` mode is active on the backend; consumed
   *  by `useGridKeyboard` so arrow keys emit the right escape form. */
  appCursorMode: boolean;
}

interface InternalState {
  snapshot: GridSnapshot;
  // Used by the reducer to detect non-contiguous deliveries. We keep
  // this outside `snapshot` so consumers don't accidentally re-render
  // when only the bookkeeping changes.
  lastRev: number;
}

const EMPTY_CURSOR: GridCursorPayload = {
  x: 0,
  y: 0,
  visible: false,
  style: "block",
};

function emptyGrid(cols: number, rows: number): GridCellPayload[][] {
  return Array.from({ length: rows }, () => Array.from({ length: cols }, () => createBlankCell()));
}

function createBlankCell(): GridCellPayload {
  return {
    ch: " ",
    fg: { kind: "default" },
    bg: { kind: "default" },
    attrs: 0,
  };
}

function createInitialState(): InternalState {
  return {
    snapshot: {
      rows: [],
      cursor: EMPTY_CURSOR,
      cols: 0,
      rowCount: 0,
      revision: 0,
      serverRev: 0,
      appCursorMode: false,
    },
    lastRev: 0,
  };
}

type GridAction =
  | { type: "full"; payload: TerminalGridUpdatePayload }
  | { type: "diff"; payload: TerminalGridUpdatePayload }
  | { type: "reset" };

function reduce(state: InternalState, action: GridAction): InternalState {
  switch (action.type) {
    case "reset":
      return createInitialState();
    case "full": {
      const { payload } = action;
      const rows = emptyGrid(payload.cols, payload.rows);
      // Apply every dirty row from the full snapshot. We start from a
      // blank grid so any rows the backend chose to omit (e.g. all
      // blank rows in a quiet vim screen) end up rendered as spaces.
      for (const row of payload.dirty_rows) {
        if (row.y >= rows.length) continue;
        rows[row.y] = padRow(row.cells, payload.cols);
      }
      return {
        snapshot: {
          rows,
          cursor: payload.cursor,
          cols: payload.cols,
          rowCount: payload.rows,
          revision: state.snapshot.revision + 1,
          serverRev: payload.rev,
          appCursorMode: payload.app_cursor_mode,
        },
        lastRev: payload.rev,
      };
    }
    case "diff": {
      const { payload } = action;
      // Allocate fresh row references so React diffs detect the change.
      // We *do* keep cells inside unchanged rows pointer-equal so list
      // virtualisation can skip them.
      const rows = state.snapshot.rows.slice();
      // Grid may have been resized server-side without our knowing if
      // we missed an event — accept any new row count.
      if (rows.length < payload.rows) {
        while (rows.length < payload.rows) {
          rows.push(Array.from({ length: payload.cols }, () => createBlankCell()));
        }
      } else if (rows.length > payload.rows) {
        rows.length = payload.rows;
      }
      for (const row of payload.dirty_rows) {
        if (row.y >= rows.length) continue;
        rows[row.y] = padRow(row.cells, payload.cols);
      }
      return {
        snapshot: {
          rows,
          cursor: payload.cursor,
          cols: payload.cols,
          rowCount: payload.rows,
          revision: state.snapshot.revision + 1,
          serverRev: payload.rev,
          appCursorMode: payload.app_cursor_mode,
        },
        lastRev: payload.rev,
      };
    }
  }
}

/** Pad / truncate a row to exactly `cols` cells so the rendering layer
 *  can index without bounds checks. Cheaper than enforcing the invariant
 *  at the backend, where a corrupt frame would crash the emitter. */
function padRow(cells: GridCellPayload[], cols: number): GridCellPayload[] {
  if (cells.length === cols) return cells;
  if (cells.length > cols) return cells.slice(0, cols);
  const padded = cells.slice();
  while (padded.length < cols) padded.push(createBlankCell());
  return padded;
}

/**
 * Hook entry point. Returns the latest [`GridSnapshot`] for the given
 * session; updates on every successful merge of a
 * `terminal_grid_update` event.
 *
 * Pass `enabled = false` to short-circuit the subscription when the
 * session isn't on alt-screen yet — that's the common case and we
 * don't want to pay for the listener until we actually need to render.
 */
export function useGridState(sessionId: string, enabled: boolean): GridSnapshot {
  const [state, dispatch] = useReducer(reduce, undefined, createInitialState);
  // Latest rev we've seen; used inside async callbacks to avoid the
  // closure-captured stale value problem.
  const lastRevRef = useRef(0);
  lastRevRef.current = state.lastRev;

  useEffect(() => {
    if (!enabled || !sessionId) {
      dispatch({ type: "reset" });
      lastRevRef.current = 0;
      return;
    }

    let cancelled = false;
    let firstFrameLogged = false;

    const handle = async (payload: TerminalGridUpdatePayload) => {
      if (cancelled) return;
      if (payload.session_id !== sessionId) return;
      if (!firstFrameLogged) {
        firstFrameLogged = true;
        // eslint-disable-next-line no-console
        console.info(
          "[grid-debug][grid-update] first frame received",
          JSON.stringify({
            sessionId,
            full: payload.full,
            rev: payload.rev,
            cols: payload.cols,
            rows: payload.rows,
            dirtyRowCount: payload.dirty_rows.length,
            appCursorMode: payload.app_cursor_mode,
          })
        );
      }

      // #region agent log
      fetch("http://127.0.0.1:7440/ingest/f9f2cacd-c1f1-479f-8225-b4a5be2ee53c", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-Debug-Session-Id": "900b3a" },
        body: JSON.stringify({
          sessionId: "900b3a",
          location: "useGridState.ts:handle",
          message: "grid-update-received",
          data: {
            sessionId,
            full: payload.full,
            rev: payload.rev,
            cols: payload.cols,
            rows: payload.rows,
            dirtyRowCount: payload.dirty_rows.length,
            lastRevBefore: lastRevRef.current,
          },
          timestamp: Date.now(),
          hypothesisId: "3",
        }),
      }).catch(() => {});
      // #endregion

      // Detect a missed event: if we have a baseline and the new rev
      // isn't strictly greater than the last seen + 1 we may have
      // dropped frames. Ask for a fresh full snapshot to resync —
      // safe because the backend always returns a `full = true` frame.
      if (!payload.full && lastRevRef.current > 0 && payload.rev !== lastRevRef.current + 1) {
        logger.debug(
          `[GridTerminal] rev jump (${lastRevRef.current} → ${payload.rev}); requesting baseline`
        );
        try {
          const baseline = await ptyRequestGridSnapshot(sessionId);
          if (cancelled) return;
          if (baseline) {
            lastRevRef.current = baseline.rev;
            dispatch({ type: "full", payload: baseline });
          }
        } catch (err) {
          logger.warn("[GridTerminal] grid snapshot request failed:", err);
        }
        return;
      }

      lastRevRef.current = payload.rev;
      dispatch({
        type: payload.full ? "full" : "diff",
        payload,
      });
    };

    const unlistenPromise = onEvent("terminal_grid_update", handle);

    // Kick off an immediate baseline so the first paint isn't an empty
    // grid (the backend also emits one on alt-screen rising edge, but
    // we may miss it if React mounts after that has already happened —
    // e.g. user switches tabs back to a session that's mid-vim).
    void (async () => {
      try {
        const baseline = await ptyRequestGridSnapshot(sessionId);
        if (cancelled) return;

        // #region agent log
        const applied = !!baseline && baseline.rev > lastRevRef.current;
        const baselineDebug = {
          sessionId,
          hasBaseline: !!baseline,
          baselineRev: baseline?.rev ?? null,
          baselineCols: baseline?.cols ?? null,
          baselineRows: baseline?.rows ?? null,
          baselineDirtyRows: baseline?.dirty_rows.length ?? null,
          lastRevAtFetch: lastRevRef.current,
          appliedFull: applied,
          reasonSkipped: !baseline
            ? "no-baseline"
            : applied
              ? "applied"
              : `baseline.rev=${baseline.rev} <= lastRev=${lastRevRef.current}`,
        };
        // eslint-disable-next-line no-console
        console.info("[grid-debug][baseline-fetch]", JSON.stringify(baselineDebug));
        fetch("http://127.0.0.1:7440/ingest/f9f2cacd-c1f1-479f-8225-b4a5be2ee53c", {
          method: "POST",
          headers: { "Content-Type": "application/json", "X-Debug-Session-Id": "900b3a" },
          body: JSON.stringify({
            sessionId: "900b3a",
            location: "useGridState.ts:initial-baseline",
            message: "baseline-fetch-result",
            data: baselineDebug,
            timestamp: Date.now(),
            hypothesisId: "3",
          }),
        }).catch(() => {});
        // #endregion

        if (!baseline) return;
        // Only apply if no later frame has clobbered us already.
        if (baseline.rev > lastRevRef.current) {
          lastRevRef.current = baseline.rev;
          dispatch({ type: "full", payload: baseline });
        }
      } catch (err) {
        logger.debug("[GridTerminal] initial snapshot fetch failed:", err);
      }
    })();

    return () => {
      cancelled = true;
      unlistenPromise.then((u) => u()).catch(() => {});
    };
  }, [sessionId, enabled]);

  return state.snapshot;
}
