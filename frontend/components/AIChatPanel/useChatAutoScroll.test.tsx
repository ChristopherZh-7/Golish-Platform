import { act, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useChatAutoScroll } from "./useChatAutoScroll";

/**
 * Capturing ResizeObserver: jsdom ships no real implementation and the global
 * test setup stubs a no-op one. This variant records instances and exposes a
 * `fire()` so we can drive the content-growth code path deterministically.
 */
class CapturingResizeObserver {
  static instances: CapturingResizeObserver[] = [];
  callback: ResizeObserverCallback;
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();
  constructor(cb: ResizeObserverCallback) {
    this.callback = cb;
    CapturingResizeObserver.instances.push(this);
  }
  fire() {
    this.callback([], this as unknown as ResizeObserver);
  }
}

function lastObserver(): CapturingResizeObserver | undefined {
  const list = CapturingResizeObserver.instances;
  return list[list.length - 1];
}

let hookState: ReturnType<typeof useChatAutoScroll> | null = null;

function Harness({
  messages,
  active = true,
  scrollKey = "conversation-1",
}: {
  messages: readonly unknown[];
  active?: boolean;
  scrollKey?: string;
}) {
  hookState = useChatAutoScroll(messages, { active, scrollKey });
  if (!active) return null;
  return (
    <div data-testid="container" ref={hookState.messagesContainerRef}>
      <div data-testid="content">body</div>
    </div>
  );
}

/** jsdom does no layout, so fake the scroll geometry the hook reads/writes. */
function patchScroll(el: HTMLElement, scrollHeight: number) {
  let top = 0;
  Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => scrollHeight });
  Object.defineProperty(el, "clientHeight", { configurable: true, get: () => 100 });
  Object.defineProperty(el, "scrollTop", {
    configurable: true,
    get: () => top,
    set: (v: number) => {
      top = v;
    },
  });
  return () => top;
}

describe("useChatAutoScroll", () => {
  let originalRO: typeof ResizeObserver;

  beforeEach(() => {
    originalRO = globalThis.ResizeObserver;
    CapturingResizeObserver.instances = [];
    globalThis.ResizeObserver = CapturingResizeObserver as unknown as typeof ResizeObserver;
  });

  afterEach(() => {
    globalThis.ResizeObserver = originalRO;
    hookState = null;
  });

  it("observes the content wrapper so non-message height growth can auto-scroll", () => {
    const { getByTestId } = render(<Harness messages={[]} />);
    const observer = lastObserver();
    expect(observer).toBeDefined();
    expect(observer?.observe).toHaveBeenCalledWith(getByTestId("content"));
  });

  it("re-points the observer at the live content wrapper when messages change", () => {
    const { getByTestId, rerender } = render(<Harness messages={[{ id: 1 }]} />);
    const observer = lastObserver();
    observer?.observe.mockClear();
    observer?.disconnect.mockClear();
    rerender(<Harness messages={[{ id: 1 }, { id: 2 }]} />);
    expect(observer?.disconnect).toHaveBeenCalled();
    expect(observer?.observe).toHaveBeenCalledWith(getByTestId("content"));
  });

  it("scrolls to the bottom when the observer fires and the user is at bottom", async () => {
    const { getByTestId } = render(<Harness messages={[{ id: 1 }]} />);
    const readTop = patchScroll(getByTestId("container"), 800);
    act(() => lastObserver()?.fire());
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });
    expect(readTop()).toBe(800);
  });

  it("does not yank the view when the user has scrolled up", () => {
    const { getByTestId } = render(<Harness messages={[{ id: 1 }]} />);
    const readTop = patchScroll(getByTestId("container"), 800);
    hookState!.userScrolledUpRef.current = true;
    act(() => lastObserver()?.fire());
    expect(readTop()).toBe(0);
  });

  it("reattaches to a rebuilt viewport and follows messages added while detail owns the UI", async () => {
    const messages = [{ id: 1 }];
    const view = render(<Harness messages={messages} />);
    const oldViewport = view.getByTestId("container");
    const readOldTop = patchScroll(oldViewport, 800);
    act(() => lastObserver()?.fire());
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });
    expect(readOldTop()).toBe(800);

    const observer = lastObserver();
    observer?.observe.mockClear();
    const hiddenMessages = [...messages, { id: 2 }];
    view.rerender(<Harness active={false} messages={hiddenMessages} />);
    view.rerender(<Harness messages={hiddenMessages} />);

    const rebuiltViewport = view.getByTestId("container");
    expect(rebuiltViewport).not.toBe(oldViewport);
    const readRebuiltTop = patchScroll(rebuiltViewport, 1_100);
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });

    expect(readRebuiltTop()).toBe(1_100);
    expect(observer?.observe).toHaveBeenCalledWith(view.getByTestId("content"));

    rebuiltViewport.scrollTop = 500;
    fireEvent.wheel(rebuiltViewport, { deltaY: -120 });
    act(() => observer?.fire());
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });
    expect(readRebuiltTop()).toBe(500);
  });

  it("restores an intentional history-reading position after the viewport is rebuilt", async () => {
    const messages = [{ id: 1 }];
    const view = render(<Harness messages={messages} />);
    const oldViewport = view.getByTestId("container");
    patchScroll(oldViewport, 800);
    oldViewport.scrollTop = 240;
    hookState!.userScrolledUpRef.current = true;

    view.rerender(<Harness active={false} messages={messages} />);
    view.rerender(<Harness messages={messages} />);

    const rebuiltViewport = view.getByTestId("container");
    const readRebuiltTop = patchScroll(rebuiltViewport, 900);
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });

    expect(readRebuiltTop()).toBe(240);
  });
});
