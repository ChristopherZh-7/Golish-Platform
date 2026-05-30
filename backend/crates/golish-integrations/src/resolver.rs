//! Schema collection.
//!
//! The resolver feeds the IPC facade ([`crate::traits::SchemaResolver`]).
//! It pulls integration descriptors from two sources:
//!
//! 1. **Tool config files** in `resources/toolsconfig/*.json` whose
//!    `tool.integration` field is set.
//! 2. **In-code intel providers** — every implementation of
//!    `IntelProvider` declares its
//!    [`crate::schema::IntegrationSchema`] inline; the host crate
//!    snapshots them and hands the snapshot to [`DefaultSchemaResolver`]
//!    at construction time (so this crate stays free of a direct
//!    `golish-intel-providers` dependency).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::fs;

use crate::error::{IntegrationError, IntegrationResult};
use crate::schema::{CaptureRule, IntegrationGroup, IntegrationSchema};
use crate::traits::{ResolvedIntegration, SchemaResolver};

/// Validate a capture recipe makes sense in the context of its
/// parent group:
///
/// 1. `login_url` parses and uses `http` / `https` scheme (rejects
///    `javascript:`, `file://`, etc.).
/// 2. Every rule's `target_field` references an existing
///    [`crate::schema::Field::key`] in the parent group.
///
/// (Timeout clamp happens engine-side; this function only enforces
/// invariants the schema parser doesn't.)
fn validate_capture(group: &IntegrationGroup) -> IntegrationResult<()> {
    let Some(recipe) = group.capture.as_ref() else {
        return Ok(());
    };

    // 1. login_url scheme whitelist
    let parsed = url::Url::parse(&recipe.login_url)
        .map_err(|e| IntegrationError::CaptureInvalidUrl(format!("{}: {e}", recipe.login_url)))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(IntegrationError::CaptureInvalidUrl(format!(
            "{} (scheme must be http or https, got {})",
            recipe.login_url,
            parsed.scheme()
        )));
    }

    // 2. target_field cross-reference
    let known: std::collections::HashSet<&str> =
        group.fields.iter().map(|f| f.key.as_str()).collect();
    for (idx, rule) in recipe.rules.iter().enumerate() {
        let tf = match rule {
            CaptureRule::Cookie { target_field, .. }
            | CaptureRule::CookieJoined { target_field, .. }
            | CaptureRule::LocalStorage { target_field, .. }
            | CaptureRule::SessionStorage { target_field, .. }
            | CaptureRule::PageContent { target_field, .. }
            | CaptureRule::UrlQuery { target_field, .. }
            | CaptureRule::RequestHeader { target_field, .. } => target_field.as_str(),
        };
        if !known.contains(tf) {
            return Err(IntegrationError::CaptureInvalidTargetField {
                rule_index: idx,
                field: tf.to_string(),
            });
        }
    }
    Ok(())
}

/// Validate every group's capture recipe in a schema. Returns the
/// first error encountered (callers should fix one issue at a time
/// rather than collecting batches).
fn validate_schema_captures(schema: &IntegrationSchema) -> IntegrationResult<()> {
    for group in schema.groups.iter() {
        validate_capture(group)?;
    }
    Ok(())
}

/// Default resolver — scans a directory of tool JSON files, plus
/// merges in any in-code schemas injected at construction time.
///
/// The result is cached behind a `tokio::sync::OnceCell` so repeated
/// IPC calls don't re-walk the filesystem.
pub struct DefaultSchemaResolver {
    /// Directory containing `*.json` tool configs. Skipped if `None`.
    toolsconfig_dir: Option<PathBuf>,
    /// Schemas declared in code (e.g. by intel providers).
    in_code: Vec<ResolvedIntegration>,
    cache: tokio::sync::OnceCell<Vec<ResolvedIntegration>>,
}

impl DefaultSchemaResolver {
    pub fn new<P: AsRef<Path>>(
        toolsconfig_dir: Option<P>,
        in_code: Vec<ResolvedIntegration>,
    ) -> Self {
        Self {
            toolsconfig_dir: toolsconfig_dir.map(|p| p.as_ref().to_path_buf()),
            in_code,
            cache: tokio::sync::OnceCell::new(),
        }
    }

    /// Build (or return cached) full integration list.
    async fn collect(&self) -> IntegrationResult<&Vec<ResolvedIntegration>> {
        self.cache
            .get_or_try_init(|| async {
                let mut by_tool: HashMap<String, ResolvedIntegration> = HashMap::new();

                // 1. Walk the toolsconfig dir.
                if let Some(dir) = &self.toolsconfig_dir {
                    if let Ok(mut entries) = fs::read_dir(dir).await {
                        while let Some(ent) = entries.next_entry().await.transpose() {
                            let ent = ent.map_err(IntegrationError::Io)?;
                            let path = ent.path();
                            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                                continue;
                            }
                            if let Some(resolved) = load_toolsconfig_file(&path).await? {
                                by_tool.insert(resolved.tool_id.clone(), resolved);
                            }
                        }
                    }
                }

                // 2. In-code schemas override toolsconfig (intel providers
                //    are authoritative for their own meta).
                for it in &self.in_code {
                    by_tool.insert(it.tool_id.clone(), it.clone());
                }

                // 3. Validate capture recipes across all merged schemas.
                //    A single bad recipe fails fast — capture is opt-in
                //    so an invalid one almost certainly means a typo
                //    rather than a desired-but-impossible config.
                for it in by_tool.values() {
                    validate_schema_captures(&it.schema).map_err(|e| match e {
                        IntegrationError::CaptureInvalidUrl(msg) => {
                            IntegrationError::CaptureInvalidUrl(format!("{} ({})", msg, it.tool_id))
                        }
                        IntegrationError::CaptureInvalidTargetField { rule_index, field } => {
                            IntegrationError::Validation(format!(
                                "tool {} integration has invalid capture target_field at rule #{rule_index}: {field}",
                                it.tool_id
                            ))
                        }
                        other => other,
                    })?;
                }

                // 4. Stable ordering by tool_id so the UI doesn't reshuffle.
                let mut out: Vec<ResolvedIntegration> = by_tool.into_values().collect();
                out.sort_by(|a, b| a.tool_id.cmp(&b.tool_id));
                Ok::<_, IntegrationError>(out)
            })
            .await
    }
}

/// Read one tool JSON; return `Some(ResolvedIntegration)` if it
/// declares an `integration` field that parses into our schema, else
/// `None`. JSON-parse failure → `IntegrationError::Validation`.
async fn load_toolsconfig_file(path: &Path) -> IntegrationResult<Option<ResolvedIntegration>> {
    let text = fs::read_to_string(path).await?;
    // The wire format wraps the config in `{ "tool": { ... } }` to
    // match the existing toolsconfig schema; we only need 2 keys.
    let outer: serde_json::Value = serde_json::from_str(&text).map_err(IntegrationError::from)?;
    let tool = match outer.get("tool") {
        Some(t) => t,
        None => return Ok(None),
    };
    let tool_id = match tool.get("id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return Ok(None),
    };
    let Some(integration_raw) = tool.get("integration").cloned() else {
        return Ok(None);
    };
    if integration_raw.is_null() {
        return Ok(None);
    }
    let schema: IntegrationSchema = serde_json::from_value(integration_raw).map_err(|e| {
        IntegrationError::Validation(format!(
            "toolsconfig {}: invalid `integration` schema: {e}",
            path.display()
        ))
    })?;
    Ok(Some(ResolvedIntegration { tool_id, schema }))
}

#[async_trait]
impl SchemaResolver for DefaultSchemaResolver {
    async fn list(&self) -> IntegrationResult<Vec<ResolvedIntegration>> {
        let list = self.collect().await?;
        Ok(list.clone())
    }

    async fn get(&self, tool_id: &str) -> IntegrationResult<ResolvedIntegration> {
        let list = self.collect().await?;
        list.iter()
            .find(|r| r.tool_id == tool_id)
            .cloned()
            .ok_or_else(|| IntegrationError::SchemaNotFound(tool_id.to_string()))
    }
}

#[cfg(test)]
#[path = "resolver_tests.rs"]
mod tests;
