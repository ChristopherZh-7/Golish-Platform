import { useCallback } from "react";
import { ptyCreate } from "@/lib/api/pty";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import { suppressNextTerminalAutoFocus } from "@/lib/terminal/terminalAutoFocus";
import { useStore } from "@/store";

/**
 * Hook that provides a function to create new terminal tabs.
 * Handles PTY creation, git status, and conversation-terminal linking.
 * AI is managed by the right-side AI chat panel, not per-terminal.
 */
export function useCreateTerminalTab() {
  const createTerminalTab = useCallback(
    async (
      workingDirectory?: string,
      skipConversationLink?: boolean,
      scrollback?: string,
      logicalTerminalId?: string
    ): Promise<string | null> => {
      const { addSession, activeConversationId, addTerminalToConversation } = useStore.getState();

      try {
        const session = await ptyCreate(workingDirectory);

        // Queue scrollback BEFORE addSession triggers React rendering
        if (scrollback) {
          TerminalInstanceManager.setPendingScrollback(session.id, scrollback);
        }

        addSession({
          id: session.id,
          logicalTerminalId: logicalTerminalId ?? crypto.randomUUID(),
          name: "Terminal",
          workingDirectory: session.working_directory,
          createdAt: new Date().toISOString(),
          mode: "terminal",
        });

        // Link terminal to the active conversation (unless skipped, e.g. during workspace restore)
        if (!skipConversationLink && activeConversationId) {
          addTerminalToConversation(activeConversationId, session.id);
        }

        // Chat-first focus: when the user explicitly opens a tab (the "+" button
        // / Cmd+T — i.e. NOT a workspace restore or auto-create-on-send, which
        // pass `skipConversationLink`), land the cursor in the AI chat panel
        // rather than letting the terminal grab it on mount.
        if (!skipConversationLink) {
          suppressNextTerminalAutoFocus(session.id);
          requestAnimationFrame(() =>
            document.querySelector<HTMLTextAreaElement>("[data-ai-chat-input]")?.focus()
          );
        }

        return session.id;
      } catch (e) {
        logger.error("Failed to create new tab:", e);
        notify.error("Failed to create new tab");
        return null;
      }
    },
    []
  );

  return { createTerminalTab };
}
