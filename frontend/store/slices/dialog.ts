/**
 * Dialog slice for the Zustand store.
 *
 * Owns the open/close state for all app-level dialogs and overlays that were
 * previously local state in App.tsx. Centralizing them lets any component
 * open or close a dialog without prop-drilling through the shell.
 */

import type { SliceCreator } from "./types";

export interface DialogState {
  commandPaletteOpen: boolean;
  quickOpenDialogOpen: boolean;
  settingsDialogOpen: boolean;
  settingsSection: string;
  shortcutsHelpOpen: boolean;
  recordingsPanelOpen: boolean;
  bottomTerminalOpen: boolean;
}

export interface DialogActions {
  setCommandPaletteOpen: (open: boolean) => void;
  setQuickOpenDialogOpen: (open: boolean) => void;
  setSettingsDialogOpen: (open: boolean) => void;
  setSettingsSection: (section: string) => void;
  setShortcutsHelpOpen: (open: boolean) => void;
  setRecordingsPanelOpen: (open: boolean) => void;
  setBottomTerminalOpen: (open: boolean) => void;
  toggleBottomTerminal: () => void;
}

export interface DialogSlice extends DialogState, DialogActions {}

export const initialDialogState: DialogState = {
  commandPaletteOpen: false,
  quickOpenDialogOpen: false,
  settingsDialogOpen: false,
  settingsSection: "environment",
  shortcutsHelpOpen: false,
  recordingsPanelOpen: false,
  bottomTerminalOpen: true,
};

export const createDialogSlice: SliceCreator<DialogSlice> = (set) => ({
  ...initialDialogState,

  setCommandPaletteOpen: (open) =>
    set((state) => {
      state.commandPaletteOpen = open;
    }),
  setQuickOpenDialogOpen: (open) =>
    set((state) => {
      state.quickOpenDialogOpen = open;
    }),
  setSettingsDialogOpen: (open) =>
    set((state) => {
      state.settingsDialogOpen = open;
    }),
  setSettingsSection: (section) =>
    set((state) => {
      state.settingsSection = section;
    }),
  setShortcutsHelpOpen: (open) =>
    set((state) => {
      state.shortcutsHelpOpen = open;
    }),
  setRecordingsPanelOpen: (open) =>
    set((state) => {
      state.recordingsPanelOpen = open;
    }),
  setBottomTerminalOpen: (open) =>
    set((state) => {
      state.bottomTerminalOpen = open;
    }),
  toggleBottomTerminal: () =>
    set((state) => {
      state.bottomTerminalOpen = !state.bottomTerminalOpen;
    }),
});
