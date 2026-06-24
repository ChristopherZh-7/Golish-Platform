import { type Dispatch, type MutableRefObject, type SetStateAction, useEffect } from "react";
import { scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";
import { getSettings } from "@/lib/settings";
import { restoreBatchTerminals } from "@/lib/terminal-restore";
import { useStore } from "@/store";
import { activateConversationTerminalFromChat } from "../conversationTerminalActivation";
import { getConfiguredProviders } from "../providerConfig";

type ApprovalMode = "ask" | "run-all";
type SelectedModel = { model: string; provider: string } | null;
type CreateTerminalTab = (path?: string, autoActivate?: boolean) => Promise<string | null>;

interface UseChatStreamingSyncOptions {
  activeConvId: string | null;
  storeApprovalMode: string | null | undefined;
  setApprovalMode: Dispatch<SetStateAction<ApprovalMode>>;
  storeAiModel: SelectedModel;
  setSelectedModel: Dispatch<SetStateAction<SelectedModel>>;
  setPentestTools: Dispatch<SetStateAction<ToolConfig[]>>;
  setConfiguredProviders: Dispatch<SetStateAction<Set<string>>>;
  setChatExecutionMode: Dispatch<SetStateAction<string>>;
  planTextOffsetRef: MutableRefObject<number | null>;
  planMessageIdRef: MutableRefObject<string | null>;
  workspaceDataReady: boolean;
  pendingTermData: unknown;
  terminalRestoreInProgress: boolean;
  createTerminalTab: CreateTerminalTab;
}

/**
 * Owns the panel-level synchronisation side effects:
 *
 *  - mirroring `approvalMode` / `selectedModel` from the global store
 *    into local component state (so manual user toggles stay snappy)
 *  - one-shot terminal restore once the workspace data is ready
 *  - loading pentest tools + configured providers on mount
 *  - resetting plan refs whenever the active conversation changes
 *    (tab scroll-into-view is owned by <ConversationTabs/>)
 *  - restoring execution mode / sub-agents toggle from the terminal
 *    session after the conv switch completes
 *
 * The original panel had ~120 lines of `useEffect`s scattered between
 * state declarations and handlers; consolidating them here keeps
 * `AIChatPanel.tsx` focused on composition.
 */
export function useChatStreamingSync(opts: UseChatStreamingSyncOptions): void {
  const {
    activeConvId,
    storeApprovalMode,
    setApprovalMode,
    storeAiModel,
    setSelectedModel,
    setPentestTools,
    setConfiguredProviders,
    setChatExecutionMode,
    planTextOffsetRef,
    planMessageIdRef,
    workspaceDataReady,
    pendingTermData,
    terminalRestoreInProgress,
    createTerminalTab,
  } = opts;

  // Mirror persisted approval mode into local state on rehydrate.
  useEffect(() => {
    if (storeApprovalMode) setApprovalMode(storeApprovalMode as ApprovalMode);
  }, [storeApprovalMode, setApprovalMode]);

  // Mirror persisted model into local state.
  useEffect(() => {
    if (storeAiModel) setSelectedModel(storeAiModel);
  }, [storeAiModel, setSelectedModel]);

  // Unified terminal restore: fires on both initial boot (App.tsx sets data)
  // and project switch (HomeView sets data). Clearing the store value
  // synchronously prevents double-processing under React Strict Mode.
  useEffect(() => {
    if (!workspaceDataReady || !pendingTermData) return;
    const data = pendingTermData;
    useStore.getState().setPendingTerminalRestoreData(null);
    void restoreBatchTerminals(
      data as Parameters<typeof restoreBatchTerminals>[0],
      createTerminalTab
    );
  }, [pendingTermData, workspaceDataReady, createTerminalTab]);

  // Load installed pentest tools once on mount.
  useEffect(() => {
    scanTools()
      .then((result) => {
        if (result.success) {
          setPentestTools(result.tools.filter((t) => t.installed));
        }
      })
      .catch(() => {});
  }, [setPentestTools]);

  // Track which providers have valid API keys; refresh on settings save.
  useEffect(() => {
    const loadProviders = () => {
      getSettings()
        .then((settings) => setConfiguredProviders(getConfiguredProviders(settings)))
        .catch(() => {});
    };

    loadProviders();
    window.addEventListener("settings-updated", loadProviders);
    return () => window.removeEventListener("settings-updated", loadProviders);
  }, [setConfiguredProviders]);

  // Reset plan refs when the active conversation changes. Scrolling the active
  // tab into view is owned by <ConversationTabs/> via useChatTabsScrollbar so
  // the new/active tab reliably stays visible (older tabs scroll off-left).
  useEffect(() => {
    planTextOffsetRef.current = null;
    planMessageIdRef.current = null;
  }, [activeConvId, planMessageIdRef, planTextOffsetRef]);

  // When switching conversations, activate its terminal and restore execution
  // mode + sub-agents flag. Re-runs when terminalRestoreInProgress flips so we
  // pick up the values DB restore just wrote.
  useEffect(() => {
    if (!activeConvId) return;
    if (terminalRestoreInProgress || useStore.getState().terminalRestoreInProgress) return;
    activateConversationTerminalFromChat(activeConvId, {
      setChatExecutionMode,
      emptyExecutionMode: "chat",
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeConvId, terminalRestoreInProgress, setChatExecutionMode]);
}
