use golish_app_core::GolishError;
use uuid::Uuid;

use golish_app_core::DbState;

pub use golish_core::vault::{
    deobfuscate, deobfuscate as deobfuscate_value, obfuscate, obfuscate as obfuscate_value,
    VaultEntry, VaultEntrySafe, VaultEntryType,
};

use golish_core::time::{now_ts, ts_from_dt};
use golish_db::repo::vault::VaultSafeRow;

/// Map a golish-db safe projection row to the frontend `VaultEntrySafe` DTO.
/// (A free fn rather than a `From` impl: both types are foreign to this crate.)
fn row_to_safe(r: VaultSafeRow) -> VaultEntrySafe {
    VaultEntrySafe {
        id: r.id.to_string(),
        name: r.name,
        entry_type: VaultEntryType::from_str(&r.entry_type),
        username: r.username,
        notes: r.notes,
        project: r.project,
        tags: serde_json::from_value(r.tags).unwrap_or_default(),
        status: r.status,
        source_url: r.source_url,
        last_validated_at: r.last_validated_at.map(ts_from_dt),
        created_at: ts_from_dt(r.created_at),
        updated_at: ts_from_dt(r.updated_at),
    }
}

#[tauri::command]
pub async fn vault_list(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<VaultEntrySafe>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows = golish_db::repo::vault::list_safe_by_project(pool, project_path.as_deref()).await?;
    Ok(rows.into_iter().map(row_to_safe).collect())
}

#[tauri::command]
pub async fn vault_add(
    state: tauri::State<'_, DbState>,
    name: String,
    entry_type: VaultEntryType,
    value: String,
    username: Option<String>,
    notes: Option<String>,
    project: Option<String>,
    tags: Option<Vec<String>>,
    source_url: Option<String>,
    project_path: Option<String>,
) -> Result<VaultEntrySafe, GolishError> {
    let pool = state.pool_ready().await?;
    let ts = now_ts();
    let id = Uuid::new_v4();
    let short_id = id.to_string()[..8].to_string();
    let un = username.unwrap_or_default();
    let nt = notes.unwrap_or_default();
    let pj = project.unwrap_or_default();
    let tg = tags.unwrap_or_default();
    let su = source_url.unwrap_or_default();
    let tags_json = serde_json::to_value(&tg).unwrap_or_else(|_| serde_json::json!([]));
    let enc_value = obfuscate(&value);

    golish_db::repo::vault::insert_full(
        pool,
        id,
        &name,
        entry_type.as_str(),
        &enc_value,
        &un,
        &nt,
        &pj,
        &tags_json,
        &su,
        project_path.as_deref(),
    )
    .await?;

    Ok(VaultEntrySafe {
        id: short_id,
        name,
        entry_type,
        username: un,
        notes: nt,
        project: pj,
        tags: tg,
        status: "unknown".to_string(),
        source_url: su,
        last_validated_at: None,
        created_at: ts,
        updated_at: ts,
    })
}

#[tauri::command]
pub async fn vault_get_value(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    // Scoping guard (AGENTS.md I2): never reveal a secret from another project.
    let enc: String = golish_app_core::scoping::ensure_scoped_found(
        golish_db::repo::vault::get_value_scoped(pool, uid, project_path.as_deref()).await?,
    )?;
    Ok(deobfuscate(&enc)?)
}

#[tauri::command]
pub async fn vault_update(
    state: tauri::State<'_, DbState>,
    id: String,
    name: Option<String>,
    value: Option<String>,
    username: Option<String>,
    notes: Option<String>,
    project: Option<String>,
    tags: Option<Vec<String>>,
    project_path: Option<String>,
) -> Result<VaultEntrySafe, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    // Scoping guard (AGENTS.md I2): only update a vault entry in the caller's project.
    let owned = golish_db::repo::vault::exists_scoped(pool, uid, project_path.as_deref()).await?;
    golish_app_core::scoping::ensure_scoped_found(owned)?;

    // Only touch the row when at least one field was supplied (preserves the
    // prior behaviour of not bumping `updated_at` on an all-`None` call).
    if name.is_some()
        || value.is_some()
        || username.is_some()
        || notes.is_some()
        || project.is_some()
        || tags.is_some()
    {
        let enc_value = value.as_ref().map(|v| obfuscate(v));
        let tags_json = tags
            .as_ref()
            .map(|t| serde_json::to_value(t).unwrap_or_else(|_| serde_json::json!([])));
        golish_db::repo::vault::update_fields(
            pool,
            uid,
            name.as_deref(),
            enc_value.as_deref(),
            username.as_deref(),
            notes.as_deref(),
            project.as_deref(),
            tags_json.as_ref(),
        )
        .await?;
    }

    let row = golish_db::repo::vault::get_safe(pool, uid)
        .await?
        .ok_or_else(|| GolishError::NotFound("vault entry not found".to_string()))?;

    Ok(row_to_safe(row))
}

#[tauri::command]
pub async fn vault_update_status(
    state: tauri::State<'_, DbState>,
    id: String,
    status: String,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let affected =
        golish_db::repo::vault::set_status_scoped(pool, uid, &status, project_path.as_deref())
            .await?;
    golish_app_core::scoping::ensure_scoped_mutation(affected)?;
    Ok(())
}

#[tauri::command]
pub async fn vault_validate(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    // Scoping guard (AGENTS.md I2): only validate a vault entry in the caller's project.
    let (enc_value, source_url, entry_type): (String, String, String) =
        golish_app_core::scoping::ensure_scoped_found(
            golish_db::repo::vault::get_validate_fields_scoped(pool, uid, project_path.as_deref())
                .await?,
        )?;

    let value = deobfuscate(&enc_value)?;

    if source_url.is_empty() {
        return Err(GolishError::Internal(
            "No source URL to validate against".into(),
        ));
    }

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(10))
        .no_proxy()
        .build()?;

    let mut req = client.get(&source_url);
    match entry_type.as_str() {
        "token" if value.starts_with("Bearer ") => {
            req = req.header("Authorization", &value);
        }
        "api_key" => {
            req = req.header("X-API-Key", &value);
        }
        "cookie" => {
            req = req.header("Cookie", &value);
        }
        _ => {
            req = req.header("Authorization", format!("Bearer {}", value));
        }
    }

    let status = match req.send().await {
        Ok(resp) => {
            let code = resp.status().as_u16();
            if code == 401 || code == 403 {
                "expired"
            } else if (200..400).contains(&code) {
                "valid"
            } else {
                "unknown"
            }
        }
        Err(_) => "unknown",
    };

    golish_db::repo::vault::set_status(pool, uid, status).await?;

    Ok(status.to_string())
}

#[tauri::command]
pub async fn vault_delete(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let affected =
        golish_db::repo::vault::delete_scoped(pool, uid, project_path.as_deref()).await?;
    golish_app_core::scoping::ensure_scoped_mutation(affected)?;
    Ok(())
}

#[tauri::command]
pub async fn vault_resolve(
    state: tauri::State<'_, DbState>,
    reference: String,
    project_path: Option<String>,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let name = reference
        .trim_start_matches("{{vault:")
        .trim_end_matches("}}");
    let enc = golish_db::repo::vault::resolve_value(pool, name, project_path.as_deref())
        .await?
        .ok_or_else(|| format!("Vault entry '{}' not found", name))?;
    Ok(deobfuscate(&enc)?)
}
