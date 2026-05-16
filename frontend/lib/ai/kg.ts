/**
 * Knowledge graph query wrappers (P-KG).
 *
 * Exposes the three `kg_*` Tauri commands as typed functions so any
 * panel that wants to surface KG state (settings page, side-bar,
 * project overview, etc.) can call them without re-inventing the
 * `snake_case` ↔ `camelCase` parameter shuffle.
 *
 * The agent already mutates the graph via the `graph_*` LLM tools, so
 * intentionally NO write helpers are exported here.
 */

import { invoke } from "@/lib/api/client";

export type KgEntityType =
  | "host"
  | "service"
  | "vulnerability"
  | "credential"
  | "technique"
  | "endpoint";

export type KgRelationType =
  | "runs_service"
  | "has_vulnerability"
  | "exploited_by"
  | "lateral_move"
  | "authenticates_to"
  | "exposes_endpoint";

export interface KgEntity {
  id: string;
  entity_type: KgEntityType | string;
  name: string;
  properties: Record<string, unknown>;
  project_id?: string | null;
  created_at: string;
  updated_at: string;
}

export interface KgRelation {
  id: string;
  from_entity_id: string;
  to_entity_id: string;
  relation_type: KgRelationType | string;
  properties: Record<string, unknown>;
  created_at: string;
}

export interface KgNeighbor {
  relation: KgRelation;
  entity: KgEntity;
}

export interface KgListOptions {
  projectId?: string | null;
  entityType?: KgEntityType | string;
  limit?: number;
}

/**
 * Most-recently-updated entities for the project. Backend caps `limit`
 * to [1, 500] and returns an empty array on DB error.
 */
export async function kgListEntities(opts: KgListOptions = {}): Promise<KgEntity[]> {
  return invoke("kg_list_entities", {
    projectId: opts.projectId ?? null,
    entityType: opts.entityType,
    limit: opts.limit,
  });
}

export interface KgSearchOptions {
  entityType?: KgEntityType | string;
  limit?: number;
}

/**
 * Name substring search (case-insensitive on the backend). Backend caps
 * `limit` to [1, 200] and returns an empty array on DB error.
 */
export async function kgSearchEntities(
  query: string,
  opts: KgSearchOptions = {}
): Promise<KgEntity[]> {
  return invoke("kg_search_entities", {
    query,
    entityType: opts.entityType,
    limit: opts.limit,
  });
}

/**
 * Outgoing edges from `entityId` paired with their destination entity.
 * Backend returns an empty array on DB error or invalid UUID.
 */
export async function kgGetNeighbors(
  entityId: string,
  relationType?: KgRelationType | string
): Promise<KgNeighbor[]> {
  return invoke("kg_get_neighbors", {
    entityId,
    relationType,
  });
}

/**
 * Group a flat list of entities by `entity_type`. Useful for rendering
 * KG snapshots as collapsible sections in panels.
 */
export function groupEntitiesByType(
  entities: KgEntity[]
): Record<string, KgEntity[]> {
  const map: Record<string, KgEntity[]> = {};
  for (const ent of entities) {
    const key = String(ent.entity_type);
    if (!map[key]) map[key] = [];
    map[key].push(ent);
  }
  return map;
}
