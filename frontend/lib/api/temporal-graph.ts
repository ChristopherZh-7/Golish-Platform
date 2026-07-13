/**
 * Authoritative structured temporal graph IPC.
 *
 * This API is intentionally separate from the legacy `kg_*` helpers. Scope is
 * a closed generated type; callers cannot provide an actor id or project path
 * as authority. The backend resolves the trusted local operator and verifies
 * the stable project/org binding before every query or rebuild.
 */

import type { KnowledgeGraphGenerationView } from "@/lib/generated/KnowledgeGraphGenerationView";
import type { KnowledgeGraphQueryRequest } from "@/lib/generated/KnowledgeGraphQueryRequest";
import type { KnowledgeGraphQueryResultView } from "@/lib/generated/KnowledgeGraphQueryResultView";
import type { KnowledgeGraphRebuildRequest } from "@/lib/generated/KnowledgeGraphRebuildRequest";
import type { KnowledgeGraphScopeRequest } from "@/lib/generated/KnowledgeGraphScopeRequest";
import { invoke } from "./client";

export type {
  KnowledgeGraphGenerationView,
  KnowledgeGraphQueryRequest,
  KnowledgeGraphQueryResultView,
  KnowledgeGraphRebuildRequest,
  KnowledgeGraphScopeRequest,
};

export function organizationGraphScope(
  projectScopeId: string,
  organizationIdAtTime: string
): KnowledgeGraphScopeRequest {
  return {
    scopeKind: "organization",
    projectScopeId,
    organizationIdAtTime,
  };
}

export function globalSanitizedGraphScope(): KnowledgeGraphScopeRequest {
  return { scopeKind: "global_sanitized" };
}

export async function knowledgeGraphQueryScoped(
  request: KnowledgeGraphQueryRequest
): Promise<KnowledgeGraphQueryResultView> {
  return invoke<KnowledgeGraphQueryResultView>("knowledge_graph_query_scoped", { request });
}

export async function knowledgeGraphRebuildScope(
  request: KnowledgeGraphRebuildRequest
): Promise<KnowledgeGraphGenerationView> {
  return invoke<KnowledgeGraphGenerationView>("knowledge_graph_rebuild_scope", { request });
}
