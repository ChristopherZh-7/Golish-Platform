/**
 * Compat re-export — actual implementation lives at `@/lib/api/sidecar`.
 *
 * Surface narrowed to symbols that are imported through this wrapper path
 * (M2.3 cleanup — previously `export *` re-exported the entire module
 * including symbols that no consumer uses through `@/lib/sidecar`).
 *
 * See ADR-0009 Phase 2.
 */

export {
  type Artifact,
  applyAllArtifacts,
  applyAllPatches,
  applyArtifact,
  applyPatch,
  discardArtifact,
  discardPatch,
  getAppliedPatches,
  getCurrentSession,
  getPendingArtifacts,
  getSessionLog,
  getSessionState,
  getSidecarStatus,
  getStagedPatches,
  previewArtifact,
  type SidecarEventType,
  type SidecarStatus,
  type StagedPatch,
} from "@/lib/api/sidecar";
