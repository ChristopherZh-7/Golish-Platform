import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../../store";
import { clearAllSessionCaches } from "../../store/selectors/session";

/**
 * TDD Tests for UnifiedInput Callback Stability
 *
 * Issues:
 * 1. handleSubmit has 13 dependencies - recreated during streaming when
 *    isAgentResponding and streamingBlocks.length change
 * 2. handleKeyDown has 24 dependencies - recreated on every keystroke
 *    because `input` is in the dependency array
 *
 * Solution:
 * Use a ref pattern to store current state values and create stable callbacks
 * with empty dependency arrays that read from the ref.
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
    gitStatus: {},
    gitStatusLoading: {},
    gitCommitMessage: {},
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

describe("UnifiedInput Callback Stability", () => {
  beforeEach(() => {
    resetStore();
    vi.clearAllMocks();
  });

  describe("handleSubmit stability", () => {
    it("should maintain stable reference when streaming state changes", async () => {
      createSession("session-1");

      const { UnifiedInput } = await import("./UnifiedInput");

      // Custom wrapper to render the component
      const TestWrapper = () => <UnifiedInput sessionId="session-1" />;

      const { rerender } = render(<TestWrapper />);

      // Get initial button reference
      // Get submit button - it doesn't have accessible name so we query all buttons
      // and find the one that's the submit (the one with SendHorizontal icon)
      const buttons = screen.getAllByRole("button");
      const submitButton = buttons.find(
        (btn) => !btn.getAttribute("aria-label")?.includes("Switch")
      );
      expect(submitButton).toBeDefined();

      // Simulate streaming state change (which previously caused callback recreation)
      act(() => {
        useStore.getState().setAgentResponding("session-1", true);
      });

      rerender(<TestWrapper />);

      // The submit button's onClick should reference the same stable callback
      // Note: We can't directly test the callback reference, but we can verify
      // the component doesn't unnecessarily re-create handlers by checking
      // that the button still works correctly after state changes

      // Simulate streaming content change
      act(() => {
        useStore.setState({
          streamingBlocks: {
            "session-1": [{ type: "text", content: "streaming..." }],
          },
        });
      });

      rerender(<TestWrapper />);

      // Agent responding should disable the button
      expect(submitButton).toBeDisabled();

      // Clear streaming state
      act(() => {
        useStore.getState().setAgentResponding("session-1", false);
        useStore.getState().clearStreamingBlocks("session-1");
      });

      rerender(<TestWrapper />);

      // Button should be enabled again (but still need input content)
      // The key point: the callback should work correctly throughout these state changes
    });

    // SKIP: outdated since the AI submit path moved to AIChatPanel/useChatSend.
    // UnifiedInput.handleSubmit now exclusively writes to the PTY, so it
    // neither calls sendPromptSession nor disables the send button while the
    // agent is responding (the textarea stays usable on purpose so users can
    // pre-type the next message). Re-enable when the AI submit path moves
    // back into UnifiedInput.
    it.skip("should work correctly with current state after streaming ends", async () => {
      createSession("session-1");
      const { sendPromptSession } = await import("@/lib/ai");

      const { UnifiedInput } = await import("./UnifiedInput");

      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");
      const submitButton = screen.getByTestId("send-button");

      await userEvent.type(input, "Hello world");
      expect(input).toHaveValue("Hello world");

      act(() => {
        useStore.getState().setAgentResponding("session-1", true);
      });
      expect(submitButton).toBeDisabled();

      act(() => {
        useStore.getState().setAgentResponding("session-1", false);
      });

      await userEvent.clear(input);
      await userEvent.type(input, "New message");
      await userEvent.click(submitButton);

      expect(sendPromptSession).toHaveBeenCalledWith("session-1", "New message");
    });
  });

  describe("handleKeyDown stability", () => {
    it("should maintain stable reference when input changes", async () => {
      createSession("session-1");

      const { UnifiedInput } = await import("./UnifiedInput");

      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      // Type several characters - each keystroke previously recreated handleKeyDown
      await userEvent.type(input, "abc");

      // The key handler should work correctly throughout typing
      expect(input).toHaveValue("abc");

      // Type more and verify the handler still works
      await userEvent.type(input, "def");
      expect(input).toHaveValue("abcdef");
    });

    it("should correctly read current input value when Enter is pressed", async () => {
      createSession("session-1");
      const { ptyWrite } = await import("@/lib/api/pty");

      const { UnifiedInput } = await import("./UnifiedInput");

      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      // Type a message and press Enter — UnifiedInput.handleSubmit always
      // routes to ptyWrite (AI submit lives in AIChatPanel/useChatSend).
      await userEvent.type(input, "Test message{Enter}");

      expect(ptyWrite).toHaveBeenCalledWith("session-1", "Test message\n");
    });

    // Skipped: `useUnifiedInputState` now hardcodes `inputMode = "terminal"`
    // (see useUnifiedInputState.ts:57/108/176 + REFACTOR_PLAN.md §"Hardcoded
    // mode"). AI-mode lives in `AIChatPanel` and `useChatSend`; the store's
    // `setInputMode` action no longer feeds back into UnifiedInput's
    // `data-mode` attribute, so toggling via the store can never produce
    // `data-mode="auto"`. Re-enable only if/when UnifiedInput regains a
    // multi-mode toggle path.
    it.skip("should correctly handle mode toggle via store API (stale: hardcoded terminal)", async () => {
      createSession("session-1");

      const { UnifiedInput } = await import("./UnifiedInput");

      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");

      await userEvent.type(input, "some text");
      act(() => {
        useStore.getState().setInputMode("session-1", "auto");
      });

      expect(input).toHaveAttribute("data-mode", "auto");
    });
  });

  describe("callback correctness with ref pattern", () => {
    it("handleSubmit should access isAgentBusy state at call time, not creation time", async () => {
      createSession("session-1");
      const { sendPromptSession } = await import("@/lib/ai");

      const { UnifiedInput } = await import("./UnifiedInput");

      render(<UnifiedInput sessionId="session-1" />);

      const input = screen.getByTestId("unified-input");
      // Get submit button - it doesn't have accessible name, find by type="button" without aria-label
      const buttons = screen.getAllByRole("button");
      const submitButton = buttons.find(
        (btn) => !btn.getAttribute("aria-label")?.includes("Switch")
      );
      expect(submitButton).toBeDefined();

      // Type a message
      await userEvent.type(input, "Test");

      // Set agent to busy state AFTER the callback was created
      act(() => {
        useStore.getState().setAgentResponding("session-1", true);
      });

      // Try to submit - the callback should check current state and block
      if (submitButton) {
        await userEvent.click(submitButton);
      }

      // Should NOT have called sendPromptSession because isAgentBusy was true at call time
      expect(sendPromptSession).not.toHaveBeenCalled();
    });

    it("handleKeyDown should access popup state at call time, not creation time", async () => {
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

      // Type "/" to trigger slash command popup
      await userEvent.type(input, "/");

      // The popup should be open and ArrowDown should navigate
      // This tests that the handler correctly reads current showSlashPopup state
      await userEvent.keyboard("{ArrowDown}");

      // If the handler used stale state, navigation wouldn't work
      // The key is that the handler works correctly even though it was
      // created before we typed "/"
    });
  });
});
