//! Template-based prompt registry with DB override support.
//!
//! Mirrors PentAGI's `PromptType` + `PromptVariables` architecture:
//! - Default templates are embedded at compile time from `prompts/*.tera`
//! - DB rows in `prompt_templates` can override any template at runtime
//! - Rendering uses Tera's `{{ variable }}` syntax
//!
//! Usage:
//! ```ignore
//! let registry = PromptRegistry::new();
//! // Optionally load DB overrides
//! registry.load_db_overrides(&pool).await?;
//! let rendered = registry.render("pentester", &ctx)?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tera::{Context, Tera};

// Embed all default templates at compile time
static TEMPLATES: &[(&str, &str)] = &[
    ("coder", include_str!("../prompts/coder.tera")),
    ("pentester", include_str!("../prompts/pentester.tera")),
    ("analyzer", include_str!("../prompts/analyzer.tera")),
    ("explorer", include_str!("../prompts/explorer.tera")),
    ("researcher", include_str!("../prompts/researcher.tera")),
    ("memorist", include_str!("../prompts/memorist.tera")),
    ("reflector", include_str!("../prompts/reflector.tera")),
    ("adviser", include_str!("../prompts/adviser.tera")),
    ("reporter", include_str!("../prompts/reporter.tera")),
    ("planner", include_str!("../prompts/planner.tera")),
    ("worker", include_str!("../prompts/worker.tera")),
    ("generator", include_str!("../prompts/generator.tera")),
    ("refiner", include_str!("../prompts/refiner.tera")),
    ("task_reporter", include_str!("../prompts/task_reporter.tera")),
    ("task_reflector", include_str!("../prompts/task_reflector.tera")),
    ("mentor", include_str!("../prompts/mentor.tera")),
    ("summarizer", include_str!("../prompts/summarizer.tera")),
    ("toolcall_fixer", include_str!("../prompts/toolcall_fixer.tera")),
];

/// Template-based prompt registry.
///
/// Thread-safe: uses `Arc<RwLock<Tera>>` internally so clones share state.
#[derive(Clone)]
pub struct PromptRegistry {
    engine: Arc<RwLock<Tera>>,
}

impl PromptRegistry {
    /// Create a new registry loaded with embedded default templates.
    pub fn new() -> Self {
        let mut tera = Tera::default();
        for (name, content) in TEMPLATES {
            if let Err(e) = tera.add_raw_template(name, content) {
                tracing::warn!("[prompt-registry] Failed to load embedded template '{name}': {e}");
            }
        }
        Self {
            engine: Arc::new(RwLock::new(tera)),
        }
    }

    /// Load DB overrides on top of embedded defaults.
    /// Active rows in `prompt_templates` replace the corresponding embedded template.
    pub async fn load_db_overrides(&self, pool: &sqlx::PgPool) -> anyhow::Result<()> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT template_name, content FROM prompt_templates WHERE is_active = true",
        )
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            return Ok(());
        }

        let mut engine = self.engine.write().await;
        for (name, content) in &rows {
            if let Err(e) = engine.add_raw_template(name, content) {
                tracing::warn!(
                    "[prompt-registry] Failed to load DB override for '{name}': {e}"
                );
            } else {
                tracing::info!("[prompt-registry] Loaded DB override for '{name}'");
            }
        }
        Ok(())
    }

    /// Render a template by name with the given context variables.
    pub async fn render(&self, template_name: &str, ctx: &PromptContext) -> anyhow::Result<String> {
        let engine = self.engine.read().await;
        let tera_ctx = ctx.to_tera_context();
        let rendered = engine.render(template_name, &tera_ctx)?;
        Ok(rendered)
    }

    /// Render a template synchronously (for non-async contexts).
    /// Requires that no concurrent writes are happening.
    pub fn render_blocking(&self, template_name: &str, ctx: &PromptContext) -> anyhow::Result<String> {
        let engine = self.engine.blocking_read();
        let tera_ctx = ctx.to_tera_context();
        let rendered = engine.render(template_name, &tera_ctx)?;
        Ok(rendered)
    }

    /// Check if a template exists in the registry.
    pub async fn has_template(&self, name: &str) -> bool {
        let engine = self.engine.read().await;
        let exists = engine.get_template_names().any(|n| n == name);
        exists
    }

    /// List all available template names.
    pub async fn list_templates(&self) -> Vec<String> {
        let engine = self.engine.read().await;
        let names = engine.get_template_names().map(String::from).collect();
        names
    }

    /// Override a single template at runtime (e.g., from API/settings).
    pub async fn set_template(&self, name: &str, content: &str) -> anyhow::Result<()> {
        let mut engine = self.engine.write().await;
        engine.add_raw_template(name, content)?;
        Ok(())
    }

    /// Get the raw template content (for display in settings UI).
    pub async fn get_raw(&self, name: &str) -> Option<String> {
        let engine = self.engine.read().await;
        if engine.get_template(name).is_ok() {
            TEMPLATES.iter().find(|(n, _)| *n == name).map(|(_, c)| c.to_string())
        } else {
            None
        }
    }
}

impl Default for PromptRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Variables available for template rendering.
///
/// Mirrors PentAGI's `PromptVariables` struct. All fields are optional;
/// templates use `{{ variable | default(value="") }}` for missing values.
#[derive(Default, Clone)]
pub struct PromptContext {
    vars: HashMap<String, String>,
}

impl PromptContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a variable. Chainable.
    pub fn set(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(key.into(), value.into());
        self
    }

    /// Set a variable by reference.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    fn to_tera_context(&self) -> Context {
        let mut ctx = Context::new();
        for (k, v) in &self.vars {
            ctx.insert(k.as_str(), v);
        }
        ctx
    }
}

/// Convenience: build a `PromptContext` from key-value pairs.
///
/// ```ignore
/// let ctx = prompt_ctx![
///     "execution_context" => execution_summary,
///     "remaining_subtasks" => subtasks_json,
/// ];
/// ```
#[macro_export]
macro_rules! prompt_ctx {
    ($($key:expr => $val:expr),* $(,)?) => {{
        let mut ctx = $crate::prompt_registry::PromptContext::new();
        $(ctx.insert($key, $val);)*
        ctx
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_embedded_templates_load() {
        let registry = PromptRegistry::new();
        let templates = registry.list_templates().await;
        assert!(templates.contains(&"pentester".to_string()));
        assert!(templates.contains(&"coder".to_string()));
        assert!(templates.contains(&"generator".to_string()));
        assert!(templates.len() >= 15);
    }

    #[tokio::test]
    async fn test_render_static_template() {
        let registry = PromptRegistry::new();
        let ctx = PromptContext::new();
        let rendered = registry.render("pentester", &ctx).await.unwrap();
        assert!(rendered.contains("penetration testing specialist"));
    }

    #[tokio::test]
    async fn test_render_with_variables() {
        let registry = PromptRegistry::new();
        let ctx = PromptContext::new()
            .set("execution_context", "Port 80 is open running nginx 1.18")
            .set("remaining_subtasks", "[{\"title\":\"test\"}]");
        let rendered = registry.render("refiner", &ctx).await.unwrap();
        assert!(rendered.contains("Port 80 is open running nginx 1.18"));
        assert!(rendered.contains("[{\"title\":\"test\"}]"));
    }

    #[tokio::test]
    async fn test_override_template() {
        let registry = PromptRegistry::new();
        registry
            .set_template("pentester", "You are a CUSTOM pentester. {{ custom_var }}")
            .await
            .unwrap();
        let ctx = PromptContext::new().set("custom_var", "hello");
        let rendered = registry.render("pentester", &ctx).await.unwrap();
        assert!(rendered.contains("CUSTOM pentester"));
        assert!(rendered.contains("hello"));
    }
}
