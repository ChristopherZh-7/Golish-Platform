/**
 * Store-aware project helpers + compat re-export of the IPC facade.
 *
 * The IPC layer lives at `@/lib/api/projects`. New callers should
 * import directly from there for the IPC functions; this file's only
 * unique responsibility is the two store-coupled helpers below.
 *
 * Schema E (2026-05-17): `useCurrentProjectMode` removed — UI now derives
 * the pentest-like vs redteam-like distinction from the organization-tree
 * shape (see `hooks/useProjectOrgShape.ts`).
 */

import { useStore } from "@/store";

// Re-export the entire IPC facade so existing
// `import { saveProject, ... } from "@/lib/projects"` paths keep working.
export * from "@/lib/api/projects";

/** Get the current project's root path from the store. */
export function getProjectPath(): string | null {
  return useStore.getState().currentProjectPath;
}

/** Helper to create a projectPath param for backend invoke calls. */
export function ppParam(): { projectPath: string | null } {
  return { projectPath: getProjectPath() };
}
