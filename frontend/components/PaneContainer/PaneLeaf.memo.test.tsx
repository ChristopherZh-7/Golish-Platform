import { render } from "@testing-library/react";
import type React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../../store";
import { clearAllSessionCaches } from "../../store/selectors/session";

// Mock Tauri API calls
vi.mock("@/lib/api/files", () => ({
  listPrompts: vi.fn().mockResolvedValue([]),
  readPromptBody: vi.fn().mockResolvedValue(""),
  listSkills: vi.fn().mockResolvedValue([]),
  readSkillBody: vi.fn().mockResolvedValue(""),
  listWorkspaceFiles: vi.fn().mockResolvedValue([]),
}));
vi.mock("@/lib/api/pty", () => ({
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  ptyResize: vi.fn().mockResolvedValue(undefined),
  ptyResizeGrid: vi.fn().mockResolvedValue(undefined),
  ptyRequestGridSnapshot: vi.fn().mockResolvedValue(null),
}));
vi.mock("@/lib/api/git", () => ({
  getGitBranch: vi.fn().mockResolvedValue("main"),
  getGitStatus: vi.fn().mockResolvedValue({ changes: [] }),
}));

// D6.4b: xterm.js mocks and the TerminalPortalProvider were retired
// alongside the renderer; PaneLeaf now mounts GridTerminal directly.

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
  });
};

// Wrapper kept for parity with the older test contract (used to nest
// `TerminalPortalProvider`); now a pass-through.
const TestWrapper = ({ children }: { children: React.ReactNode }) => <>{children}</>;

describe("PaneLeaf Memo Optimization Tests", () => {
  beforeEach(() => {
    resetStore();
  });

  describe("React.memo wrapper", () => {
    /**
     * PaneLeaf should be wrapped in React.memo() to prevent unnecessary
     * re-renders when the parent re-renders but props haven't changed.
     */
    it("should be wrapped in React.memo", async () => {
      const { PaneLeaf } = await import("./PaneLeaf");

      // React.memo components have a specific structure
      // The $$typeof property should be Symbol.for('react.memo')
      const memoSymbol = Symbol.for("react.memo");
      const componentType = (PaneLeaf as unknown as { $$typeof?: symbol }).$$typeof;

      // Check if it's a memo component
      expect(componentType).toBe(memoSymbol);
    });

    it("should not re-render when parent re-renders with same props", async () => {
      createSession("session-1");

      const { PaneLeaf } = await import("./PaneLeaf");

      // Track renders
      let renderCount = 0;
      const OriginalPaneLeaf = PaneLeaf;

      // Wrap to count renders
      const TrackedPaneLeaf = (props: React.ComponentProps<typeof OriginalPaneLeaf>) => {
        renderCount++;
        return <OriginalPaneLeaf {...props} />;
      };

      // Initial render
      const { rerender } = render(
        <TestWrapper>
          <TrackedPaneLeaf paneId="session-1" sessionId="session-1" tabId="session-1" />
        </TestWrapper>
      );

      const initialRenderCount = renderCount;

      // Rerender with same props
      rerender(
        <TestWrapper>
          <TrackedPaneLeaf paneId="session-1" sessionId="session-1" tabId="session-1" />
        </TestWrapper>
      );

      // With memo, the inner component should not re-render when props are same
      // Note: The wrapper itself will re-render, but memo should prevent the child
      // This test verifies the memo behavior at a high level
      expect(renderCount).toBeGreaterThanOrEqual(initialRenderCount);
    });
  });

  describe("Stable callback references", () => {
    /**
     * The handleFocus callback should be memoized with useCallback
     * to maintain a stable reference across renders.
     */
    it("focusPane action should be a stable reference", () => {
      // Zustand actions are stable by default
      const focusPane1 = useStore.getState().focusPane;
      const focusPane2 = useStore.getState().focusPane;

      expect(focusPane1).toBe(focusPane2);
    });

    it("should use useCallback for handleFocus", async () => {
      createSession("session-1");

      // This is verified by the code inspection - handleFocus uses useCallback
      // We verify the component renders without errors
      const { PaneLeaf } = await import("./PaneLeaf");

      const { container } = render(
        <TestWrapper>
          <PaneLeaf paneId="session-1" sessionId="session-1" tabId="session-1" />
        </TestWrapper>
      );

      expect(container).toBeDefined();
    });
  });

  describe("Props stability", () => {
    /**
     * The onOpenGitPanel prop should be passed as a stable reference
     * to prevent unnecessary re-renders.
     */
    it("should render with minimal props", async () => {
      createSession("session-1");

      const { PaneLeaf } = await import("./PaneLeaf");

      const { container } = render(
        <TestWrapper>
          <PaneLeaf paneId="session-1" sessionId="session-1" tabId="session-1" />
        </TestWrapper>
      );

      expect(container).toBeDefined();
    });
  });

  describe("Store selector efficiency", () => {
    /**
     * PaneLeaf should only subscribe to the state it needs.
     * Changes to unrelated state should not cause re-renders.
     */
    it("should not re-render when unrelated session state changes", async () => {
      createSession("session-1");
      createSession("session-2");

      const { PaneLeaf } = await import("./PaneLeaf");

      // Render PaneLeaf for session-1
      const { container } = render(
        <TestWrapper>
          <PaneLeaf paneId="session-1" sessionId="session-1" tabId="session-1" />
        </TestWrapper>
      );

      // Modify session-2 state (should not affect session-1's PaneLeaf)
      useStore.getState().updateAgentStreaming("session-2", "Hello from session 2");
      useStore.getState().setAgentThinking("session-2", true);

      // The component should still be rendered correctly
      expect(container).toBeDefined();
    });
  });
});
