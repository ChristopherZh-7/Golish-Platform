interface ChatRestoreLoadingInput {
  workspaceDataReady: boolean;
  terminalRestoreInProgress: boolean;
  pendingTerminalRestoreData: unknown;
  activeSessionId: string | null;
}

export function shouldShowChatRestoreLoading({
  workspaceDataReady,
  terminalRestoreInProgress,
  pendingTerminalRestoreData,
  activeSessionId,
}: ChatRestoreLoadingInput): boolean {
  return (
    !workspaceDataReady ||
    terminalRestoreInProgress ||
    pendingTerminalRestoreData != null ||
    !activeSessionId
  );
}
