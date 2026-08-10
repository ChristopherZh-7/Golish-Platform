import * as jestDomMatchers from "@testing-library/jest-dom/matchers";
import type {} from "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { enableMapSet } from "immer";
import { afterEach, expect, vi } from "vitest";

// Extend the exact Vitest expect instance used by this workspace. Importing
// the jest-dom Vitest side-effect can resolve a second Vitest instance when a
// git worktree has an independent pnpm virtual store.
expect.extend(jestDomMatchers);

// Enable Immer MapSet plugin for Set/Map support in store
enableMapSet();

// Mock terminal managers for tests
vi.mock("@/lib/terminal", () => ({
  liveTerminalManager: {
    create: vi.fn(),
    getOrCreate: vi.fn(),
    attachToContainer: vi.fn(),
    detach: vi.fn(),
    write: vi.fn(),
    dispose: vi.fn(),
    scrollToBottom: vi.fn(),
    serializeAndDispose: vi.fn().mockResolvedValue(""),
    enableInput: vi.fn(),
    disableInput: vi.fn(),
    fit: vi.fn(),
    focus: vi.fn(),
  },
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
