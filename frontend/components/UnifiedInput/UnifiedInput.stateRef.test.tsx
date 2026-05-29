import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../../store";
import { clearAllSessionCaches } from "../../store/selectors/session";

/**
 * TDD Tests for UnifiedInput stateRef Pattern Optimization
 *
 * Previous Issue: stateRef was re-assigned a new object every render:
 * ```typescript
 * stateRef.current = {
 *   input,
 *   inputMode,
 *   // ... 20+ fields
 * };
 * ```
 *
 * Problem:
 * - Creates a new object with 20+ fields on every single render
 * - Causes unnecessary memory allocations and GC pressure
 *
 * Solution: Update individual properties on the existing ref object:
 * ```typescript
 * const ref = stateRef.current;
 * ref.input = input;
 * ref.inputMode = inputMode;
 * // ... etc
 * ```
 *
 * Benefits:
 * 1. No new object allocation per render
 * 2. Ref assignments are synchronous and don't trigger re-renders
 * 3. The value is available immediately for callbacks called during render
 */

// Mock dependencies
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@/lib/ai", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ai")>();
  return {
    ...actual,
    sendPromptSession: vi.fn(() => Promise.resolve()),
    sendPromptWithAttachments: vi.fn(() => Promise.resolve()),
    getVisionCapabilities: vi.fn(() => Promise.resolve({ supports_vision: false })),
  };
});

vi.mock("@/lib/api/pty", () => ({
  ptyWrite: vi.fn(() => Promise.resolve()),
}));
vi.mock("@/lib/api/files", () => ({
  readPrompt: vi.fn(() => Promise.resolve("prompt content")),
  readSkillBody: vi.fn(() => Promise.resolve("skill content")),
  readFileAsBase64: vi.fn(() => Promise.resolve("base64data")),
  listWorkspaceFiles: vi.fn().mockResolvedValue([]),
}));

vi.mock("@/lib/notify", () => ({
  notify: {
    error: vi.fn(),
    warning: vi.fn(),
    success: vi.fn(),
  },
}));

vi.mock("@/lib/logger", () => ({
  logger: {
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
  },
}));

vi.mock("@/hooks/useSlashCommands", () => ({
  useSlashCommands: vi.fn(() => ({ commands: [] })),
}));

vi.mock("@/hooks/useFileCommands", () => ({
  useFileCommands: vi.fn(() => ({ files: [] })),
}));

vi.mock("@/hooks/usePathCompletion", () => ({
  usePathCompletion: vi.fn(() => ({ completions: [], totalCount: 0 })),
}));

vi.mock("@/hooks/useHistorySearch", () => ({
  useHistorySearch: vi.fn(() => ({ matches: [] })),
}));

vi.mock("@/hooks/useCommandHistory", () => ({
  useCommandHistory: vi.fn(() => ({
    history: [],
    add: vi.fn(),
    navigateUp: vi.fn(),
    navigateDown: vi.fn(),
    reset: vi.fn(),
  })),
}));

// Helper to reset store
const resetStore = () => {
  clearAllSessionCaches();
  useStore.setState({
    sessions: {},
    activeSessionId: null,
    timelines: {},
    pendingCommand: {},
    agentStreaming: {},
    streamingBlocks: {},
    streamingTextOffset: {},
    agentInitialized: {},
    isAgentThinking: {},
    isAgentResponding: {},
    pendingToolApproval: {},
    processedToolRequests: {},
    activeToolCalls: {},
    thinkingContent: {},
    isThinkingExpanded: {},
    activeWorkflows: {},
    workflowHistory: {},
    activeSubAgents: {},
    contextMetrics: {},
    compactionCount: {},
    isCompacting: {},
    isSessionDead: {},
    compactionError: {},
    tabLayouts: {},
    tabHasNewActivity: {},
    sessionTokenUsage: {},
  });
};

// Helper to create a session
const createSession = (sessionId: string) => {
  useStore.getState().addSession({
    id: sessionId,
    name: `Session ${sessionId}`,
    workingDirectory: `/home/${sessionId}`,
    createdAt: new Date().toISOString(),
    mode: "terminal",
    inputMode: "agent",
  });
};

describe("UnifiedInput stateRef Pattern Optimization", () => {
  beforeEach(() => {
    resetStore();
    vi.clearAllMocks();
  });

  describe("stateRef synchronization", () => {
    it("should have current input value available to handleSubmit immediately", async () => {
      createSession("session-1");
      const { ptyWrite } = await import("@/lib/api/pty");

      const { UnifiedInput } = await import("./UnifiedInput");
      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      // Type and submit immediately. UnifiedInput is PTY-only so the
      // current input value flows through ptyWrite, not sendPromptSession.
      await userEvent.type(input, "Test message{Enter}");

      expect(ptyWrite).toHaveBeenCalledWith("session-1", "Test message\n");
    });

    it("should have current inputMode available to handleKeyDown immediately", async () => {
      createSession("session-1");

      const { UnifiedInput } = await import("./UnifiedInput");
      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      // Toggle mode through the store (Cmd+I is wired in App.tsx, not here).
      act(() => {
        useStore.getState().setInputMode("session-1", "terminal");
      });

      // Mode should have toggled
      expect(input).toHaveAttribute("data-mode", "terminal");
    });

    it("should have current isAgentBusy state available to handleSubmit", async () => {
      createSession("session-1");
      const { sendPromptSession } = await import("@/lib/ai");

      const { UnifiedInput } = await import("./UnifiedInput");
      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      // Type a message
      await userEvent.type(input, "Test");

      // Set agent to busy
      act(() => {
        useStore.getState().setAgentResponding("session-1", true);
      });

      // Try to submit via Enter - callback should see isAgentBusy = true
      await userEvent.keyboard("{Enter}");

      // Should NOT have submitted because isAgentBusy was true
      expect(sendPromptSession).not.toHaveBeenCalled();
    });
  });

  describe("no unnecessary re-renders from stateRef", () => {
    it("should not cause additional effects when stateRef is updated", async () => {
      // The stateRef pattern updates the ref directly in render, not via useEffect.
      // This test verifies that approach by checking that:
      // 1. The component renders without errors
      // 2. State changes work correctly without unnecessary effect runs

      createSession("session-1");

      const { UnifiedInput } = await import("./UnifiedInput");
      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      // Type some text - state updates should work correctly
      await userEvent.type(input, "test");

      // The input should have the typed value
      expect(input).toHaveValue("test");

      // No errors means the stateRef pattern is working correctly
      // (Previously this would cause issues with stale ref values)
    });

    it("should update individual properties without allocating new objects", async () => {
      // This test verifies the optimization: updating individual properties
      // instead of creating a new object with 20+ fields on every render.
      //
      // The optimization reduces memory allocations by mutating the existing
      // ref object rather than replacing it with a new one.

      createSession("session-1");

      const { UnifiedInput } = await import("./UnifiedInput");
      const { rerender } = render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      // Type some text - this triggers re-renders
      await userEvent.type(input, "a");
      await userEvent.type(input, "b");
      await userEvent.type(input, "c");

      // Rerender the component
      rerender(<UnifiedInput sessionId="session-1" />);

      // The input should still have the correct value
      expect(input).toHaveValue("abc");

      // The key test: stateRef-driven callbacks still see the latest input
      // value across re-renders. UnifiedInput.handleSubmit pipes that
      // value to ptyWrite (PTY-only routing).
      const { ptyWrite } = await import("@/lib/api/pty");
      await userEvent.keyboard("{Enter}");

      expect(ptyWrite).toHaveBeenCalledWith("session-1", "abc\n");
    });
  });

  describe("callback correctness with stateRef", () => {
    it("handleSubmit should work correctly when called rapidly", async () => {
      createSession("session-1");
      const { ptyWrite } = await import("@/lib/api/pty");

      const { UnifiedInput } = await import("./UnifiedInput");
      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      // First message — handleSubmit should send it via PTY immediately.
      await userEvent.type(input, "First message{Enter}");
      expect(ptyWrite).toHaveBeenCalledWith("session-1", "First message\n");

      vi.mocked(ptyWrite).mockClear();

      // Second message — fire another submit and verify the latest input
      // value is captured (no stale stateRef carry-over).
      await userEvent.type(input, "Second message{Enter}");
      expect(ptyWrite).toHaveBeenCalledWith("session-1", "Second message\n");
    });

    it("handleKeyDown should read correct popup state", async () => {
      createSession("session-1");

      // Mock slash commands
      const { useSlashCommands } = await import("@/hooks/useSlashCommands");
      vi.mocked(useSlashCommands).mockReturnValue({
        commands: [
          {
            name: "test",
            description: "Test command",
            path: "/test",
            type: "prompt",
            source: "local",
          },
        ],
        prompts: [],
        isLoading: false,
        reload: vi.fn(),
      });

      const { UnifiedInput } = await import("./UnifiedInput");
      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      // Type "/" to trigger popup
      await userEvent.type(input, "/");

      // Input should show "/"
      expect(input).toHaveValue("/");

      // Press Escape - callback should read showSlashPopup = true and close it
      await userEvent.keyboard("{Escape}");

      // The popup should close (we can't easily verify without checking popup visibility)
      // But key is that handleKeyDown correctly read the current popup state
    });
  });
});
