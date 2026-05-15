import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

describe("installScrollbarAutoHide", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    document.documentElement.removeAttribute("data-scrolling");
  });

  afterEach(() => {
    vi.useRealTimers();
    document.documentElement.removeAttribute("data-scrolling");
  });

  it("flashes the `data-scrolling` attribute on scroll, then removes it", async () => {
    const { installScrollbarAutoHide } = await import("./scrollbar-autohide");
    installScrollbarAutoHide();

    expect(document.documentElement.hasAttribute("data-scrolling")).toBe(false);

    const scrollable = document.createElement("div");
    document.body.appendChild(scrollable);
    scrollable.dispatchEvent(new Event("scroll", { bubbles: false }));

    expect(document.documentElement.hasAttribute("data-scrolling")).toBe(true);

    vi.advanceTimersByTime(799);
    expect(document.documentElement.hasAttribute("data-scrolling")).toBe(true);

    vi.advanceTimersByTime(2);
    expect(document.documentElement.hasAttribute("data-scrolling")).toBe(false);

    document.body.removeChild(scrollable);
  });

  it("resets the hide-timer on each new scroll event (debounced flash)", async () => {
    const { installScrollbarAutoHide } = await import("./scrollbar-autohide");
    installScrollbarAutoHide();

    const target = document.createElement("div");
    document.body.appendChild(target);

    target.dispatchEvent(new Event("scroll", { bubbles: false }));
    vi.advanceTimersByTime(500);
    target.dispatchEvent(new Event("scroll", { bubbles: false }));
    vi.advanceTimersByTime(500);

    // First scroll was 1000ms ago — without debounce the attr would be gone.
    expect(document.documentElement.hasAttribute("data-scrolling")).toBe(true);

    vi.advanceTimersByTime(301);
    expect(document.documentElement.hasAttribute("data-scrolling")).toBe(false);

    document.body.removeChild(target);
  });

  it("is idempotent — multiple installs do not double-fire the timer", async () => {
    const { installScrollbarAutoHide } = await import("./scrollbar-autohide");
    installScrollbarAutoHide();
    installScrollbarAutoHide();
    installScrollbarAutoHide();

    const target = document.createElement("div");
    document.body.appendChild(target);
    target.dispatchEvent(new Event("scroll", { bubbles: false }));

    expect(document.documentElement.hasAttribute("data-scrolling")).toBe(true);
    vi.advanceTimersByTime(801);
    expect(document.documentElement.hasAttribute("data-scrolling")).toBe(false);

    document.body.removeChild(target);
  });

  it("also flashes on wheel events", async () => {
    const { installScrollbarAutoHide } = await import("./scrollbar-autohide");
    installScrollbarAutoHide();

    const target = document.createElement("div");
    document.body.appendChild(target);
    target.dispatchEvent(new Event("wheel", { bubbles: false }));

    expect(document.documentElement.hasAttribute("data-scrolling")).toBe(true);

    document.body.removeChild(target);
  });
});
