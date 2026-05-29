import { useCallback } from "react";
import { ptyCreate } from "@/lib/api/pty";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
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
