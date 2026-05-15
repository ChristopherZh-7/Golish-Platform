import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { enableMapSet } from "immer";
import { afterEach, vi } from "vitest";

// Enable Immer MapSet plugin for Set/Map support in store
enableMapSet();

// Mock terminal managers for tests. The legacy `liveTerminalManager`
// was retired in Phase A of the Warp-style interaction refactor (see
// `docs/design/2026-05-15-warp-style-interaction.md`); we keep only
// the virtual terminal mock that survives.
vi.mock("@/lib/terminal", () => ({
  virtualTerminalManager: {
    create: vi.fn(),
    write: vi.fn(),
    dispose: vi.fn(),
  },
}));

// Cleanup after each test
afterEach(() => {
  cleanup();
});

// Mock crypto.randomUUID for consistent test IDs
vi.stubGlobal("crypto", {
  randomUUID: vi.fn(() => `test-uuid-${Math.random().toString(36).slice(2, 9)}`),
});

// Mock scrollIntoView which is not implemented in jsdom
Element.prototype.scrollIntoView = vi.fn();

// Mock matchMedia which is not implemented in jsdom
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: vi.fn().mockImplementation((query) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(), // deprecated
    removeListener: vi.fn(), // deprecated
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
});

// Mock ResizeObserver which is not implemented in jsdom
class MockResizeObserver {
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();
}
vi.stubGlobal("ResizeObserver", MockResizeObserver);
