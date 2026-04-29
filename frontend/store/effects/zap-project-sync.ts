import { useStore } from "../index";

/**
 * Side-effect subscriber that keeps the ZAP sidecar in sync with the active project.
 *
 * When `currentProjectPath` changes:
 *  - Resets the in-flight "running" flag so the UI doesn't show a stale state for the
 *    previous project.
 *  - Tears down the previous project's sidecar (if any) and updates ZAP's project so
 *    requests are routed to the right working directory.
 *
 * This used to live inline in `store/index.ts`, which coupled the generic store to a
 * pentest-specific feature module (architecture layer inversion). The subscriber is
 * now opt-in: callers (typically `main.tsx`) install it once at app bootstrap.
 */
export function installZapProjectSync(): () => void {
  let prevProjectPath: string | null = null;

  return useStore.subscribe((state) => {
    const curPath = state.currentProjectPath;
    if (curPath === prevProjectPath) return;
    const prev = prevProjectPath;
    prevProjectPath = curPath;

    if (prev && prev !== curPath) {
      useStore.getState().setZapRunning(false);
      import("@/lib/pentest/zap-api").then(({ zapStop, zapUpdateProject }) => {
        zapStop(prev)
          .catch(() => {})
          .then(() => zapUpdateProject(curPath).catch(() => {}));
      });
    } else {
      import("@/lib/pentest/zap-api").then(({ zapUpdateProject }) => {
        zapUpdateProject(curPath).catch(() => {});
      });
    }
  });
}
