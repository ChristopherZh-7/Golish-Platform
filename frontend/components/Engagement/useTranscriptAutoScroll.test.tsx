import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useTranscriptAutoScroll } from "./useTranscriptAutoScroll";

class CapturingResizeObserver {
  static instances: CapturingResizeObserver[] = [];
  callback: ResizeObserverCallback;
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
    CapturingResizeObserver.instances.push(this);
  }

  fire() {
    this.callback([], this as unknown as ResizeObserver);
  }
}

function lastObserver(): CapturingResizeObserver | undefined {
  const instances = CapturingResizeObserver.instances;
  return instances[instances.length - 1];
}

function Harness({
  items,
  toolOutput = "",
  supplementary = false,
}: {
  items: readonly string[];
  toolOutput?: string;
  supplementary?: boolean;
}) {
  const { viewportRef, contentRef, onViewportScroll, onViewportWheel } =
    useTranscriptAutoScroll();
  return (
    <div
      ref={viewportRef}
      data-testid="viewport"
      onScroll={onViewportScroll}
      onWheel={onViewportWheel}
    >
      <div ref={contentRef} data-testid="content">
        {items.map((item) => (
          <div key={item}>{item}</div>
        ))}
        <pre>{toolOutput}</pre>
      </div>
      {supplementary && <div data-testid="supplementary">Evidence</div>}
    </div>
  );
}

function setMetrics(
  viewport: HTMLElement,
  content: HTMLElement,
  readContentHeight: () => number,
  options: {
    unrelatedOffsetTop?: number;
    dispatchProgrammaticScroll?: boolean;
    readViewportHeight?: () => number;
  } = {}
) {
  let top = 0;
  const readViewportHeight = options.readViewportHeight ?? readContentHeight;
  Object.defineProperties(viewport, {
    clientHeight: { configurable: true, value: 100 },
    scrollHeight: { configurable: true, get: readViewportHeight },
    scrollTop: {
      configurable: true,
      get: () => top,
      set: (value: number) => {
        top = Math.max(0, Math.min(value, readViewportHeight() - 100));
        if (options.dispatchProgrammaticScroll) {
          viewport.dispatchEvent(new Event("scroll", { bubbles: true }));
        }
      },
    },
  });
  Object.defineProperties(content, {
    offsetTop: { configurable: true, value: options.unrelatedOffsetTop ?? 0 },
    scrollHeight: { configurable: true, get: readContentHeight },
  });
  viewport.getBoundingClientRect = () =>
    ({ top: 100, bottom: 200, height: 100, left: 0, right: 100, width: 100, x: 0, y: 100, toJSON() {} }) as DOMRect;
  content.getBoundingClientRect = () =>
    ({
      top: 100 - top,
      bottom: 100 - top + readContentHeight(),
      height: readContentHeight(),
      left: 0,
      right: 100,
      width: 100,
      x: 0,
      y: 100 - top,
      toJSON() {},
    }) as DOMRect;
  return {
    readTop: () => top,
    setTop: (value: number) => {
      top = value;
    },
  };
}

describe("useTranscriptAutoScroll", () => {
  let originalResizeObserver: typeof ResizeObserver;

  beforeEach(() => {
    originalResizeObserver = globalThis.ResizeObserver;
    CapturingResizeObserver.instances = [];
    globalThis.ResizeObserver = CapturingResizeObserver as unknown as typeof ResizeObserver;
  });

  afterEach(() => {
    globalThis.ResizeObserver = originalResizeObserver;
  });

  it("follows new transcript content, pauses on manual history reading, and resumes at bottom", async () => {
    const view = render(<Harness items={["one"]} />);
    const viewport = screen.getByTestId("viewport");
    const content = screen.getByTestId("content");
    let contentHeight = 300;
    const geometry = setMetrics(viewport, content, () => contentHeight);

    view.rerender(<Harness items={["one", "two"]} />);
    await waitFor(() => expect(viewport.scrollTop).toBe(200));

    geometry.setTop(60);
    fireEvent.scroll(viewport);
    contentHeight = 400;
    view.rerender(<Harness items={["one", "two", "three"]} />);
    await new Promise((resolve) => requestAnimationFrame(resolve));
    expect(viewport.scrollTop).toBe(60);

    geometry.setTop(300);
    fireEvent.scroll(viewport);
    contentHeight = 500;
    view.rerender(<Harness items={["one", "two", "three", "four"]} />);
    await waitFor(() => expect(viewport.scrollTop).toBe(400));
  });

  it("pauses follow mode as soon as the user wheels upward", async () => {
    const view = render(<Harness items={["one"]} />);
    const viewport = screen.getByTestId("viewport");
    const content = screen.getByTestId("content");
    let contentHeight = 300;
    setMetrics(viewport, content, () => contentHeight);
    view.rerender(<Harness items={["one", "two"]} />);
    await waitFor(() => expect(viewport.scrollTop).toBe(200));

    fireEvent.wheel(viewport, { deltaY: -120 });
    contentHeight = 400;
    view.rerender(<Harness items={["one", "two", "three"]} />);
    await new Promise((resolve) => requestAnimationFrame(resolve));
    expect(viewport.scrollTop).toBe(200);
  });

  it("keeps following repeated tool-output growth after clamped programmatic scroll events", async () => {
    const stableItems = ["tool-call"];
    const view = render(<Harness items={stableItems} toolOutput="start" />);
    const viewport = screen.getByTestId("viewport");
    const content = screen.getByTestId("content");
    let contentHeight = 300;
    const geometry = setMetrics(viewport, content, () => contentHeight, {
      unrelatedOffsetTop: 900,
      dispatchProgrammaticScroll: true,
    });

    view.rerender(<Harness items={stableItems} toolOutput="first output chunk" />);
    await waitFor(() => expect(geometry.readTop()).toBe(200));

    contentHeight = 420;
    act(() => lastObserver()?.fire());
    await waitFor(() => expect(geometry.readTop()).toBe(320));

    contentHeight = 560;
    act(() => lastObserver()?.fire());
    await waitFor(() => expect(geometry.readTop()).toBe(460));
  });

  it("follows a tool-output rerender even when the transcript entry array is unchanged", async () => {
    const stableItems = ["tool-call"];
    const view = render(<Harness items={stableItems} toolOutput="start" />);
    const viewport = screen.getByTestId("viewport");
    const content = screen.getByTestId("content");
    let contentHeight = 300;
    const geometry = setMetrics(viewport, content, () => contentHeight);

    view.rerender(<Harness items={stableItems} toolOutput="first output chunk" />);
    await waitFor(() => expect(geometry.readTop()).toBe(200));

    contentHeight = 480;
    view.rerender(<Harness items={stableItems} toolOutput="a much larger second output chunk" />);
    await waitFor(() => expect(geometry.readTop()).toBe(380));
  });

  it("resumes at the transcript bottom without requiring a scroll through later Evidence", async () => {
    const view = render(<Harness items={["one"]} supplementary />);
    const viewport = screen.getByTestId("viewport");
    const content = screen.getByTestId("content");
    let contentHeight = 300;
    let viewportHeight = 500;
    const geometry = setMetrics(viewport, content, () => contentHeight, {
      readViewportHeight: () => viewportHeight,
    });

    view.rerender(<Harness items={["one", "two"]} supplementary />);
    await waitFor(() => expect(geometry.readTop()).toBe(200));

    fireEvent.wheel(viewport, { deltaY: -120 });
    geometry.setTop(50);
    fireEvent.scroll(viewport);
    geometry.setTop(200);
    fireEvent.scroll(viewport);

    contentHeight = 400;
    viewportHeight = 600;
    view.rerender(<Harness items={["one", "two", "three"]} supplementary />);
    await waitFor(() => expect(geometry.readTop()).toBe(300));
  });

  it("does not pull the viewport out of supplementary Evidence being read below the transcript", async () => {
    const view = render(<Harness items={["one"]} supplementary />);
    const viewport = screen.getByTestId("viewport");
    const content = screen.getByTestId("content");
    let contentHeight = 300;
    let viewportHeight = 500;
    const geometry = setMetrics(viewport, content, () => contentHeight, {
      readViewportHeight: () => viewportHeight,
    });

    view.rerender(<Harness items={["one", "two"]} supplementary />);
    await waitFor(() => expect(geometry.readTop()).toBe(200));

    geometry.setTop(350);
    fireEvent.scroll(viewport);
    contentHeight = 400;
    viewportHeight = 600;
    view.rerender(<Harness items={["one", "two", "three"]} supplementary />);
    await new Promise((resolve) => requestAnimationFrame(resolve));
    expect(geometry.readTop()).toBe(350);

    geometry.setTop(300);
    fireEvent.scroll(viewport);
    contentHeight = 500;
    viewportHeight = 700;
    view.rerender(<Harness items={["one", "two", "three", "four"]} supplementary />);
    await waitFor(() => expect(geometry.readTop()).toBe(400));
  });
});
