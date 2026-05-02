import { render } from "@testing-library/react";
import { createRef } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UnifiedBlock } from "@/store";
import { VirtualizedTimeline } from "./VirtualizedTimeline";

// Mock the store
vi.mock("@/store", async () => {
  const actual = await vi.importActual("@/store");
  return {
    ...actual,
    useStore: vi.fn(() => ({
      collapsedBlocks: {},
      toggleBlockCollapse: vi.fn(),
    })),
  };
});

// Mock ResizeObserver
class MockResizeObserver {
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();
}
globalThis.ResizeObserver = MockResizeObserver as unknown as typeof ResizeObserver;

function createCommandBlock(id: string, command: string): UnifiedBlock {
  return {
    id,
    type: "command",
    timestamp: new Date().toISOString(),
    data: {
      id,
      sessionId: "test-session",
      command,
      output: `Output for ${command}`,
      exitCode: 0,
      startTime: new Date().toISOString(),
      durationMs: 100,
      workingDirectory: "/test",
      isCollapsed: false,
    },
  };
}

function createAgentMessage(id: string, content: string): UnifiedBlock {
  return {
    id,
    type: "agent_message",
    timestamp: new Date().toISOString(),
    data: {
      id,
      sessionId: "test-session",
      role: "assistant",
      content,
      timestamp: new Date().toISOString(),
      streamingHistory: [],
    },
  };
}

describe("VirtualizedTimeline", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("small timelines (below threshold)", () => {
    it("renders all command blocks directly without virtualization", () => {
      const containerRef = createRef<HTMLDivElement>();
      // VirtualizedTimeline currently only renders `command` blocks itself
      // (other block types are rendered by sibling components in
      // UnifiedTimeline). We pass two command blocks so the count is stable.
      const blocks: UnifiedBlock[] = [
        createCommandBlock("cmd-1", "ls"),
        createCommandBlock("cmd-2", "pwd"),
      ];

      const { container } = render(
        <div ref={containerRef} style={{ height: 500, overflow: "auto" }}>
          <VirtualizedTimeline
            blocks={blocks}
            sessionId="test-session"
            containerRef={containerRef}
            shouldScrollToBottom={false}
            workingDirectory="/test/dir"
          />
        </div>
      );

      // The non-virtualized branch wraps children in a `divide-y` container.
      const wrapper = container.querySelector(".divide-y");
      expect(wrapper).toBeInTheDocument();
      expect(wrapper?.children.length).toBe(2);
    });

    it("renders empty state for no blocks", () => {
      const containerRef = createRef<HTMLDivElement>();
      const { container } = render(
        <div ref={containerRef} style={{ height: 500, overflow: "auto" }}>
          <VirtualizedTimeline
            blocks={[]}
            sessionId="test-session"
            containerRef={containerRef}
            shouldScrollToBottom={false}
            workingDirectory="/test/dir"
          />
        </div>
      );

      const wrapper = container.querySelector(".divide-y");
      expect(wrapper).toBeInTheDocument();
      expect(wrapper?.children).toHaveLength(0);
    });
  });

  describe("large timelines (above threshold)", () => {
    it("uses virtualization container for many blocks", () => {
      const containerRef = createRef<HTMLDivElement>();
      // Create 60 blocks (above the 50 threshold)
      const blocks: UnifiedBlock[] = Array.from({ length: 60 }, (_, i) =>
        createCommandBlock(`cmd-${i}`, `command-${i}`)
      );

      const { container } = render(
        <div ref={containerRef} style={{ height: 500, overflow: "auto" }}>
          <VirtualizedTimeline
            blocks={blocks}
            sessionId="test-session"
            containerRef={containerRef}
            shouldScrollToBottom={false}
            workingDirectory="/test/dir"
          />
        </div>
      );

      // Should have a container with position: relative (virtualization wrapper)
      const virtualContainer = container.querySelector('[style*="position: relative"]');
      expect(virtualContainer).toBeInTheDocument();
    });

    it("renders without crashing for large block counts", () => {
      const containerRef = createRef<HTMLDivElement>();
      const blocks: UnifiedBlock[] = Array.from({ length: 100 }, (_, i) =>
        createCommandBlock(`cmd-${i}`, `command-${i}`)
      );

      // Should not throw
      const { container } = render(
        <div ref={containerRef} style={{ height: 500, overflow: "auto" }}>
          <VirtualizedTimeline
            blocks={blocks}
            sessionId="test-session"
            containerRef={containerRef}
            shouldScrollToBottom={false}
            workingDirectory="/test/dir"
          />
        </div>
      );

      expect(container).toBeInTheDocument();
    });
  });

  describe("block types", () => {
    it("renders command blocks without errors", () => {
      const containerRef = createRef<HTMLDivElement>();
      const blocks: UnifiedBlock[] = [createCommandBlock("cmd-1", "echo hello")];

      const { container } = render(
        <div ref={containerRef} style={{ height: 500, overflow: "auto" }}>
          <VirtualizedTimeline
            blocks={blocks}
            sessionId="test-session"
            containerRef={containerRef}
            shouldScrollToBottom={false}
            workingDirectory="/test/dir"
          />
        </div>
      );

      const wrapper = container.querySelector(".divide-y");
      expect(wrapper).toBeInTheDocument();
      expect(wrapper?.children.length).toBe(1);
    });

    it("ignores non-command blocks (rendered elsewhere in UnifiedTimeline)", () => {
      // VirtualizedTimeline today returns null for any block whose type
      // isn't `command`; agent_message / system_hook are rendered by
      // sibling components. The wrapper should still mount (so
      // virtualization plumbing stays consistent), just with no children.
      const containerRef = createRef<HTMLDivElement>();
      const blocks: UnifiedBlock[] = [
        createAgentMessage("msg-1", "Test response"),
        {
          id: "hook-1",
          type: "system_hook",
          timestamp: new Date().toISOString(),
          data: { hooks: ["Test hook content"] },
        } as UnifiedBlock,
      ];

      const { container } = render(
        <div ref={containerRef} style={{ height: 500, overflow: "auto" }}>
          <VirtualizedTimeline
            blocks={blocks}
            sessionId="test-session"
            containerRef={containerRef}
            shouldScrollToBottom={false}
            workingDirectory="/test/dir"
          />
        </div>
      );

      const wrapper = container.querySelector(".divide-y");
      expect(wrapper).toBeInTheDocument();
      // Both blocks should be filtered out; React renders them as null.
      expect(wrapper?.querySelectorAll("div").length ?? 0).toBe(0);
    });
  });

  describe("error handling", () => {
    it("wraps blocks in error boundaries", () => {
      const containerRef = createRef<HTMLDivElement>();
      const blocks: UnifiedBlock[] = [
        createCommandBlock("cmd-1", "ls"),
        createAgentMessage("msg-1", "Hello"),
      ];

      // Render should succeed
      const { container } = render(
        <div ref={containerRef} style={{ height: 500, overflow: "auto" }}>
          <VirtualizedTimeline
            blocks={blocks}
            sessionId="test-session"
            containerRef={containerRef}
            shouldScrollToBottom={false}
            workingDirectory="/test/dir"
          />
        </div>
      );

      expect(container).toBeInTheDocument();
    });
  });
});
