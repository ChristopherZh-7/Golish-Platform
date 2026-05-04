import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../../store";
import { clearAllSessionCaches } from "../../store/selectors/session";

/**
 * Tests for textarea availability while agent is busy.
 *
 * The textarea should remain typeable while the agent is processing,
 * so users can prepare their next message. Only the send button
 * should be disabled during agent activity. The textarea should only
 * be disabled when the session is dead (unrecoverable).
 */

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

const getSubmitButton = () => {
  const buttons = screen.getAllByRole("button");
  return buttons.find((btn) => !btn.getAttribute("aria-label")?.includes("Switch"));
};

describe("UnifiedInput: typing while agent is busy", () => {
  beforeEach(() => {
    resetStore();
    vi.clearAllMocks();
  });

  it("textarea remains enabled while agent is responding", async () => {
    createSession("s1");
    const { UnifiedInput } = await import("./UnifiedInput");
    render(<UnifiedInput sessionId="s1" />);

    const input = screen.getByTestId("unified-input");

    act(() => {
      useStore.getState().setAgentResponding("s1", true);
    });

    expect(input).not.toBeDisabled();
  });

  it("textarea remains enabled while streaming blocks are active", async () => {
    createSession("s1");
    const { UnifiedInput } = await import("./UnifiedInput");
    render(<UnifiedInput sessionId="s1" />);

    const input = screen.getByTestId("unified-input");

    act(() => {
      useStore.setState({
        streamingBlocks: {
          s1: [{ type: "text", content: "streaming..." }],
        },
      });
    });

    expect(input).not.toBeDisabled();
  });

  it("textarea remains enabled during context compaction", async () => {
    createSession("s1");
    const { UnifiedInput } = await import("./UnifiedInput");
    render(<UnifiedInput sessionId="s1" />);

    const input = screen.getByTestId("unified-input");

    act(() => {
      useStore.getState().setCompacting("s1", true);
    });

    expect(input).not.toBeDisabled();
  });

  it("user can type into textarea while agent is responding", async () => {
    createSession("s1");
    const { UnifiedInput } = await import("./UnifiedInput");
    render(<UnifiedInput sessionId="s1" />);

    const input = screen.getByTestId("unified-input");

    act(() => {
      useStore.getState().setAgentResponding("s1", true);
    });

    await userEvent.type(input, "next message");

    expect(input).toHaveValue("next message");
  });

  // SKIP: send-button-disabled-while-busy logic was removed.
  // UnifiedInput is now PTY-only (AI submit lives in AIChatPanel/useChatSend),
  // and `isInputDisabled` is `isSessionDead` only — agent activity no longer
  // disables the send button (or the textarea) so users can pre-type.
  it.skip("send button is disabled while agent is responding", async () => {
    createSession("s1");
    const { UnifiedInput } = await import("./UnifiedInput");
    render(<UnifiedInput sessionId="s1" />);

    const input = screen.getByTestId("unified-input");
    await userEvent.type(input, "some text");

    act(() => {
      useStore.getState().setAgentResponding("s1", true);
    });

    const submitButton = getSubmitButton();
    expect(submitButton).toBeDisabled();
  });

  // SKIP: same reason — the send button never disables on agent activity.
  it.skip("send button re-enables after agent finishes and input has content", async () => {
    createSession("s1");
    const { UnifiedInput } = await import("./UnifiedInput");
    render(<UnifiedInput sessionId="s1" />);

    const input = screen.getByTestId("unified-input");

    act(() => {
      useStore.getState().setAgentResponding("s1", true);
    });

    await userEvent.type(input, "prepared message");

    const submitButton = getSubmitButton();
    expect(submitButton).toBeDisabled();

    act(() => {
      useStore.getState().setAgentResponding("s1", false);
    });

    expect(submitButton).not.toBeDisabled();
  });

  it("textarea is disabled when session is dead", async () => {
    createSession("s1");
    const { UnifiedInput } = await import("./UnifiedInput");
    render(<UnifiedInput sessionId="s1" />);

    const input = screen.getByTestId("unified-input");

    act(() => {
      useStore.getState().setSessionDead("s1", true);
    });

    expect(input).toBeDisabled();
  });

  // SKIP: outdated. Today UnifiedInput.handleSubmit pipes Enter straight
  // through to ptyWrite without consulting agent-busy state — the AI guard
  // lives in AIChatPanel/useChatSend now. The behaviour we want here
  // ("don't submit AI prompt while busy") is asserted in
  // useChatSend.test.* instead.
  it.skip("Enter does not submit while agent is busy", async () => {
    createSession("s1");
    const { sendPromptSession } = await import("@/lib/ai");

    const { UnifiedInput } = await import("./UnifiedInput");
    render(<UnifiedInput sessionId="s1" />);

    const input = screen.getByTestId("unified-input");

    await userEvent.type(input, "queued message");

    act(() => {
      useStore.getState().setAgentResponding("s1", true);
    });

    await userEvent.keyboard("{Enter}");

    expect(sendPromptSession).not.toHaveBeenCalled();
    expect(input).toHaveValue("queued message");
  });

  it("typed message submits via PTY after agent finishes", async () => {
    createSession("s1");
    const { ptyWrite } = await import("@/lib/api/pty");

    const { UnifiedInput } = await import("./UnifiedInput");
    render(<UnifiedInput sessionId="s1" />);

    const input = screen.getByTestId("unified-input");

    act(() => {
      useStore.getState().setAgentResponding("s1", true);
    });

    await userEvent.type(input, "follow-up");

    act(() => {
      useStore.getState().setAgentResponding("s1", false);
    });

    await userEvent.keyboard("{Enter}");

    // UnifiedInput is PTY-only after the AIChatPanel split; assert that
    // Enter writes to the terminal rather than the (legacy) AI submit path.
    expect(ptyWrite).toHaveBeenCalledWith("s1", "follow-up\n");
  });
});
