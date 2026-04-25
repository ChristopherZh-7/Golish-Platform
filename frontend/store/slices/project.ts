/**
 * Project slice for the Zustand store.
 *
 * Manages the currently-active workspace project (name + path) and the ZAP
 * sidecar's running flag. Switching the active project automatically stops
 * any running ZAP scanner attached to the previous path and points the
 * scanner at the new path.
 */

import type { SliceCreator } from "./types";

export interface ProjectState {
  /** Display name of the active workspace project (null = none open). */
  currentProjectName: string | null;
  /** Filesystem path of the active workspace project (null = none open). */
  currentProjectPath: string | null;
  /** Whether the ZAP sidecar is currently running for the active project. */
  zapRunning: boolean;
}

export interface ProjectActions {
  /**
   * Set the current project name and path (for workspace persistence).
   *
   * If the path changes, the ZAP sidecar attached to the previous path is
   * stopped (best-effort) and the scanner is then re-pointed at the new path.
   * `zapRunning` is reset to `false` so any UI badge clears immediately.
   */
  setCurrentProject: (name: string | null, path?: string | null) => void;
  /** Update the cached ZAP-running flag (driven by sidecar status events). */
  setZapRunning: (running: boolean) => void;
}

export interface ProjectSlice extends ProjectState, ProjectActions {}

export const initialProjectState: ProjectState = {
  currentProjectName: null,
  currentProjectPath: null,
  zapRunning: false,
};

export const createProjectSlice: SliceCreator<ProjectSlice> = (set, get) => ({
  ...initialProjectState,

  setZapRunning: (running) =>
    set((state) => {
      state.zapRunning = running;
    }),

  setCurrentProject: (name, path) => {
    const prevPath = get().currentProjectPath;
    set((state) => {
      state.currentProjectName = name;
      state.currentProjectPath = path ?? null;
    });
    if (prevPath && prevPath !== (path ?? null)) {
      // Path changed: optimistically clear the running flag so the UI badge
      // disappears, then ask the sidecar to stop the previous scan and
      // re-point at the new path.
      set((state) => {
        state.zapRunning = false;
      });
      import("@/lib/pentest/zap-api").then(({ zapStop, zapUpdateProject }) => {
        zapStop(prevPath)
          .catch(() => {})
          .then(() => {
            zapUpdateProject(path ?? null).catch(() => {});
          });
      });
    } else {
      // First open or same path: just sync the sidecar.
      import("@/lib/pentest/zap-api").then(({ zapUpdateProject }) => {
        zapUpdateProject(path ?? null).catch(() => {});
      });
    }
  },
});
