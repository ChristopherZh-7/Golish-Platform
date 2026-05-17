import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TerminalGridUpdatePayload } from "@/lib/events/payloads";

// Stub the Tauri listener facade so we can hand-feed grid updates
// from the tests below. The real implementation goes through
// `@tauri-apps/api/event::listen`, which jsdom can't satisfy.
type Handler = (payload: TerminalGridUpdatePayload) => void | Promise<void>;
const listeners = new Set<Handler>();
const unlisteners = new Set<() => void>();

vi.mock("@/lib/events", () => ({
  onEvent: vi.fn((channel: string, handler: Handler) => {
    if (channel !== "terminal_grid_update") {
      return Promise.resolve(() => {});
    }
    listeners.add(handler);
    const unlisten = () => {
      listeners.delete(handler);
    };
    unlisteners.add(unlisten);
    return Promise.resolve(unlisten);
  }),
}));

const requestSnapshotMock = vi.fn<
  (sessionId: string) => Promise<TerminalGridUpdatePayload | null>
>();

vi.mock("@/lib/api/pty", () => ({
  ptyRequestGridSnapshot: (sessionId: string) => requestSnapshotMock(sessionId),
}));

// Import the hook after the mocks are wired so it picks up our stubs.
import { useGridState } from "./useGridState";

const SESSION = "sess-1";

function cell(ch: string) {
  return {
    ch,
    fg: { kind: "default" as const },
    bg: { kind: "default" as const },
    attrs: 0,
  };
}

function frame(
  rev: number,
  full: boolean,
  dirty: Array<{ y: number; cells: string[] }>,
  overrides?: Partial<TerminalGridUpdatePayload>
): TerminalGridUpdatePayload {
  return {
    session_id: SESSION,
    rev,
    cols: 3,
    rows: 2,
    full,
    dirty_rows: dirty.map((row) => ({
      y: row.y,
      cells: row.cells.map(cell),
    })),
    cursor: { x: 0, y: 0, visible: true, style: "block" },
    alt_screen: true,
    app_cursor_mode: false,
    ...overrides,
  };
}

async function emit(payload: TerminalGridUpdatePayload) {
  // Snapshot to a local array so handlers that synchronously unsubscribe
  // don't affect the iteration.
  const snapshot = Array.from(listeners);
  await Promise.all(snapshot.map((h) => h(payload)));
}

beforeEach(() => {
  listeners.clear();
  for (const u of unlisteners) u();
  unlisteners.clear();
  requestSnapshotMock.mockReset();
  requestSnapshotMock.mockResolvedValue(null);
});

describe("useGridState", () => {
  it("starts empty when disabled", () => {
    const { result } = renderHook(() => useGridState(SESSION, false));
    expect(result.current.rowCount).toBe(0);
    expect(result.current.rows).toEqual([]);
  });

  it("renders a baseline after a full frame arrives", async () => {
    const { result } = renderHook(() => useGridState(SESSION, true));

    await act(async () => {
      await emit(frame(1, true, [{ y: 0, cells: ["a", "b", "c"] }]));
    });

    await waitFor(() => expect(result.current.rowCount).toBe(2));
    expect(result.current.rows[0].map((c) => c.ch)).toEqual(["a", "b", "c"]);
    // Row 1 was omitted by the backend so it should be blank.
    expect(result.current.rows[1].map((c) => c.ch)).toEqual([" ", " ", " "]);
    expect(result.current.serverRev).toBe(1);
  });

  it("merges diff frames onto the cached grid", async () => {
    const { result } = renderHook(() => useGridState(SESSION, true));

    await act(async () => {
      await emit(frame(1, true, [{ y: 0, cells: ["x", "x", "x"] }]));
    });
    await waitFor(() => expect(result.current.serverRev).toBe(1));

    // The hook fires an initial baseline fetch in `useEffect` (returns
    // null in this test), so we only care that the subsequent
    // contiguous diff does NOT trigger another fetch.
    const callsBefore = requestSnapshotMock.mock.calls.length;

    await act(async () => {
      await emit(frame(2, false, [{ y: 1, cells: ["y", "y", "y"] }]));
    });

    await waitFor(() => expect(result.current.serverRev).toBe(2));
    expect(result.current.rows[0].map((c) => c.ch)).toEqual(["x", "x", "x"]);
    expect(result.current.rows[1].map((c) => c.ch)).toEqual(["y", "y", "y"]);
    expect(requestSnapshotMock.mock.calls.length).toBe(callsBefore);
  });

  it("requests a fresh baseline when a rev jump is detected", async () => {
    const { result } = renderHook(() => useGridState(SESSION, true));

    await act(async () => {
      await emit(frame(1, true, [{ y: 0, cells: ["a", "b", "c"] }]));
    });
    await waitFor(() => expect(result.current.serverRev).toBe(1));

    const recovery = frame(7, true, [{ y: 0, cells: ["R", "R", "R"] }]);
    requestSnapshotMock.mockResolvedValueOnce(recovery);

    // Skip from rev 1 to rev 5 — the diff is non-contiguous so the
    // hook should fetch a full snapshot to resync.
    await act(async () => {
      await emit(frame(5, false, [{ y: 1, cells: ["?", "?", "?"] }]));
    });

    await waitFor(() => expect(result.current.serverRev).toBe(7));
    expect(requestSnapshotMock).toHaveBeenCalledWith(SESSION);
    expect(result.current.rows[0].map((c) => c.ch)).toEqual(["R", "R", "R"]);
  });

  it("ignores updates for other sessions", async () => {
    const { result } = renderHook(() => useGridState(SESSION, true));

    await act(async () => {
      await emit({
        ...frame(1, true, [{ y: 0, cells: ["x", "x", "x"] }]),
        session_id: "other-session",
      });
    });

    // Should remain empty.
    expect(result.current.rowCount).toBe(0);
  });

  it("resets state when disabled mid-flight", async () => {
    const { result, rerender } = renderHook(
      ({ enabled }: { enabled: boolean }) => useGridState(SESSION, enabled),
      { initialProps: { enabled: true } }
    );

    await act(async () => {
      await emit(frame(1, true, [{ y: 0, cells: ["a", "b", "c"] }]));
    });
    await waitFor(() => expect(result.current.serverRev).toBe(1));

    rerender({ enabled: false });

    expect(result.current.rowCount).toBe(0);
    expect(result.current.serverRev).toBe(0);
  });
});
