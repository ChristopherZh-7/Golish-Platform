/**
 * App-shell slice for the Zustand store.
 *
 * Holds root-level state that doesn't belong to a specific domain slice but
 * needs to be globally reachable: app focus / visibility, workspace bootstrap
 * status, ZAP running flag, the chat-panel toggle, the deferred terminal
 * restore payload, and the current project name/path.
 *
 * Centralizing this here lets `store/index.ts` stay slim (~60 lines) and
 * matches the slice composition pattern used by `dialog.ts`, `git.ts`, etc.
 */

import type { PersistedTerminalData } from "@/lib/workspace-storage";
import type { SliceCreator } from "./types";

type PendingTerminalRestoreMap = Record<string, PersistedTerminalData[]> | null;

export interface AppShellState {
  appIsFocused: boolean;
  appIsVisible: boolean;
  terminalRestoreInProgress: boolean;
  workspaceDataReady: boolean;
  zapRunning: boolean;
  pendingTerminalRestoreData: PendingTerminalRestoreMap;
  chatPanelVisible: boolean;
  currentProjectName: string | null;
  currentProjectPath: string | null;
}

export interface AppShellActions {
  setAppIsFocused: (focused: boolean) => void;
  setAppIsVisible: (visible: boolean) => void;
  setTerminalRestoreInProgress: (inProgress: boolean) => void;
  setWorkspaceDataReady: (ready: boolean) => void;
  setZapRunning: (running: boolean) => void;
  setPendingTerminalRestoreData: (data: PendingTerminalRestoreMap) => void;
  setChatPanelVisible: (visible: boolean) => void;
  toggleChatPanel: () => void;
  setCurrentProject: (name: string | null, path?: string | null) => void;
}

export interface AppShellSlice extends AppShellState, AppShellActions {}

export const initialAppShellState: AppShellState = {
  appIsFocused: true,
  appIsVisible: true,
  terminalRestoreInProgress: false,
  workspaceDataReady: false,
  zapRunning: false,
  pendingTerminalRestoreData: null,
  chatPanelVisible: true,
  currentProjectName: null,
  currentProjectPath: null,
};

export const createAppShellSlice: SliceCreator<AppShellSlice> = (set) => ({
  ...initialAppShellState,

  setAppIsFocused: (focused) =>
    set((state) => {
      state.appIsFocused = focused;
    }),
  setAppIsVisible: (visible) =>
    set((state) => {
      state.appIsVisible = visible;
    }),
  setTerminalRestoreInProgress: (inProgress) =>
    set((state) => {
      state.terminalRestoreInProgress = inProgress;
    }),
  setWorkspaceDataReady: (ready) =>
    set((state) => {
      state.workspaceDataReady = ready;
    }),
  setZapRunning: (running) =>
    set((state) => {
      state.zapRunning = running;
    }),
  setPendingTerminalRestoreData: (data) =>
    set((state) => {
      state.pendingTerminalRestoreData = data;
    }),
  setChatPanelVisible: (visible) =>
    set((state) => {
      state.chatPanelVisible = visible;
    }),
  toggleChatPanel: () =>
    set((state) => {
      state.chatPanelVisible = !state.chatPanelVisible;
    }),
  setCurrentProject: (name, path) =>
    set((state) => {
      state.currentProjectName = name;
      state.currentProjectPath = path ?? null;
    }),
});
