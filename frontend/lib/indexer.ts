/**
 * Compat re-export — actual implementation lives at `@/lib/api/indexer`.
 *
 * Surface narrowed to symbols that are imported through this wrapper path
 * (M2.3 cleanup — previously `export *` re-exported the entire module
 * including symbols that no consumer uses through `@/lib/indexer`). Consumers
 * that need newer surface should import from `@/lib/api/indexer` directly
 * (or use `api.indexer.*` from `@/lib/api`).
 */

export {
  addIndexedCodebase,
  type CodebaseInfo,
  detectMemoryFiles,
  getAllIndexedFiles,
  getIndexerWorkspace,
  indexDirectory,
  initIndexer,
  isIndexerInitialized,
  listIndexedCodebases,
  listRecentDirectories,
  type RecentDirectory,
  reindexCodebase,
  removeIndexedCodebase,
  searchCode,
  searchFiles,
  updateCodebaseMemoryFile,
} from "@/lib/api/indexer";
