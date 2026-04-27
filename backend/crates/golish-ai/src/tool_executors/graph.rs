//! Graph knowledge base tool executors.
//!
//! Bridges the LLM-facing `graph_*` tools onto the typed
//! [`golish_graphiti::GraphClient`]. When the client is not configured
//! (e.g. database not yet attached), graph tool calls return a graceful
//! "not available" error instead of crashing the agent loop.
//!
//! Entity-name resolution: agents typically reference entities by their
//! human-readable name (e.g. `"10.0.0.5"`). When a tool argument can be
//! either a UUID or a name, we first try to parse it as a UUID and fall
//! back to a substring-match lookup against the entity store.

use serde_json::json;
use uuid::Uuid;

use golish_graphiti::{GraphClient, GraphEntity};

use super::common::{error_result, extract_string_param, ToolResult};

/// Execute graph tool calls (graph_add_entity, graph_add_relation,
/// graph_search, graph_neighbors, graph_attack_paths).
///
/// Delegates to [`GraphClient`]. Returns graceful errors if the graph
/// backend is not available.
pub async fn execute_graph_tool(
    tool_name: &str,
    args: &serde_json::Value,
    graph_client: Option<&GraphClient>,
) -> Option<ToolResult> {
    let graph_tools = [
        "graph_add_entity",
        "graph_add_relation",
        "graph_search",
        "graph_neighbors",
        "graph_attack_paths",
    ];
    if !graph_tools.contains(&tool_name) {
        return None;
    }

    let client = match graph_client {
        Some(c) => c,
        None => {
            return Some(error_result(
                "Graph knowledge base not available (not configured)",
            ));
        }
    };

    match tool_name {
        "graph_add_entity" => {
            let entity_type = match extract_string_param(args, &["entity_type", "type"]) {
                Some(t) => t,
                None => {
                    return Some(error_result(
                        "graph_add_entity requires 'entity_type' (host/service/vulnerability/credential/technique/endpoint)",
                    ));
                }
            };
            let name = match extract_string_param(args, &["name"]) {
                Some(n) => n,
                None => {
                    return Some(error_result(
                        "graph_add_entity requires a non-empty 'name' parameter",
                    ));
                }
            };
            let properties = args
                .get("properties")
                .cloned()
                .unwrap_or_else(|| json!({}));

            match client
                .upsert_entity(&entity_type, &name, properties, None)
                .await
            {
                Ok(entity) => Some((
                    json!({
                        "success": true,
                        "entity": entity_to_json(&entity),
                    }),
                    true,
                )),
                Err(e) => Some(error_result(format!("graph_add_entity failed: {e}"))),
            }
        }

        "graph_add_relation" => {
            let from_ref = match extract_string_param(args, &["from_entity", "from", "source"]) {
                Some(s) => s,
                None => return Some(error_result("graph_add_relation requires 'from_entity'")),
            };
            let to_ref = match extract_string_param(args, &["to_entity", "to", "target"]) {
                Some(s) => s,
                None => return Some(error_result("graph_add_relation requires 'to_entity'")),
            };
            let relation_type = match extract_string_param(args, &["relation_type", "type"]) {
                Some(s) => s,
                None => {
                    return Some(error_result(
                        "graph_add_relation requires 'relation_type' (runs_service/has_vulnerability/exploited_by/lateral_move/authenticates_to/exposes_endpoint)",
                    ));
                }
            };
            let properties = args
                .get("properties")
                .cloned()
                .unwrap_or_else(|| json!({}));

            let from_id = match resolve_entity_id(client, &from_ref).await {
                Ok(id) => id,
                Err(msg) => return Some(error_result(format!("from_entity: {msg}"))),
            };
            let to_id = match resolve_entity_id(client, &to_ref).await {
                Ok(id) => id,
                Err(msg) => return Some(error_result(format!("to_entity: {msg}"))),
            };

            match client
                .upsert_relation(from_id, to_id, &relation_type, properties)
                .await
            {
                Ok(rel) => Some((
                    json!({
                        "success": true,
                        "relation": {
                            "id": rel.id,
                            "from_entity_id": rel.from_entity_id,
                            "to_entity_id": rel.to_entity_id,
                            "relation_type": rel.relation_type,
                            "properties": rel.properties,
                            "created_at": rel.created_at.to_rfc3339(),
                        }
                    }),
                    true,
                )),
                Err(e) => Some(error_result(format!("graph_add_relation failed: {e}"))),
            }
        }

        "graph_search" => {
            let query = match extract_string_param(args, &["query", "q"]) {
                Some(q) => q,
                None => {
                    return Some(error_result(
                        "graph_search requires a non-empty 'query' parameter",
                    ));
                }
            };
            let entity_type = args
                .get("entity_type")
                .and_then(|v| v.as_str())
                .map(String::from);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10)
                .min(100) as i64;

            match client
                .search_entities(&query, entity_type.as_deref(), limit)
                .await
            {
                Ok(entities) => {
                    let results: Vec<serde_json::Value> =
                        entities.iter().map(entity_to_json).collect();
                    let count = results.len();
                    Some((
                        json!({
                            "entities": results,
                            "count": count,
                            "query": query,
                        }),
                        true,
                    ))
                }
                Err(e) => Some(error_result(format!("graph_search failed: {e}"))),
            }
        }

        "graph_neighbors" => {
            let entity_ref = match extract_string_param(args, &["entity", "entity_id", "id", "name"]) {
                Some(s) => s,
                None => {
                    return Some(error_result(
                        "graph_neighbors requires 'entity' (name or UUID)",
                    ));
                }
            };
            let relation_type = args
                .get("relation_type")
                .and_then(|v| v.as_str())
                .map(String::from);

            let entity_id = match resolve_entity_id(client, &entity_ref).await {
                Ok(id) => id,
                Err(msg) => return Some(error_result(msg)),
            };

            match client
                .get_neighbors(entity_id, relation_type.as_deref())
                .await
            {
                Ok(rows) => {
                    let neighbors: Vec<serde_json::Value> = rows
                        .iter()
                        .map(|(rel, ent)| {
                            json!({
                                "relation": {
                                    "id": rel.id,
                                    "relation_type": rel.relation_type,
                                    "properties": rel.properties,
                                    "created_at": rel.created_at.to_rfc3339(),
                                },
                                "entity": entity_to_json(ent),
                            })
                        })
                        .collect();
                    let count = neighbors.len();
                    Some((
                        json!({
                            "entity_id": entity_id,
                            "neighbors": neighbors,
                            "count": count,
                        }),
                        true,
                    ))
                }
                Err(e) => Some(error_result(format!("graph_neighbors failed: {e}"))),
            }
        }

        "graph_attack_paths" => {
            let entity_ref = match extract_string_param(args, &["from_entity", "entity", "from"]) {
                Some(s) => s,
                None => {
                    return Some(error_result(
                        "graph_attack_paths requires 'from_entity' (name or UUID)",
                    ));
                }
            };
            let max_depth = args
                .get("max_depth")
                .and_then(|v| v.as_i64())
                .unwrap_or(5)
                .clamp(1, 20) as i32;

            let from_id = match resolve_entity_id(client, &entity_ref).await {
                Ok(id) => id,
                Err(msg) => return Some(error_result(msg)),
            };

            match client.find_attack_paths(from_id, max_depth).await {
                Ok(paths) => {
                    let path_jsons: Vec<serde_json::Value> = paths
                        .iter()
                        .map(|path| {
                            let entities: Vec<serde_json::Value> =
                                path.iter().map(entity_to_json).collect();
                            json!({
                                "length": entities.len(),
                                "entities": entities,
                            })
                        })
                        .collect();
                    let count = path_jsons.len();
                    Some((
                        json!({
                            "from_entity_id": from_id,
                            "max_depth": max_depth,
                            "paths": path_jsons,
                            "count": count,
                        }),
                        true,
                    ))
                }
                Err(e) => Some(error_result(format!("graph_attack_paths failed: {e}"))),
            }
        }

        _ => None,
    }
}

/// Resolve a user-supplied entity reference to a [`Uuid`].
///
/// First attempts to parse the string as a UUID. If that fails, falls back
/// to a name search via [`GraphClient::search_entities`] and returns the
/// most recently updated match. Returns a human-readable error message on
/// failure suitable for direct surfacing to the LLM.
async fn resolve_entity_id(client: &GraphClient, reference: &str) -> Result<Uuid, String> {
    if let Ok(id) = Uuid::parse_str(reference) {
        return Ok(id);
    }
    match client.search_entities(reference, None, 1).await {
        Ok(matches) => matches
            .into_iter()
            .next()
            .map(|e| e.id)
            .ok_or_else(|| format!("entity '{reference}' not found in the graph")),
        Err(e) => Err(format!("entity lookup failed: {e}")),
    }
}

/// Convert a [`GraphEntity`] to a stable JSON shape for tool results.
fn entity_to_json(entity: &GraphEntity) -> serde_json::Value {
    json!({
        "id": entity.id,
        "entity_type": entity.entity_type,
        "name": entity.name,
        "properties": entity.properties,
        "session_id": entity.session_id,
        "project_id": entity.project_id,
        "created_at": entity.created_at.to_rfc3339(),
        "updated_at": entity.updated_at.to_rfc3339(),
    })
}
