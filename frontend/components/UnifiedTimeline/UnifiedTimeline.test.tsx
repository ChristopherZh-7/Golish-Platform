import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { selectCommandBlocksFromTimeline } from "@/lib/timeline/selectors";
import { getOutputBuffer } from "@/store/slices/session-helpers";
import { useStore } from "../../store";
import { UnifiedTimeline } from "./UnifiedTimeline";

// Mock xterm.js and addons - they don't work in jsdom
vi.mock("@xterm/xterm", () => ({
  Terminal: class MockTerminal {
    options = { theme: {} };
    rows = 24;
    cols = 80;
    loadAddon = vi.fn();
    open = vi.fn();
    write = vi.fn();
    clear = vi.fn();
    dispose = vi.fn();
    scrollToBottom = vi.fn();
    resize = vi.fn();
    element = document.createElement("div");
    registerLinkProvider = vi.fn(() => ({ dispose: vi.fn() }));
    buffer = {
      active: {
        getLine: vi.fn(() => ({
          translateToString: vi.fn(() => ""),
        })),
      },
    };
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class MockFitAddon {
    fit = vi.fn();
  },
}));

vi.mock("@xterm/addon-serialize", () => ({
  SerializeAddon: class MockSerializeAddon {
    serialize = vi.fn(() => "");
  },
}));

describe("UnifiedTimeline", () => {
  beforeEach(() => {
    // Reset store state
    useStore.setState({
      sessions: {},
      activeSessionId: null,
      timelines: {},
      pendingCommand: {},
      agentStreaming: {},
      agentInitialized: {},
      pendingToolApproval: {},
      processedToolRequests: {},
    });

    // Create a test session
    useStore.getState().addSession({
      id: "test-session",
      name: "Test",
      workingDirectory: "/test",
      createdAt: new Date().toISOString(),
      mode: "terminal",
    });

    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe("Empty State", () => {
    it("should show empty state when no timeline, no streaming, and no running command", () => {
      render(<UnifiedTimeline sessionId="test-session" />);

      // WelcomeScreen renders empty state (no running command indicators)
      expect(screen.queryByText("Running...")).not.toBeInTheDocument();
      // Verify the WelcomeScreen container is rendered
      expect(document.querySelector(".h-full")).toBeInTheDocument();
    });

    it("should NOT show empty state when there is a running command with command text", () => {
      useStore.getState().handleCommandStart("test-session", "ls -la");

      render(<UnifiedTimeline sessionId="test-session" />);

      // Advance past 250ms LiveTerminalBlock debounce
      act(() => { vi.advanceTimersByTime(300); });

      // Empty state text should NOT be visible
      expect(screen.queryByText("Golish")).not.toBeInTheDocument();
      // Command header should show the command text
      expect(screen.getByText("ls -la")).toBeInTheDocument();
    });

    it("should show empty state when pendingCommand exists but command is null", () => {
      useStore.getState().handleCommandStart("test-session", null);

      render(<UnifiedTimeline sessionId="test-session" />);
      act(() => { vi.advanceTimersByTime(300); });

      expect(screen.queryByText("Running...")).not.toBeInTheDocument();
    });

    it("should NOT show empty state when agent is streaming", () => {
      useStore.getState().updateAgentStreaming("test-session", "Thinking...");

      render(<UnifiedTimeline sessionId="test-session" />);
      act(() => { vi.advanceTimersByTime(300); });

      expect(screen.queryByText("Golish")).not.toBeInTheDocument();
    });
  });

  describe("Running Command Display", () => {
    it("should show terminal container when command is running", () => {
      useStore.getState().handleCommandStart("test-session", "ping localhost");

      render(<UnifiedTimeline sessionId="test-session" />);
      act(() => { vi.advanceTimersByTime(300); });

      // LiveTerminalBlock should render after debounce
      expect(screen.getByText("ping localhost")).toBeInTheDocument();
    });

    it("should NOT show running indicator when pendingCommand.command is null", () => {
      useStore.getState().handleCommandStart("test-session", null);

      render(<UnifiedTimeline sessionId="test-session" />);

      // The running command section shouldn't render
      expect(screen.queryByText("Running...")).not.toBeInTheDocument();
    });

    it("should show terminal container for running command with output", () => {
      useStore.getState().handleCommandStart("test-session", "cat file.txt");
      useStore.getState().appendOutput("test-session", "line 1\nline 2\n");

      render(<UnifiedTimeline sessionId="test-session" />);
      act(() => { vi.advanceTimersByTime(300); });

      expect(screen.getByText("cat file.txt")).toBeInTheDocument();
    });

    it("should show terminal container even when pendingCommand has no output yet", () => {
      useStore.getState().handleCommandStart("test-session", "ls");

      render(<UnifiedTimeline sessionId="test-session" />);
      act(() => { vi.advanceTimersByTime(300); });

      expect(screen.getByText("ls")).toBeInTheDocument();
    });
  });

  describe("Completed Commands in Timeline", () => {
    it("should show completed command block in timeline", () => {
      useStore.getState().handleCommandStart("test-session", "echo hello");
      useStore.getState().appendOutput("test-session", "hello\n");
      useStore.getState().handleCommandEnd("test-session", 0);

      render(<UnifiedTimeline sessionId="test-session" />);

      // Command should be in the timeline (via UnifiedBlock)
      expect(screen.getByText("echo hello")).toBeInTheDocument();
    });

    it("should show multiple completed commands in order", () => {
      const store = useStore.getState();

      store.handleCommandStart("test-session", "first");
      store.appendOutput("test-session", "1\n");
      store.handleCommandEnd("test-session", 0);

      store.handleCommandStart("test-session", "second");
      store.appendOutput("test-session", "2\n");
      store.handleCommandEnd("test-session", 0);

      render(<UnifiedTimeline sessionId="test-session" />);

      screen.getAllByRole("code");
      // Both commands should be visible
      expect(screen.getByText("first")).toBeInTheDocument();
      expect(screen.getByText("second")).toBeInTheDocument();
    });
  });

  describe("Agent Streaming", () => {
    it("should not render agent streaming in terminal timeline (moved to AI chat panel)", () => {
      useStore
        .getState()
        .updateAgentStreaming("test-session", "I am thinking about your request...");

      render(<UnifiedTimeline sessionId="test-session" />);

      // Agent streaming is rendered in the separate AI chat panel, not the terminal timeline
      expect(screen.queryByText(/I am thinking about your request/)).not.toBeInTheDocument();
    });

    it("should show welcome screen when only agent streaming is active", () => {
      useStore.getState().updateAgentStreaming("test-session", "Response...");

      render(<UnifiedTimeline sessionId="test-session" />);

      // Terminal timeline shows welcome screen since no commands are running
      expect(screen.queryByText("Running...")).not.toBeInTheDocument();
    });
  });

  describe("Bug Prevention - The Issues We Fixed", () => {
    it("BUG: should NOT show Running or empty command when app starts fresh", () => {
      // Fresh state - no commands started
      render(<UnifiedTimeline sessionId="test-session" />);

      // Should show empty state (WelcomeScreen), not "Running..."
      expect(screen.queryByText("Running...")).not.toBeInTheDocument();
      expect(document.querySelector(".h-full")).toBeInTheDocument();
    });

    it("BUG: should NOT create (empty command) blocks", () => {
      const store = useStore.getState();

      // Simulate what was happening: command_start with null followed by command_end
      store.handleCommandStart("test-session", null);
      store.handleCommandEnd("test-session", 0);

      render(<UnifiedTimeline sessionId="test-session" />);

      // Should show empty state, not a block with "(empty command)"
      expect(screen.queryByText("Running...")).not.toBeInTheDocument();
      const state = useStore.getState();
      expect(selectCommandBlocksFromTimeline(state.timelines["test-session"])).toHaveLength(0);
    });

    it("terminal output before command_start SHOULD create pendingCommand (fallback for missing shell integration)", () => {
      const store = useStore.getState();

      // This simulates receiving output when no command is running (shell integration missing)
      // The new behavior is to show output even without command_start, as a fallback
      store.appendOutput("test-session", "prompt text\n");

      render(<UnifiedTimeline sessionId="test-session" />);

      // Should show the terminal block (no header, just the terminal container)
      // pendingCommand should be auto-created with null command
      expect(useStore.getState().pendingCommand["test-session"]).toBeDefined();
      expect(useStore.getState().pendingCommand["test-session"]?.command).toBeNull();
      expect(getOutputBuffer("test-session")).toBe("prompt text\n");
    });

    it("BUG: empty string command should NOT create a block", () => {
      const store = useStore.getState();

      store.handleCommandStart("test-session", "");
      store.handleCommandEnd("test-session", 0);

      render(<UnifiedTimeline sessionId="test-session" />);

      // Should show empty state
      expect(screen.queryByText("Running...")).not.toBeInTheDocument();
      const state = useStore.getState();
      expect(selectCommandBlocksFromTimeline(state.timelines["test-session"])).toHaveLength(0);
    });
  });
});
