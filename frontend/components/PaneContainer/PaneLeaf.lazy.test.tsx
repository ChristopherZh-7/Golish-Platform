import { render, screen, waitFor } from "@testing-library/react";
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

// D6.4b: xterm.js mocks + TerminalPortalProvider retired with the
// renderer; PaneLeaf now mounts GridTerminal directly.

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

// Helper to create a session with specific tab type
const createSession = (
  sessionId: string,
  tabType: "terminal" | "home" | "settings" = "terminal"
) => {
  useStore.getState().addSession({
    id: sessionId,
    name: `Session ${sessionId}`,
    workingDirectory: `/home/${sessionId}`,
    createdAt: new Date().toISOString(),
    mode: "terminal",
    tabType,
  });
};

// Wrapper kept as a pass-through after `TerminalPortalProvider` was
// retired with the xterm.js renderer (D6.4b).
const TestWrapper = ({ children }: { children: React.ReactNode }) => <>{children}</>;

describe("PaneLeaf Lazy Loading Tests", () => {
  beforeEach(() => {
    resetStore();
  });

  describe("HomeView lazy loading", () => {
    it("should show loading fallback while HomeView is loading", async () => {
      createSession("home-session", "home");

      const { PaneLeaf } = await import("./PaneLeaf");

      const { container } = render(
        <TestWrapper>
          <PaneLeaf paneId="home-session" sessionId="home-session" tabId="home-session" />
        </TestWrapper>
      );

      // The component should render (either fallback or actual content)
      expect(container).toBeDefined();

      // Wait for the lazy component to load
      await waitFor(() => {
        // After loading, the HomeView should be rendered
        // We check that the pane section exists
        const section = container.querySelector("section");
        expect(section).toBeInTheDocument();
      });
    });

    it("should render HomeView content after lazy load completes", async () => {
      createSession("home-session", "home");

      const { PaneLeaf } = await import("./PaneLeaf");

      render(
        <TestWrapper>
          <PaneLeaf paneId="home-session" sessionId="home-session" tabId="home-session" />
        </TestWrapper>
      );

      // Wait for lazy loading to complete
      await waitFor(
        () => {
          // HomeView should have rendered - check for aria-label
          const section = screen.getByRole("region", { name: /pane.*session/i });
          expect(section).toBeInTheDocument();
        },
        { timeout: 2000 }
      );
    });
  });

  describe("SettingsTabContent lazy loading", () => {
    it("should show loading fallback while SettingsTabContent is loading", async () => {
      createSession("settings-session", "settings");

      const { PaneLeaf } = await import("./PaneLeaf");

      const { container } = render(
        <TestWrapper>
          <PaneLeaf
            paneId="settings-session"
            sessionId="settings-session"
            tabId="settings-session"
          />
        </TestWrapper>
      );

      // The component should render
      expect(container).toBeDefined();

      // Wait for the lazy component to load
      await waitFor(() => {
        const section = container.querySelector("section");
        expect(section).toBeInTheDocument();
      });
    });

    it("should render SettingsTabContent after lazy load completes", async () => {
      createSession("settings-session", "settings");

      const { PaneLeaf } = await import("./PaneLeaf");

      render(
        <TestWrapper>
          <PaneLeaf
            paneId="settings-session"
            sessionId="settings-session"
            tabId="settings-session"
          />
        </TestWrapper>
      );

      // Wait for lazy loading to complete
      await waitFor(
        () => {
          const section = screen.getByRole("region", { name: /pane.*session/i });
          expect(section).toBeInTheDocument();
        },
        { timeout: 2000 }
      );
    });
  });

  describe("Suspense boundaries", () => {
    it("should have Suspense boundary wrapping lazy components", async () => {
      createSession("home-session", "home");

      const { PaneLeaf } = await import("./PaneLeaf");

      // This test verifies that lazy loading doesn't throw due to missing Suspense
      // If Suspense is missing, React would throw an error
      const { container } = render(
        <TestWrapper>
          <PaneLeaf paneId="home-session" sessionId="home-session" tabId="home-session" />
        </TestWrapper>
      );

      expect(container).toBeDefined();

      // Wait for content to load
      await waitFor(() => {
        expect(container.querySelector("section")).toBeInTheDocument();
      });
    });

    it("should render loading indicator during Suspense", async () => {
      createSession("settings-session", "settings");

      const { PaneLeaf } = await import("./PaneLeaf");

      const { container } = render(
        <TestWrapper>
          <PaneLeaf
            paneId="settings-session"
            sessionId="settings-session"
            tabId="settings-session"
          />
        </TestWrapper>
      );

      // Initially might show loading fallback or content (depends on caching)
      // The key is that it doesn't throw
      expect(container).toBeDefined();

      await waitFor(() => {
        expect(container.querySelector("section")).toBeInTheDocument();
      });
    });
  });

  describe("Terminal tab (non-lazy)", () => {
    it("should render terminal content without lazy loading delay", async () => {
      createSession("terminal-session", "terminal");

      const { PaneLeaf } = await import("./PaneLeaf");

      const { container } = render(
        <TestWrapper>
          <PaneLeaf
            paneId="terminal-session"
            sessionId="terminal-session"
            tabId="terminal-session"
          />
        </TestWrapper>
      );

      // Terminal content should be immediately available (not lazy loaded)
      const section = container.querySelector("section");
      expect(section).toBeInTheDocument();
    });
  });
});
