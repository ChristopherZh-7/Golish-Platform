//! Plain database helpers (no Tauri annotations) for targets/recon updates.
//! Used by the `#[tauri::command]` wrappers in `cmds.rs` and by other modules
//! that need to write through directly.

use sqlx::PgPool;
use uuid::Uuid;

use super::types::{detect_type, Target, TargetRow, TargetType};
use super::recon::ReconUpdate;


// ============================================================================
// Standalone DB functions for AI tool integration (no Tauri state needed)
// ============================================================================

pub async fn db_target_add(
    pool: &PgPool,
    name: &str,
    value: &str,
    target_type: Option<&str>,
    project_path: Option<&str>,
    source: &str,
    parent_id: Option<Uuid>,
) -> Result<Target, String> {
    let tt = target_type.map(TargetType::from_str).unwrap_or_else(|| detect_type(value));
    let n = if name.is_empty() { value } else { name };

    let existing = sqlx::query_as::<_, TargetRow>(
        r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                  status::text, source, parent_id, ports,
                  real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type,
                  created_at, updated_at
           FROM targets WHERE value=$1 AND ($2 IS NULL OR project_path = $2 OR project_path = '') LIMIT 1"#,
    )
    .bind(value)
    .bind(project_path)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    if let Some(r) = existing {
        return Ok(Target::from(r));
    }

    let row = sqlx::query_as::<_, TargetRow>(
        r#"INSERT INTO targets (name, target_type, value, tags, notes, scope, grp, project_path, source, parent_id)
           VALUES ($1, $2::target_type, $3, '[]', '', 'in'::scope_type, 'default', $4, $5, $6)
           RETURNING id, name, target_type::text, value, tags, notes, scope::text,
                     status::text, source, parent_id, ports,
                     real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type,
                     created_at, updated_at"#,
    )
    .bind(n)
    .bind(tt.as_str())
    .bind(value)
    .bind(project_path)
    .bind(source)
    .bind(parent_id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(Target::from(row))
}

pub async fn db_target_list(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<Target>, String> {
    let rows = sqlx::query_as::<_, TargetRow>(
        r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                  status::text, source, parent_id, ports,
                     real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type,
                     created_at, updated_at
           FROM targets WHERE project_path = $1
           ORDER BY created_at"#,
    )
    .bind(project_path)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.into_iter().map(Target::from).collect())
}

pub async fn db_target_update_status(
    pool: &PgPool,
    id: Uuid,
    status: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE targets SET status=$1::target_status, updated_at=NOW() WHERE id=$2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn db_target_update_recon(
    pool: &PgPool,
    id: Uuid,
    ports: &serde_json::Value,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE targets SET ports=$1, updated_at=NOW() WHERE id=$2",
    )
    .bind(ports)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Extended recon update accepting all httpx/nmap-derived fields.
/// Only non-empty values overwrite existing data.
pub async fn db_target_update_recon_extended(
    pool: &PgPool,
    id: Uuid,
    updates: &ReconUpdate,
) -> Result<(), String> {
    sqlx::query(
        r#"UPDATE targets SET
            real_ip       = CASE WHEN $1 != '' THEN $1 ELSE real_ip END,
            cdn_waf       = CASE WHEN $2 != '' THEN $2 ELSE cdn_waf END,
            http_title    = CASE WHEN $3 != '' THEN $3 ELSE http_title END,
            http_status   = COALESCE($4, http_status),
            webserver     = CASE WHEN $5 != '' THEN $5 ELSE webserver END,
            os_info       = CASE WHEN $6 != '' THEN $6 ELSE os_info END,
            content_type  = CASE WHEN $7 != '' THEN $7 ELSE content_type END,
            ports         = CASE WHEN $8::jsonb = '[]'::jsonb THEN ports
                            ELSE (
                                SELECT COALESCE(jsonb_agg(merged), '[]'::jsonb) FROM (
                                    -- Existing ports that are NOT in the new data (keep as-is)
                                    SELECT ep AS merged
                                    FROM jsonb_array_elements(ports) ep
                                    WHERE NOT EXISTS (
                                        SELECT 1 FROM jsonb_array_elements($8::jsonb) np
                                        WHERE (ep->>'port') = (np->>'port')
                                          AND COALESCE(ep->>'proto','tcp') = COALESCE(np->>'proto','tcp')
                                    )
                                    UNION ALL
                                    -- New/updated ports: merge with existing entry if present
                                    SELECT CASE
                                        WHEN ep IS NOT NULL THEN ep || np
                                        ELSE np
                                    END AS merged
                                    FROM jsonb_array_elements($8::jsonb) np
                                    LEFT JOIN LATERAL (
                                        SELECT ep FROM jsonb_array_elements(ports) ep
                                        WHERE (ep->>'port') = (np->>'port')
                                          AND COALESCE(ep->>'proto','tcp') = COALESCE(np->>'proto','tcp')
                                        LIMIT 1
                                    ) existing(ep) ON true
                                ) sub
                            ) END,
            updated_at    = NOW()
           WHERE id = $9"#,
    )
    .bind(&updates.real_ip)
    .bind(&updates.cdn_waf)
    .bind(&updates.http_title)
    .bind(updates.http_status)
    .bind(&updates.webserver)
    .bind(&updates.os_info)
    .bind(&updates.content_type)
    .bind(&updates.ports)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}
