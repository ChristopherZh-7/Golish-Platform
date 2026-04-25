import { useEffect, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { getSettings } from "@/lib/settings";
import { restoreBatchTerminals } from "@/lib/terminal-restore";
import { scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";
import { useStore } from "@/store";
import { getConfiguredProviders } from "../providerConfig";

type ApprovalMode = "ask" | "allowlist" | "run-all";
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
  setChatExecutionMode: Dispatch<SetStateAction<"chat" | "task">>;
  setChatUseSubAgents: Dispatch<SetStateAction<boolean>>;
  chatUseSubAgents: boolean;
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
 *  - scrolling the active conversation tab into view and resetting plan
 *    refs whenever the active conversation changes
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
    setChatUseSubAgents,
    chatUseSubAgents,
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
    void restoreBatchTerminals(data as Parameters<typeof restoreBatchTerminals>[0], createTerminalTab);
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

  // Reset plan refs + scroll the active conversation tab into view when
  // the active conversation changes. The tab strip lives in
  // <ConversationTabs/> so we look the active tab up by data-attribute.
  useEffect(() => {
    planTextOffsetRef.current = null;
    planMessageIdRef.current = null;
    if (!activeConvId) return;
    const activeTab = document.querySelector(`[data-conv-id="${activeConvId}"]`);
    activeTab?.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
  }, [activeConvId, planMessageIdRef, planTextOffsetRef]);

  // When switching conversations, activate its terminal and restore execution
  // mode + sub-agents flag. Re-runs when terminalRestoreInProgress flips so we
  // pick up the values DB restore just wrote.
  useEffect(() => {
    if (!activeConvId) return;
    if (terminalRestoreInProgress || useStore.getState().terminalRestoreInProgress) return;
    const store = useStore.getState();
    const terminals = store.conversationTerminals[activeConvId];
    if (terminals && terminals.length > 0) {
      const firstTerminal = terminals[0];
      if (store.sessions[firstTerminal] && store.activeSessionId !== firstTerminal) {
        store.setActiveSession(firstTerminal);
      }
      for (const tid of terminals) {
        const sess = store.sessions[tid];
        if (sess?.executionMode === "task") {
          setChatExecutionMode("task");
          break;
        }
      }
      const hasAgents = terminals.some((tid) => store.sessions[tid]?.useAgents);
      if (hasAgents !== chatUseSubAgents) setChatUseSubAgents(hasAgents);
    } else {
      setChatExecutionMode("chat");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeConvId, terminalRestoreInProgress]);
}
