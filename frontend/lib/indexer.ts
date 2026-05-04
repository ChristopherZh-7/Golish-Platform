/**
 * Compat re-export — actual implementation lives at `@/lib/api/indexer`.
 *
 * Surface narrowed to symbols that are imported through this wrapper path
 * (M2.3 cleanup — previously `export *` re-exported the entire module
 * including symbols that no consumer uses through `@/lib/indexer`). Consumers
 * that need newer surface should import from `@/lib/api/indexer` directly
 * (or use `api.indexer.*` from `@/lib/api`).
 *
 * See ADR-0009 Phase 2.
 */

export {
  addIndexedCodebase,
  type CodebaseInfo,
  createGitWorktree,
  detectMemoryFiles,
  getAllIndexedFiles,
  getIndexerWorkspace,
  indexDirectory,
  initIndexer,
  isIndexerInitialized,
  listGitBranches,
  listIndexedCodebases,
  listProjectsForHome,
  listRecentDirectories,
  type ProjectInfo,
  type RecentDirectory,
  reindexCodebase,
  removeIndexedCodebase,
  searchCode,
  searchFiles,
  updateCodebaseMemoryFile,
} from "@/lib/api/indexer";
