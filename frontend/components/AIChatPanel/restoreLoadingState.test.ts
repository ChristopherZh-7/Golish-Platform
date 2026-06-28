import { describe, expect, it } from "vitest";
import { shouldShowChatRestoreLoading } from "./restoreLoadingState";

describe("shouldShowChatRestoreLoading", () => {
  it("shows loading before workspace data is ready", () => {
    expect(
      shouldShowChatRestoreLoading({
        workspaceDataReady: false,
        terminalRestoreInProgress: false,
        pendingTerminalRestoreData: null,
        activeSessionId: null,
      })
    ).toBe(true);
  });

  it("shows loading while restored terminal data is pending or running", () => {
    expect(
      shouldShowChatRestoreLoading({
        workspaceDataReady: true,
        terminalRestoreInProgress: false,
        pendingTerminalRestoreData: { conv: [] },
        activeSessionId: "term-1",
      })
    ).toBe(true);

    expect(
      shouldShowChatRestoreLoading({
        workspaceDataReady: true,
        terminalRestoreInProgress: true,
        pendingTerminalRestoreData: null,
        activeSessionId: "term-1",
      })
    ).toBe(true);
  });

  it("keeps showing loading during the conversation-to-terminal binding gap", () => {
    expect(
      shouldShowChatRestoreLoading({
        workspaceDataReady: true,
        terminalRestoreInProgress: false,
        pendingTerminalRestoreData: null,
        activeSessionId: null,
      })
    ).toBe(true);
  });

  it("allows the normal empty prompt once the workspace and active session are ready", () => {
    expect(
      shouldShowChatRestoreLoading({
        workspaceDataReady: true,
        terminalRestoreInProgress: false,
        pendingTerminalRestoreData: null,
        activeSessionId: "term-1",
      })
    ).toBe(false);
  });
});
