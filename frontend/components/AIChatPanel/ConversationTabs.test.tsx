import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatConversation } from "@/store/slices/conversation";
import { ConversationTabs } from "./ConversationTabs";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("./useChatTabsScrollbar", () => ({
  useChatTabsScrollbar: () => ({
    tabsRef: { current: null },
    tabsHovered: false,
    setTabsHovered: vi.fn(),
    scrollThumb: { left: 0, width: 0, visible: false },
    handleThumbDragStart: vi.fn(),
  }),
}));

const CONVERSATION: ChatConversation = {
  id: "conv-a",
  title: "Conversation A",
  messages: [],
  createdAt: 1,
  aiSessionId: "ai-a",
  aiInitialized: true,
  isStreaming: false,
};

describe("ConversationTabs destructive-reset lock", () => {
  it("blocks select, close, new-chat, and history actions while disabled", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    const onNewChat = vi.fn();
    const onToggleHistory = vi.fn();

    const { container } = render(
      <ConversationTabs
        conversations={[CONVERSATION]}
        activeConvId={CONVERSATION.id}
        showHistory={false}
        disabled
        onSelect={onSelect}
        onClose={onClose}
        onNewChat={onNewChat}
        onToggleHistory={onToggleHistory}
      />
    );

    fireEvent.click(container.querySelector('[data-conv-id="conv-a"]')!);
    fireEvent.click(screen.getByTitle("ai.newChat"));
    fireEvent.click(screen.getByTitle("ai.history"));
    fireEvent.click(screen.getAllByRole("button")[1]);

    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(onNewChat).not.toHaveBeenCalled();
    expect(onToggleHistory).not.toHaveBeenCalled();
  });
});
