import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/store";
import { AIChatPanel } from "./AIChatPanel";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@/hooks/useCreateTerminalTab", () => ({
  useCreateTerminalTab: () => ({ createTerminalTab: vi.fn() }),
}));

vi.mock("@/components/Engagement/ReportReadModelView", () => ({
  ReportReadModelView: ({
    operationId,
    refreshVersion,
  }: {
    operationId: string;
    refreshVersion: number;
  }) => (
    <div data-testid="ai-chat-report-read-model">
      {operationId}:v{refreshVersion}
    </div>
  ),
}));

vi.mock("./hooks/useAiChatInit", () => ({
  useAiChatInit: () => ({ pentestTools: [], configuredProviders: [] }),
}));
vi.mock("./hooks/useChatSessionInit", () => ({
  useChatSessionInit: () => ({ initializeSession: vi.fn(), generateTitleRef: { current: null } }),
}));
vi.mock("./hooks/useChatSend", () => ({
  useChatSend: () => ({ handleSend: vi.fn(), handleStop: vi.fn() }),
}));
vi.mock("./hooks/useChatConversationOps", () => ({
  useChatConversationOps: () => ({ handleNewChat: vi.fn(), handleCloseTab: vi.fn() }),
}));
vi.mock("./hooks/useChatHotkeys", () => ({
  useChatHotkeys: () => ({ handleKeyDown: vi.fn(), handleTextareaInput: vi.fn() }),
}));
vi.mock("./hooks/useChatModes", () => ({
  useChatModes: () => ({
    approvalMode: "ask",
    setApprovalMode: vi.fn(),
    chatExecutionMode: "chat",
    chatExecutionModeRef: { current: "chat" },
    setChatExecutionMode: vi.fn(),
    handleExecutionModeChange: vi.fn(),
    handleAgentModeChange: vi.fn(),
    pendingApproval: null,
    pendingApprovalRef: { current: null },
    setPendingApproval: vi.fn(),
    handleApprovalModeChange: vi.fn(),
    handleToolApprove: vi.fn(),
    handleToolApproveAlways: vi.fn(),
    handleToolDeny: vi.fn(),
  }),
}));
vi.mock("./hooks/useAiChatEvents", () => ({
  useAiChatEvents: () => ({
    contextUsage: null,
    askHumanRequest: null,
    lastDiscoverOrgId: null,
    lastDiscoverThreshold: null,
    activeWorkflow: null,
    compactionState: null,
    planTextOffsetRef: { current: null },
    planMessageIdRef: { current: null },
    handleAskHumanSubmit: vi.fn(),
    handleAskHumanSkip: vi.fn(),
  }),
}));
vi.mock("./hooks/useTaskPlanState", () => ({
  useTaskPlanState: () => ({
    activeAiSessionId: "ai-reporting-session",
    taskPlan: null,
    stagePlans: null,
    planTargetIdx: -1,
  }),
}));
vi.mock("./useChatAutoScroll", () => ({
  useChatAutoScroll: () => ({
    messagesContainerRef: { current: null },
    userScrolledUpRef: { current: false },
  }),
}));
vi.mock("./conversationTerminalActivation", () => ({
  activateConversationTerminalFromChat: vi.fn(),
}));

vi.mock("./ConversationTabs", () => ({ ConversationTabs: () => <div /> }));
vi.mock("./MessageBlock", () => ({ MessageBlock: () => <div data-testid="chat-message" /> }));
vi.mock("./ExecutionModePicker", () => ({ ExecutionModePicker: () => <div /> }));
vi.mock("./ChatModelSelector", () => ({ ChatModelSelector: () => <div /> }));
vi.mock("./ContextUsageRing", () => ({ ContextUsageRing: () => <div /> }));
vi.mock("./StageMarker", () => ({ StageMarker: () => <div /> }));
vi.mock("./StageProgressBar", () => ({ StageProgressBar: () => <div /> }));
vi.mock("./StageResetMenu", () => ({ StageResetMenu: () => <div /> }));
vi.mock("./AgentStatusIndicator", () => ({ AgentStatusIndicator: () => <div /> }));
vi.mock("./ChatSubComponents", () => ({
  AskHumanInline: () => <div />,
  CompactionNotice: () => <div />,
  WorkflowProgress: () => <div />,
}));

const CONVERSATION_ID = "reporting-conversation";
const TERMINAL_ID = "reporting-terminal";

describe("AIChatPanel Reporting production entry", () => {
  beforeEach(() => {
    useStore.setState({
      conversations: {
        [CONVERSATION_ID]: {
          id: CONVERSATION_ID,
          title: "Reporting",
          messages: [
            {
              id: "message-1",
              role: "assistant",
              content: "Reporting ready",
              timestamp: 1,
            },
          ],
          createdAt: 1,
          aiSessionId: "ai-reporting-session",
          aiInitialized: true,
          isStreaming: false,
        },
      },
      conversationOrder: [CONVERSATION_ID],
      activeConversationId: CONVERSATION_ID,
      conversationTerminals: { [CONVERSATION_ID]: [TERMINAL_ID] },
      sessions: {
        [TERMINAL_ID]: {
          id: TERMINAL_ID,
          name: "Reporting terminal",
          workingDirectory: "/tmp/reporting",
          createdAt: "2026-07-13T00:00:00Z",
          mode: "agent",
          reportingReadModelHint: {
            operationId: "operation-reporting-1",
            refreshVersion: 1,
          },
        },
      },
      activeSessionId: TERMINAL_ID,
      workspaceDataReady: true,
      terminalRestoreInProgress: false,
      pendingTerminalRestoreData: null,
      pendingAskHuman: {},
    });
  });

  it("mounts and refreshes the DB-backed report from the active conversation session hint", () => {
    render(<AIChatPanel />);
    expect(screen.getByTestId("ai-chat-report-read-model")).toHaveTextContent(
      "operation-reporting-1:v1"
    );

    act(() => {
      useStore
        .getState()
        .setReportingReadModelHint(TERMINAL_ID, { operationId: "operation-reporting-1" });
    });
    expect(screen.getByTestId("ai-chat-report-read-model")).toHaveTextContent(
      "operation-reporting-1:v2"
    );

    act(() => {
      useStore
        .getState()
        .setReportingReadModelHint(TERMINAL_ID, { operationId: "operation-reporting-2" });
    });
    expect(screen.getByTestId("ai-chat-report-read-model")).toHaveTextContent(
      "operation-reporting-2:v1"
    );

    act(() => {
      useStore.getState().setReportingReadModelHint(TERMINAL_ID, null);
    });
    expect(screen.queryByTestId("ai-chat-report-read-model")).not.toBeInTheDocument();
  });
});
