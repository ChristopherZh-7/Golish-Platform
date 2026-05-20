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

use golish_intel_providers::shared::KeyStore;
use golish_intel_providers::{
    error::IntelError, fofa::FofaProvider, hunter::HunterProvider, quake::QuakeProvider,
    shodan::ShodanProvider, zone::ZoneProvider, ConnectionStatus, IntelProvider, ProviderMeta,
    ProviderRecord, QueryType,
};
use golish_pentest::output_store::OutputStore;

use crate::error::GolishError;
use crate::state::DbState;

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
    for record in &records {
        match pg_store
            .store_organization_update(&record.fields, project_path.as_deref())
            .await
        {
            Ok(()) => persisted += 1,
            Err(e) => errors.push(e.to_string()),
        }
    }

    Ok(IntelQueryResult {
        provider: provider_id,
        query_type,
        records,
        persisted,
        errors,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct IntelQueryResult {
    pub provider: String,
    pub query_type: String,
    pub records: Vec<ProviderRecord>,
    /// How many records were successfully persisted to `organizations`.
    pub persisted: usize,
    /// Per-record persistence errors (non-fatal — surfaced so UI can warn).
    pub errors: Vec<String>,
}
