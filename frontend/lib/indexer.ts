/**
 * Compat re-export — actual implementation lives at `@/lib/api/indexer`.
 *
 * Kept for backward compatibility with existing imports. New code should
 * import from `@/lib/api/indexer` or use `api.indexer.*` from `@/lib/api`.
 *
 * See ADR-0009 Phase 2.
 */

export * from "@/lib/api/indexer";
