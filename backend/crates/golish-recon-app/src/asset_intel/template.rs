//! Pure template rendering + secret-ref extraction for asset-intel runtimes.
//!
//! Renders CLI skill args / HTTP request templates (substituting `{{org}}`,
//! `{{config.*}}`, `{{secret:*}}`) and scans templates for the secret keys they
//! reference. No DB / IO: the caller resolves secrets and runs the request.
//! Re-exported from the parent module so existing call sites keep using the
//! bare function names.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use super::AssetIntelHydrateConfig;

/// Render one host-owned, already-encoded semantic binding. The semantic path
/// has a single closed placeholder and refuses any leftover template syntax;
/// legacy hydrate templates continue to use the functions below unchanged.
#[cfg(test)]
pub(crate) fn render_semantic_binding(
    template: &str,
    placeholder: &str,
    encoded_literal: &str,
) -> Result<String, &'static str> {
    if placeholder.is_empty()
        || placeholder.contains(['{', '}'])
        || encoded_literal.trim().is_empty()
    {
        return Err("INTEL_SEMANTIC_BINDING_INVALID");
    }
    let marker = format!("{{{{{placeholder}}}}}");
    if template.matches(&marker).count() != 1 {
        return Err("INTEL_SEMANTIC_TEMPLATE_NOT_CLOSED");
    }
    let rendered = template.replacen(&marker, encoded_literal, 1);
    if rendered.contains("{{") || rendered.contains("}}") {
        return Err("INTEL_SEMANTIC_TEMPLATE_UNKNOWN_BINDING");
    }
    Ok(rendered)
}

pub(crate) fn render_asset_intel_skill_args(
    skill_args: &str,
    company_name: &str,
    out_dir: &Path,
    config: &AssetIntelHydrateConfig,
    arg_bindings: &std::collections::HashMap<String, String>,
) -> String {
    fn render_template(
        template: &str,
        company_name: &str,
        out_dir: &Path,
        config: &AssetIntelHydrateConfig,
    ) -> String {
        template
            .replace("{{org}}", company_name)
            .replace("{{company_name}}", company_name)
            // b1 (design 2026-06-24): domain-keyed survey value (empty in the
            // legacy company-name survey).
            .replace("{{domain}}", config.domain.as_deref().unwrap_or_default())
            .replace("{{out_dir}}", &out_dir.to_string_lossy())
            .replace(
                "{{config.min_ownership_percent}}",
                config.min_ownership_percent.as_deref().unwrap_or_default(),
            )
            .replace(
                "{{config.depth}}",
                config.depth.as_deref().unwrap_or_default(),
            )
            .replace(
                "{{config.include_branches}}",
                if config.include_branches.unwrap_or(false) {
                    "true"
                } else {
                    "false"
                },
            )
    }

    let mut rendered = render_template(skill_args, company_name, out_dir, config);
    let mut binding_keys: Vec<&String> = arg_bindings.keys().collect();
    binding_keys.sort_by(|a, b| {
        fn order(key: &str) -> usize {
            match key {
                "min_ownership_percent" => 0,
                "depth" => 1,
                "include_branches" => 2,
                _ => 100,
            }
        }
        order(a).cmp(&order(b)).then_with(|| a.cmp(b))
    });
    for key in binding_keys {
        let enabled = match key.as_str() {
            "min_ownership_percent" => config
                .min_ownership_percent
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            "depth" => config
                .depth
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            "include_branches" => config.include_branches.unwrap_or(false),
            _ => false,
        };
        if enabled {
            let binding = render_template(&arg_bindings[key], company_name, out_dir, config);
            if !binding.trim().is_empty() {
                rendered.push(' ');
                rendered.push_str(binding.trim());
            }
        }
    }
    rendered
}

pub(crate) fn split_command_args(rendered: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for ch in rendered.chars() {
        match ch {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn extract_secret_refs_from_str(value: &str, out: &mut HashSet<String>) {
    let mut rest = value;
    while let Some(start) = rest.find("{{secret:") {
        let after_start = &rest[start + "{{secret:".len()..];
        let Some(end) = after_start.find("}}") else {
            break;
        };
        let key = after_start[..end].trim();
        if !key.is_empty() {
            out.insert(key.to_string());
        }
        rest = &after_start[end + "}}".len()..];
    }
}

fn extract_secret_refs_from_json(value: &Value, out: &mut HashSet<String>) {
    match value {
        Value::String(text) => extract_secret_refs_from_str(text, out),
        Value::Array(items) => {
            for item in items {
                extract_secret_refs_from_json(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                extract_secret_refs_from_json(item, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_http_secret_refs(
    requests: &[golish_pentest::models::AssetIntelHttpRequest],
) -> HashSet<String> {
    let mut refs = HashSet::new();
    for request in requests {
        extract_secret_refs_from_str(&request.url, &mut refs);
        for value in request.headers.values() {
            extract_secret_refs_from_str(value, &mut refs);
        }
        for value in request.form.values() {
            extract_secret_refs_from_str(value, &mut refs);
        }
        extract_secret_refs_from_json(&request.json, &mut refs);
    }
    refs
}

pub(crate) fn render_http_template(
    template: &str,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
    secrets: &HashMap<String, String>,
) -> String {
    let semantic_value = config
        .semantic_pivot
        .as_ref()
        .map(|pivot| pivot.value.as_str());
    let company_binding = semantic_value.unwrap_or(company_name);
    let domain_binding = semantic_value
        .or(config.domain.as_deref())
        .unwrap_or_default();
    let mut rendered = template
        .replace("{{org}}", company_binding)
        .replace("{{company_name}}", company_binding)
        // b1 (design 2026-06-24): domain-keyed survey value (empty in the legacy
        // company-name survey).
        .replace("{{domain}}", domain_binding)
        .replace(
            "{{config.min_ownership_percent}}",
            config.min_ownership_percent.as_deref().unwrap_or_default(),
        )
        .replace(
            "{{config.depth}}",
            config.depth.as_deref().unwrap_or_default(),
        );
    for (key, value) in secrets {
        rendered = rendered.replace(&format!("{{{{secret:{key}}}}}"), value);
    }
    rendered
}

pub(crate) fn render_http_url_template(
    template: &str,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
    secrets: &HashMap<String, String>,
    literal_encoder: Option<&str>,
) -> String {
    if literal_encoder == Some("url_query_component.v1") {
        if let Some(pivot) = config.semantic_pivot.as_ref() {
            let encoded =
                url::form_urlencoded::byte_serialize(pivot.value.as_bytes()).collect::<String>();
            let template = template
                .replace("{{org}}", &encoded)
                .replace("{{company_name}}", &encoded)
                .replace("{{domain}}", &encoded);
            return render_http_template(&template, company_name, config, secrets);
        }
    }
    render_http_template(template, company_name, config, secrets)
}

pub(crate) fn render_http_json_value(
    value: &Value,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
    secrets: &HashMap<String, String>,
) -> Value {
    match value {
        Value::String(text) => {
            Value::String(render_http_template(text, company_name, config, secrets))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_http_json_value(item, company_name, config, secrets))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    (
                        key.clone(),
                        render_http_json_value(item, company_name, config, secrets),
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Render the lookup skill template. Mirrors `render_asset_intel_skill_args`
/// but without the optional `arg_bindings` for ownership / depth / branches
/// — lookup is intentionally lightweight.
pub(crate) fn render_lookup_skill_args(skill_args: &str, keyword: &str, out_dir: &Path) -> String {
    skill_args
        .replace("{{org}}", keyword)
        .replace("{{keyword}}", keyword)
        .replace("{{company_name}}", keyword)
        .replace("{{out_dir}}", &out_dir.to_string_lossy())
}

#[cfg(test)]
mod semantic_template_tests {
    use super::{render_http_url_template, render_semantic_binding};
    use crate::asset_intel::AssetIntelHydrateConfig;
    use golish_pentest_domain::models::{AssetIntelPivot, AssetIntelPivotKind};
    use std::collections::HashMap;

    #[test]
    fn semantic_template_requires_one_closed_host_binding() {
        assert_eq!(
            render_semantic_binding("query={{semantic_query}}", "semantic_query", "x=\"y\"")
                .unwrap(),
            "query=x=\"y\""
        );
        assert_eq!(
            render_semantic_binding(
                "query={{semantic_query}}&leak={{model_dsl}}",
                "semantic_query",
                "x=\"y\""
            ),
            Err("INTEL_SEMANTIC_TEMPLATE_UNKNOWN_BINDING")
        );
    }

    #[test]
    fn semantic_public_url_uses_one_encoded_query_component() {
        let config = AssetIntelHydrateConfig {
            semantic_pivot: Some(
                AssetIntelPivot::parse(AssetIntelPivotKind::CompanyName, "杭州 默安科技").unwrap(),
            ),
            ..Default::default()
        };
        assert_eq!(
            render_http_url_template(
                "https://api.github.com/search/repositories?q={{company_name}}&per_page=50",
                "ignored",
                &config,
                &HashMap::new(),
                Some("url_query_component.v1")
            ),
            "https://api.github.com/search/repositories?q=%E6%9D%AD%E5%B7%9E+%E9%BB%98%E5%AE%89%E7%A7%91%E6%8A%80&per_page=50"
        );
    }
}
