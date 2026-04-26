import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { createDbAutoSaver } from "../../lib/conversation-db-sync";
import { useStore } from "../../store";

/**
 * Owns DB auto-save lifecycle and close-time synchronization hooks.
 */
export function useDbAutoSync(): void {
  // Auto-save conversation state to PostgreSQL on store changes + window close.
  useEffect(() => {
    return createDbAutoSaver(
      () => useStore.getState().currentProjectPath ?? null,
      (listener) => useStore.subscribe(listener),
      () => {
        const s = useStore.getState();
        return {
          conversations: s.conversations,
          conversationOrder: s.conversationOrder,
          activeConversationId: s.activeConversationId,
          conversationTerminals: s.conversationTerminals,
          sessions: s.sessions,
          timelines: s.timelines,
          selectedAiModel: s.selectedAiModel,
          approvalMode: s.approvalMode,
          terminalRestoreInProgress: s.terminalRestoreInProgress,
          pendingTerminalRestoreData: s.pendingTerminalRestoreData,
        };
      }
    );
  }, []);

  // Ensure pentest browser resources are closed on normal unload and Tauri flush-state.
  useEffect(() => {
    const closePentestBrowser = () => {
      invoke("pentest_browser_close").catch(() => {});
    };

    closePentestBrowser();

    const handleBeforeUnload = () => {
      closePentestBrowser();
    };

    let unlistenFlushState: (() => void) | null = null;
    listen("flush-state", closePentestBrowser)
      .then((unlisten) => {
        unlistenFlushState = unlisten;
      })
      .catch(() => {});

    window.addEventListener("beforeunload", handleBeforeUnload);

    return () => {
      window.removeEventListener("beforeunload", handleBeforeUnload);
      unlistenFlushState?.();
    };
  }, []);
}
