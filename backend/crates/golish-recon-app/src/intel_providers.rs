//! ASM intel-provider IPC facade.
//!
//! Bridges the `golish-intel-providers` crate's `IntelProvider` trait into
//! Tauri-callable commands. Provides:
//!
//! - [`intel_list_providers`] — UI lists all known providers + their meta
//! - [`intel_test_connection`] — UI button verifies a configured API key
//! - [`intel_query_provider`] — UI / agent triggers a query, results go
//!   straight into `organizations` via `output_store::store_organization_update`
//!
//! API keys are fetched from the existing `vault_entries` table
//! (entry_type=`api_key`, name=`<provider_id>`, tags=`["intel-provider"]`).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use golish_intel_providers::shared::KeyStore;
use golish_intel_providers::{
    error::IntelError, fofa::FofaProvider, hunter::HunterProvider, quake::QuakeProvider,
    shodan::ShodanProvider, zone::ZoneProvider, ConnectionStatus, IntelProvider, ProviderMeta,
    ProviderRecord, QueryType,
};
use golish_pentest::output_store::OutputStore;

use golish_app_core::DbState;
use golish_app_core::GolishError;

/// `KeyStore` impl that reads from the `vault_entries` table.
///
/// Lookup convention: vault entry **name** matches the provider id
/// (e.g. `"0.zone"`), entry_type is `api_key`. The newest matching row
/// wins (ORDER BY created_at DESC LIMIT 1).
struct PgVaultKeyStore {
    pool: PgPool,
}

#[async_trait]
impl KeyStore for PgVaultKeyStore {
    async fn get_key(
        &self,
        provider_id: &str,
    ) -> golish_intel_providers::IntelResult<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT value FROM vault_entries \
             WHERE name = $1 AND entry_type = 'api_key' \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(provider_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| IntelError::Other(format!("vault read failed: {e}")))?;

        // NOTE: golish-core's vault stores `value` as base64-obfuscated bytes
        // via `golish_core::vault::deobfuscate`. We call it here so the
        // returned string is the cleartext API key.
        match row {
            None => Ok(None),
            Some((obf,)) => match golish_core::vault::deobfuscate(&obf) {
                Ok(plain) => Ok(Some(plain)),
                Err(e) => Err(IntelError::Other(format!("deobfuscate failed: {e}"))),
            },
        }
    }
}

fn provider_registry() -> HashMap<String, Arc<dyn IntelProvider>> {
    let mut m: HashMap<String, Arc<dyn IntelProvider>> = HashMap::new();
    m.insert("0.zone".into(), Arc::new(ZoneProvider::default()));
    m.insert("fofa".into(), Arc::new(FofaProvider::default()));
    m.insert("quake".into(), Arc::new(QuakeProvider::default()));
    m.insert("hunter".into(), Arc::new(HunterProvider::default()));
    m.insert("shodan".into(), Arc::new(ShodanProvider::default()));
    m
}

fn parse_query_type(s: &str) -> Result<QueryType, GolishError> {
    match s {
        "site" => Ok(QueryType::Site),
        "domain" => Ok(QueryType::Domain),
        "email" => Ok(QueryType::Email),
        "apk" => Ok(QueryType::Apk),
        "sensitive" => Ok(QueryType::Sensitive),
        "code" => Ok(QueryType::Code),
        "member" => Ok(QueryType::Member),
        "org" => Ok(QueryType::Org),
        "branch" => Ok(QueryType::Branch),
        "darknet" => Ok(QueryType::Darknet),
        "cert" => Ok(QueryType::Cert),
        "asn" => Ok(QueryType::Asn),
        "cidr" => Ok(QueryType::Cidr),
        other => Err(GolishError::Validation(format!(
            "unknown query_type: {other}"
        ))),
    }
}

/// List all registered ASM intel providers and their static metadata.
///
/// Settings UI calls this on mount to render one card per provider.
#[tauri::command]
pub async fn intel_list_providers() -> Result<Vec<ProviderMeta>, GolishError> {
    let reg = provider_registry();
    let mut metas: Vec<ProviderMeta> = reg.values().map(|p| p.meta()).collect();
    // Stable ordering by id so the UI doesn't reshuffle on every call.
    metas.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(metas)
}

/// Test whether the configured API key for `provider_id` is valid.
///
/// Settings UI calls this from the "Test Connection" button on each
/// provider card.
#[tauri::command]
pub async fn intel_test_connection(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<ConnectionStatus, GolishError> {
    let pool = state.pool_ready().await?;
    let reg = provider_registry();
    let provider = reg
        .get(&provider_id)
        .ok_or_else(|| GolishError::NotFound(format!("intel provider '{provider_id}'")))?;

    let store = PgVaultKeyStore { pool: pool.clone() };
    let key = store
        .get_key(&provider_id)
        .await
        .map_err(|e| GolishError::Internal(e.to_string()))?
        .unwrap_or_default();

    provider
        .test_connection(&key)
        .await
        .map_err(|e| GolishError::Internal(e.to_string()))
}

/// Run an ASM intel query and persist results into `organizations`.
///
/// Returns the parsed `ProviderRecord`s so the UI can also display them.
/// Results are written into `organizations` via
/// `output_store::store_organization_update` before this returns.
#[tauri::command]
pub async fn intel_query_provider(
    state: tauri::State<'_, DbState>,
    provider_id: String,
    query_type: String,
    query: String,
    project_path: Option<String>,
) -> Result<IntelQueryResult, GolishError> {
    let pool = state.pool_ready().await?;
    let reg = provider_registry();
    let provider = reg
        .get(&provider_id)
        .ok_or_else(|| GolishError::NotFound(format!("intel provider '{provider_id}'")))?;

    let qt = parse_query_type(&query_type)?;
    let store = PgVaultKeyStore { pool: pool.clone() };
    let key = store
        .get_key(&provider_id)
        .await
        .map_err(|e| GolishError::Internal(e.to_string()))?
        .ok_or_else(|| {
            GolishError::Config(format!(
                "no API key configured for provider '{provider_id}' (Settings → Intel Providers)"
            ))
        })?;

    let records: Vec<ProviderRecord> = provider
        .query(qt, &query, &key)
        .await
        .map_err(|e| GolishError::Internal(e.to_string()))?;

    let pg_store = golish_pentest::output_store::PgPentestStore::new(pool);
    let mut persisted: usize = 0;
    let mut errors: Vec<String> = Vec::new();
    let mut targets_written: usize = 0;
    for record in &records {
        // Enrich with provider + query_type meta keys so the writer can
        // bucket leftover fields into organizations.intel.records[]
        // (see organizations.rs docstring for the meta-key convention).
        let mut enriched: HashMap<String, String> = record.fields.clone();
        enriched.insert("_provider".into(), record.provider.clone());
        enriched.insert("_query_type".into(), record.query_type.as_str().into());

        // Step 1 (always): write into organizations.* + intel.records[] catch-all.
        match pg_store
            .store_organization_update(&enriched, project_path.as_deref())
            .await
        {
            Ok(()) => persisted += 1,
            Err(e) => errors.push(format!("organization_update: {e}")),
        }

        // Step 2 (conditional): when the record carries asset-level fields
        // (ip / port / title / webserver / ...), also persist them into the
        // `targets` table so the Asset / Recon UI surfaces them instead of
        // leaving everything buried under organizations.intel.records[].
        if let Some(target_fields) = build_target_fields_from_intel(&enriched) {
            let host_val = target_fields
                .get("host")
                .cloned()
                .or_else(|| target_fields.get("ip").cloned())
                .or_else(|| target_fields.get("url").cloned())
                .unwrap_or_default();
            if host_val.trim().is_empty() {
                continue;
            }
            let target_id = match pg_store
                .find_or_create_target(&host_val, project_path.as_deref())
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    errors.push(format!("target_find: {e}"));
                    continue;
                }
            };
            let tool_name = format!("intel/{}/{}", record.provider, record.query_type.as_str());
            if let Err(e) = pg_store
                .store_target_update_recon(&target_fields, project_path.as_deref(), &tool_name)
                .await
            {
                errors.push(format!("target_update_recon: {e}"));
            } else {
                targets_written += 1;
            }
            if let Some(org_name) = enriched.get("organization_name").map(|s| s.trim()) {
                if !org_name.is_empty() {
                    if let Err(e) = link_target_to_organization(
                        pool,
                        target_id,
                        org_name,
                        project_path.as_deref(),
                    )
                    .await
                    {
                        errors.push(format!("target_link_org: {e}"));
                    }
                }
            }
        }
    }
    tracing::info!(
        "[intel_query_provider] provider={} qt={} records={} orgs_persisted={} targets_written={} errors={}",
        provider_id,
        query_type,
        records.len(),
        persisted,
        targets_written,
        errors.len(),
    );

    Ok(IntelQueryResult {
        provider: provider_id,
        query_type,
        records,
        persisted,
        targets_written,
        errors,
    })
}

/// Derive a `fields` map suitable for `store_target_update_recon` from an
/// intel-provider record. Returns `None` when the record lacks any
/// host-identifying key (e.g. a 0.zone `member` or `email` record that
/// has no asset surface).
///
/// Side effects on the returned map:
/// - If `host` is absent but `domain` is present, copy `domain → host` so
///   the writer picks the most stable identifier (avoids spawning a `1.2.3.4`
///   IP target alongside a `api.example.com` domain target for the same asset).
fn build_target_fields_from_intel(
    fields: &HashMap<String, String>,
) -> Option<HashMap<String, String>> {
    let has_asset = ["host", "ip", "url", "domain"].iter().any(|k| {
        fields
            .get(*k)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    });
    if !has_asset {
        return None;
    }
    let mut out = fields.clone();
    let host_blank = out.get("host").map(|s| s.trim().is_empty()).unwrap_or(true);
    if host_blank {
        if let Some(domain) = out.get("domain").cloned() {
            if !domain.trim().is_empty() {
                out.insert("host".into(), domain);
            }
        }
    }
    Some(out)
}

/// Idempotently attach a freshly-created/updated target to the root
/// organization named `organization_name` (matching the find-or-create
/// rule used by `store_organization_update`).
///
/// Best-effort: returns Ok even when the organization row hasn't been
/// created yet — in that case `targets.organization_id` stays NULL and
/// will be populated on the next intel write.
async fn link_target_to_organization(
    pool: &PgPool,
    target_id: Uuid,
    organization_name: &str,
    project_path: Option<&str>,
) -> anyhow::Result<()> {
    let pp = project_path.unwrap_or("");
    let org_id =
        golish_db::repo::organizations::find_root_id_by_name(pool, pp, organization_name).await?;
    let Some(oid) = org_id else { return Ok(()) };

    sqlx::query(
        r#"UPDATE targets
           SET organization_id = $1
           WHERE id = $2 AND organization_id IS NULL"#,
    )
    .bind(oid)
    .bind(target_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct IntelQueryResult {
    pub provider: String,
    pub query_type: String,
    pub records: Vec<ProviderRecord>,
    /// How many records were successfully persisted to `organizations`.
    pub persisted: usize,
    /// How many records also produced an asset row in the `targets` table
    /// (records lacking any host/ip/url/domain key are skipped here).
    pub targets_written: usize,
    /// Per-record persistence errors (non-fatal — surfaced so UI can warn).
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn build_target_fields_returns_none_when_no_asset_key() {
        // 0.zone "member" / "email" / "code" records carry no host/ip/url/domain
        let fields = h(&[
            ("organization_name", "Acme"),
            ("contact_name", "Alice"),
            ("contact_source", "linkedin"),
        ]);
        assert!(build_target_fields_from_intel(&fields).is_none());
    }

    #[test]
    fn build_target_fields_copies_domain_to_host_when_host_missing() {
        // Shodan / Quake commonly emit `ip + domain` without an explicit `host`.
        // We want the resulting target keyed on the most stable identifier (the
        // domain), so the writer doesn't spawn separate IP-typed and
        // domain-typed targets for the same asset.
        let fields = h(&[
            ("ip", "1.2.3.4"),
            ("domain", "api.example.com"),
            ("port", "443"),
            ("title", "Hello"),
        ]);
        let out = build_target_fields_from_intel(&fields).expect("has ip");
        assert_eq!(out.get("host").map(String::as_str), Some("api.example.com"));
        assert_eq!(out.get("ip").map(String::as_str), Some("1.2.3.4"));
        assert_eq!(out.get("port").map(String::as_str), Some("443"));
    }

    #[test]
    fn build_target_fields_keeps_explicit_host() {
        // FOFA emits `host` as a full URL; let the downstream writer normalize.
        let fields = h(&[
            ("host", "https://example.com"),
            ("ip", "93.184.216.34"),
            ("domain", "example.com"),
        ]);
        let out = build_target_fields_from_intel(&fields).expect("has host");
        assert_eq!(
            out.get("host").map(String::as_str),
            Some("https://example.com")
        );
    }

    #[test]
    fn build_target_fields_accepts_domain_only() {
        // 0.zone `domain` query → only the `domain` key is set.
        let fields = h(&[("domain", "sub.example.com"), ("organization_name", "Acme")]);
        let out = build_target_fields_from_intel(&fields).expect("has domain");
        assert_eq!(out.get("host").map(String::as_str), Some("sub.example.com"));
        assert_eq!(
            out.get("domain").map(String::as_str),
            Some("sub.example.com")
        );
    }

    #[test]
    fn build_target_fields_treats_blank_values_as_missing() {
        let fields = h(&[("ip", "   "), ("host", "")]);
        assert!(build_target_fields_from_intel(&fields).is_none());
    }

    #[test]
    fn build_target_fields_does_not_overwrite_existing_host_with_domain() {
        // host is set (even if it's a URL); leave it alone.
        let fields = h(&[("host", "https://a.com"), ("domain", "b.com")]);
        let out = build_target_fields_from_intel(&fields).expect("has host");
        assert_eq!(out.get("host").map(String::as_str), Some("https://a.com"));
    }
}
