import { expect, type Page, test } from "@playwright/test";
import { waitForAppReady } from "./helpers/app";

/**
 * Phase B · GridTerminal e2e
 *
 * Verifies the Rust-grid → React-DOM rendering path that replaces
 * xterm.js for alt-screen TUI sessions. See
 * `docs/design/2026-05-15-grid-terminal-phase-b.md`.
 *
 * Browser-mock mode here only exercises the frontend wiring (the
 * `__MOCK_INVOKE__` shim resolves Tauri commands synchronously and the
 * backend's `terminal_grid_update` events are simulated). The point of
 * this spec is to assert that the frontend half of the contract
 * doesn't regress; cross-platform interactive QA (real vim / htop) is
 * tracked separately in the handoff doc.
 */

interface MockGridCellPayload {
  ch: string;
  fg: { kind: "default" } | { kind: "indexed"; value: number } | { kind: "rgb"; value: number };
  bg: { kind: "default" } | { kind: "indexed"; value: number } | { kind: "rgb"; value: number };
  attrs: number;
}

interface MockGridUpdatePayload {
  session_id: string;
  rev: number;
  cols: number;
  rows: number;
  full: boolean;
  dirty_rows: Array<{ y: number; cells: MockGridCellPayload[] }>;
  cursor: { x: number; y: number; visible: boolean; style: "block" | "underline" | "bar" };
  alt_screen: boolean;
  app_cursor_mode: boolean;
}

/** Emit a `terminal_grid_update` event through the mock listener bus. */
async function emitGridUpdate(page: Page, payload: MockGridUpdatePayload) {
  await page.evaluate((p) => {
    type MockEntry = {
      handlerId: number;
      callback: (event: { event: string; payload: unknown }) => void;
    };
    const win = window as unknown as {
      __MOCK_EVENT_LISTENERS__?: Map<string, MockEntry[]>;
    };
    const listeners = win.__MOCK_EVENT_LISTENERS__?.get("terminal_grid_update");
    if (!listeners) return;
    for (const entry of listeners) {
      entry.callback({ event: "terminal_grid_update", payload: p });
    }
  }, payload);
}

/** Force the active session into fullterm render mode (skips the
 *  alt-screen → render-mode flip the backend would do for real). */
async function setActiveSessionFullterm(page: Page) {
  await page.evaluate(() => {
    type Store = {
      getState: () => {
        activeSessionId: string | null;
        setRenderMode: (id: string, mode: "fullterm" | "timeline") => void;
      };
    };
    const store = (window as unknown as { __GOLISH_STORE__?: Store }).__GOLISH_STORE__;
    if (!store) return;
    const state = store.getState();
    if (state.activeSessionId) {
      state.setRenderMode(state.activeSessionId, "fullterm");
    }
  });
}

/** Flip the `use_grid_renderer` settings flag at runtime via the
 *  `settings-updated` event the app listens for. */
async function enableGridRenderer(page: Page) {
  await page.evaluate(() => {
    window.dispatchEvent(
      new CustomEvent("settings-updated", {
        detail: {
          terminal: { use_grid_renderer: true },
        },
      })
    );
  });
}

async function getActiveSessionId(page: Page): Promise<string | null> {
  return await page.evaluate(() => {
    type Store = { getState: () => { activeSessionId: string | null } };
    return (
      (window as unknown as { __GOLISH_STORE__?: Store }).__GOLISH_STORE__?.getState()
        .activeSessionId ?? null
    );
  });
}

function defaultCell(ch: string): MockGridCellPayload {
  return {
    ch,
    fg: { kind: "default" },
    bg: { kind: "default" },
    attrs: 0,
  };
}

test.describe("Phase B · GridTerminal", () => {
  test("renders the empty-state placeholder before any frame arrives", async ({ page }) => {
    await waitForAppReady(page);
    await enableGridRenderer(page);
    await setActiveSessionFullterm(page);

    const placeholder = page.locator('[data-testid="grid-terminal-empty"]');
    await expect(placeholder).toBeVisible({ timeout: 5000 });
    await expect(placeholder).toContainText("Waiting for first frame");
  });

  test("renders a full snapshot frame into the grid", async ({ page }) => {
    await waitForAppReady(page);
    await enableGridRenderer(page);
    await setActiveSessionFullterm(page);

    const sessionId = await getActiveSessionId(page);
    expect(sessionId).not.toBeNull();
    if (!sessionId) return;

    await emitGridUpdate(page, {
      session_id: sessionId,
      rev: 1,
      cols: 5,
      rows: 2,
      full: true,
      dirty_rows: [
        { y: 0, cells: "hello".split("").map(defaultCell) },
        { y: 1, cells: "world".split("").map(defaultCell) },
      ],
      cursor: { x: 0, y: 0, visible: true, style: "block" },
      alt_screen: true,
      app_cursor_mode: false,
    });

    const grid = page.locator('[data-testid="grid-terminal"]');
    await expect(grid).toBeVisible({ timeout: 5000 });
    await expect(grid).toHaveAttribute("data-rev", "1");
    await expect(grid).toHaveAttribute("data-cols", "5");
    await expect(grid).toHaveAttribute("data-rows", "2");

    // Each row renders as a `.gt-row` containing per-cell `<span>`s;
    // we assert the visible glyphs landed in the right place rather
    // than counting spans (the wide-char spacer skip path emits null).
    const rows = grid.locator(".gt-row");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toContainText("hello");
    await expect(rows.nth(1)).toContainText("world");
  });

  test("diff frames overwrite only the dirty rows", async ({ page }) => {
    await waitForAppReady(page);
    await enableGridRenderer(page);
    await setActiveSessionFullterm(page);

    const sessionId = await getActiveSessionId(page);
    if (!sessionId) return;

    await emitGridUpdate(page, {
      session_id: sessionId,
      rev: 1,
      cols: 3,
      rows: 2,
      full: true,
      dirty_rows: [
        { y: 0, cells: "abc".split("").map(defaultCell) },
        { y: 1, cells: "def".split("").map(defaultCell) },
      ],
      cursor: { x: 0, y: 0, visible: true, style: "block" },
      alt_screen: true,
      app_cursor_mode: false,
    });

    await expect(page.locator('[data-testid="grid-terminal"]')).toHaveAttribute("data-rev", "1");

    // Now ship a diff that only touches row 1.
    await emitGridUpdate(page, {
      session_id: sessionId,
      rev: 2,
      cols: 3,
      rows: 2,
      full: false,
      dirty_rows: [{ y: 1, cells: "xyz".split("").map(defaultCell) }],
      cursor: { x: 0, y: 1, visible: true, style: "block" },
      alt_screen: true,
      app_cursor_mode: false,
    });

    const rows = page.locator('[data-testid="grid-terminal"] .gt-row');
    await expect(page.locator('[data-testid="grid-terminal"]')).toHaveAttribute("data-rev", "2");
    await expect(rows.nth(0)).toContainText("abc");
    await expect(rows.nth(1)).toContainText("xyz");
  });

  test("exposes app_cursor_mode via data attribute", async ({ page }) => {
    await waitForAppReady(page);
    await enableGridRenderer(page);
    await setActiveSessionFullterm(page);

    const sessionId = await getActiveSessionId(page);
    if (!sessionId) return;

    await emitGridUpdate(page, {
      session_id: sessionId,
      rev: 1,
      cols: 1,
      rows: 1,
      full: true,
      dirty_rows: [{ y: 0, cells: [defaultCell(" ")] }],
      cursor: { x: 0, y: 0, visible: true, style: "block" },
      alt_screen: true,
      app_cursor_mode: true,
    });

    await expect(page.locator('[data-testid="grid-terminal"]')).toHaveAttribute(
      "data-app-cursor",
      "true"
    );
  });

  test("ignores updates targeted at other sessions", async ({ page }) => {
    await waitForAppReady(page);
    await enableGridRenderer(page);
    await setActiveSessionFullterm(page);

    const sessionId = await getActiveSessionId(page);
    if (!sessionId) return;

    await emitGridUpdate(page, {
      session_id: `${sessionId}-someone-else`,
      rev: 1,
      cols: 3,
      rows: 1,
      full: true,
      dirty_rows: [{ y: 0, cells: "xxx".split("").map(defaultCell) }],
      cursor: { x: 0, y: 0, visible: true, style: "block" },
      alt_screen: true,
      app_cursor_mode: false,
    });

    // The placeholder should still be visible — no frame for our
    // session means no transition out of empty state.
    await expect(page.locator('[data-testid="grid-terminal-empty"]')).toBeVisible();
    await expect(page.locator('[data-testid="grid-terminal"]')).toHaveCount(0);
  });
});
