import { useCallback } from "react";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import { getGitBranch, gitStatus, ptyCreate } from "@/lib/tauri";
import { useStore } from "@/store";

/**
 * Hook that provides a function to create new terminal tabs.
 * Handles PTY creation, git status, and conversation-terminal linking.
 * AI is managed by the right-side AI chat panel, not per-terminal.
 */
export function useCreateTerminalTab() {
  const createTerminalTab = useCallback(
    async (workingDirectory?: string, skipConversationLink?: boolean): Promise<string | null> => {
      const {
        addSession,
        updateGitBranch,
        setGitStatus,
        setGitStatusLoading,
        activeConversationId,
        addTerminalToConversation,
      } = useStore.getState();

      try {
        const session = await ptyCreate(workingDirectory);

        addSession({
          id: session.id,
          name: "Terminal",
          workingDirectory: session.working_directory,
          createdAt: new Date().toISOString(),
          mode: "terminal",
        });

        // Link terminal to the active conversation (unless skipped, e.g. during workspace restore)
        if (!skipConversationLink && activeConversationId) {
          addTerminalToConversation(activeConversationId, session.id);
        }

        // Fetch git branch and status in the background
        void (async () => {
          setGitStatusLoading(session.id, true);
          try {
            const [branch, status] = await Promise.all([
              getGitBranch(session.working_directory),
              gitStatus(session.working_directory),
            ]);
            updateGitBranch(session.id, branch);
            setGitStatus(session.id, status);
          } catch {
            // Silently ignore - not a git repo or git not installed
          } finally {
            setGitStatusLoading(session.id, false);
          }
        })();

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
