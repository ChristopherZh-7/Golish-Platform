//! Pure profile-patch building + merge helpers.
//!
//! Folds normalized `ProfileFieldEntry` values into a `ProfilePatch`, and
//! merges a patch against an organization's existing row (union semantics,
//! case-insensitive dedupe). No DB / IO: the caller persists the resulting
//! patch. Re-exported from the parent module so existing call sites keep using
//! the bare function names.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use golish_app_core::GolishError;

use super::ProfileFieldEntry;

fn merge_string_vec(existing: &[String], incoming: &mut Vec<String>) {
    let mut out = existing.to_vec();
    for item in std::mem::take(incoming) {
        let key = item.trim().to_lowercase();
        if key.is_empty() || out.iter().any(|value| value.trim().to_lowercase() == key) {
            continue;
        }
        out.push(item);
    }
    *incoming = out;
}

fn merge_json_array(existing: &Value, incoming: &mut Option<Value>) {
    let Some(Value::Array(next)) = incoming else {
        return;
    };
    let mut out = existing.as_array().cloned().unwrap_or_default();
    for item in std::mem::take(next) {
        let key = display_json_atom(&item).trim().to_lowercase();
        if key.is_empty()
            || out
                .iter()
                .any(|value| display_json_atom(value).trim().to_lowercase() == key)
        {
            continue;
        }
        out.push(item);
    }
    *incoming = Some(Value::Array(out));
}

fn merge_contacts_object(existing: &Value, incoming: &mut Option<Value>) {
    let Some(Value::Object(next)) = incoming else {
        return;
    };
    let mut out = existing.as_object().cloned().unwrap_or_default();
    for (channel, value) in std::mem::take(next) {
        let target = out
            .entry(channel)
            .or_insert_with(|| Value::Array(Vec::new()));
        if !target.is_array() {
            *target = Value::Array(Vec::new());
        }
        let Some(target_array) = target.as_array_mut() else {
            continue;
        };
        for item in value.as_array().cloned().unwrap_or_else(|| vec![value]) {
            let key = display_json_atom(&item).trim().to_lowercase();
            if key.is_empty()
                || target_array
                    .iter()
                    .any(|existing| display_json_atom(existing).trim().to_lowercase() == key)
            {
                continue;
            }
            target_array.push(item);
        }
    }
    *incoming = Some(Value::Object(out));
}

fn display_json_atom(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Object(map) => map
            .get("domain")
            .or_else(|| map.get("name"))
            .or_else(|| map.get("value"))
            .or_else(|| map.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn merge_profile_patch_with_existing(
    org: &golish_db::models::Organization,
    patch: &mut golish_db::repo::organizations::ProfilePatch,
) {
    if let Some(aliases) = patch.aliases.as_mut() {
        merge_string_vec(&org.aliases, aliases);
    }
    merge_json_array(&org.domains, &mut patch.domains);
    merge_json_array(&org.ip_ranges, &mut patch.ip_ranges);
    merge_json_array(&org.asns, &mut patch.asns);
    merge_json_array(&org.email_domains, &mut patch.email_domains);
    merge_json_array(&org.scope_rules, &mut patch.scope_rules);
    merge_json_array(&org.certificates, &mut patch.certificates);
    merge_json_array(&org.subsidiaries, &mut patch.subsidiaries);
    merge_json_array(&org.business_systems, &mut patch.business_systems);
    merge_json_array(&org.cloud_assets, &mut patch.cloud_assets);
    merge_json_array(&org.github_orgs, &mut patch.github_orgs);
    merge_json_array(&org.social_accounts, &mut patch.social_accounts);
    merge_json_array(&org.historical_vulns, &mut patch.historical_vulns);
    match patch.contacts {
        Some(Value::Array(_)) => merge_json_array(&org.contacts, &mut patch.contacts),
        Some(Value::Object(_)) => merge_contacts_object(&org.contacts, &mut patch.contacts),
        _ => {}
    }
}

/// Fold a flat list of `ProfileFieldEntry` into a single
/// `ProfilePatch`, layered on top of the organization's existing `intel`
/// JSON.
///
/// Returns `Ok(None)` when there's nothing meaningful to write (no scalar
/// entries, no intel mutations, no contact additions) — avoiding a noisy
/// `update_profile` roundtrip on every hydrate run.
///
/// Conflict policy when multiple providers (or multiple paths in one
/// descriptor) supply the same key:
/// - Scalar: keep the **first non-empty** value seen (later providers don't
///   silently overwrite).
/// - Intel key: same — first wins (use a more specific descriptor to break
///   ties at config time).
/// - Contact channel: append unique values; duplicates from raw input are
///   dropped via lowercase-trim compare.
pub(crate) fn build_profile_patch_from_entries(
    existing_intel: &Value,
    entries: &[ProfileFieldEntry],
) -> Result<Option<golish_db::repo::organizations::ProfilePatch>, GolishError> {
    use golish_pentest::models::AssetIntelProfileFieldTarget as Target;

    let mut scalars: HashMap<String, String> = HashMap::new();
    let mut aliases: Vec<String> = Vec::new();
    let mut json_array_fields: HashMap<String, Vec<String>> = HashMap::new();
    let mut intel_overrides: HashMap<String, Value> = HashMap::new();
    let mut intel_array_fields: HashMap<String, Vec<String>> = HashMap::new();
    let mut contact_additions: HashMap<String, Vec<String>> = HashMap::new();

    fn push_unique(values: &mut Vec<String>, value: String) {
        let key = value.trim().to_lowercase();
        if !values.iter().any(|item| item.trim().to_lowercase() == key) {
            values.push(value);
        }
    }

    fn is_json_array_profile_field(field: &str) -> bool {
        matches!(
            field,
            "domains"
                | "ip_ranges"
                | "asns"
                | "email_domains"
                | "scope_rules"
                | "certificates"
                | "subsidiaries"
                | "business_systems"
                | "cloud_assets"
                | "github_orgs"
                | "social_accounts"
                | "historical_vulns"
                | "contacts"
        )
    }

    fn is_intel_array_profile_field(field: &str) -> bool {
        matches!(
            field,
            "icp_records"
                | "mobile_apps"
                | "mini_programs"
                | "app_domains"
                | "wechat_official_accounts"
                | "exposed_emails"
                | "code_leaks"
                | "code_repositories"
                | "mail_mx"
        )
    }

    for entry in entries {
        let value = entry.value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        match entry.target_kind {
            Target::Scalar => {
                if entry.target_field == "aliases" {
                    push_unique(&mut aliases, value);
                } else if is_json_array_profile_field(&entry.target_field) {
                    push_unique(
                        json_array_fields
                            .entry(entry.target_field.clone())
                            .or_default(),
                        value,
                    );
                } else {
                    scalars.entry(entry.target_field.clone()).or_insert(value);
                }
            }
            Target::Intel => {
                if is_intel_array_profile_field(&entry.target_field) {
                    push_unique(
                        intel_array_fields
                            .entry(entry.target_field.clone())
                            .or_default(),
                        value,
                    );
                } else {
                    intel_overrides
                        .entry(entry.target_field.clone())
                        .or_insert_with(|| Value::String(value));
                }
            }
            Target::Contact => {
                let bucket = contact_additions
                    .entry(entry.target_field.clone())
                    .or_default();
                let lower = value.to_lowercase();
                if !bucket.iter().any(|item| item.to_lowercase() == lower) {
                    bucket.push(value);
                }
            }
        }
    }

    let mut patch = golish_db::repo::organizations::ProfilePatch::default();
    let mut touched = false;

    if !aliases.is_empty() {
        patch.aliases = Some(aliases);
        touched = true;
    }

    for (field, value) in &scalars {
        match field.as_str() {
            "industry" => {
                patch.industry = Some(value.clone());
                touched = true;
            }
            "credit_code" => {
                patch.credit_code = Some(value.clone());
                touched = true;
            }
            "notes" => {
                patch.notes = Some(value.clone());
                touched = true;
            }
            // tier is technically scalar but constrained to enum; let users
            // promote tier manually rather than via auto-hydrate
            other => {
                tracing::debug!(
                    field = other,
                    value,
                    "asset_intel profile scalar field is not wired to ProfilePatch — ignoring"
                );
            }
        }
    }

    for (field, values) in json_array_fields {
        if values.is_empty() {
            continue;
        }
        let json = Some(Value::Array(
            values.into_iter().map(Value::String).collect(),
        ));
        match field.as_str() {
            "domains" => patch.domains = json,
            "ip_ranges" => patch.ip_ranges = json,
            "asns" => patch.asns = json,
            "email_domains" => patch.email_domains = json,
            "scope_rules" => patch.scope_rules = json,
            "certificates" => patch.certificates = json,
            "subsidiaries" => patch.subsidiaries = json,
            "business_systems" => patch.business_systems = json,
            "cloud_assets" => patch.cloud_assets = json,
            "github_orgs" => patch.github_orgs = json,
            "social_accounts" => patch.social_accounts = json,
            "historical_vulns" => patch.historical_vulns = json,
            "contacts" => patch.contacts = json,
            _ => continue,
        }
        touched = true;
    }

    let mut intel_value = if existing_intel.is_object() {
        existing_intel.clone()
    } else {
        Value::Object(serde_json::Map::new())
    };
    let intel_object = intel_value
        .as_object_mut()
        .expect("intel_value initialized as object above");

    let mut intel_touched = false;
    for (key, value) in intel_overrides {
        intel_object.entry(key).or_insert(value);
        intel_touched = true;
    }

    for (key, values) in intel_array_fields {
        if values.is_empty() {
            continue;
        }
        let entry = intel_object
            .entry(key)
            .or_insert_with(|| Value::Array(Vec::new()));
        if !entry.is_array() {
            *entry = Value::Array(vec![entry.clone()]);
        }
        let existing = entry
            .as_array_mut()
            .expect("intel array field initialized above");
        let mut seen: HashSet<String> = existing
            .iter()
            .map(display_json_atom)
            .map(|item| item.trim().to_lowercase())
            .filter(|item| !item.is_empty())
            .collect();
        for value in values {
            let key = value.trim().to_lowercase();
            if key.is_empty() || !seen.insert(key) {
                continue;
            }
            existing.push(Value::String(value));
        }
        intel_touched = true;
    }

    if !contact_additions.is_empty() {
        let contacts_entry = intel_object
            .entry("contacts")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !contacts_entry.is_object() {
            *contacts_entry = Value::Object(serde_json::Map::new());
        }
        let contacts_map = contacts_entry
            .as_object_mut()
            .expect("contacts initialized as object above");
        for (channel, mut values) in contact_additions {
            let existing_list = match contacts_map.entry(channel.clone()) {
                serde_json::map::Entry::Occupied(o) => o.into_mut(),
                serde_json::map::Entry::Vacant(v) => v.insert(Value::Array(Vec::new())),
            };
            if !existing_list.is_array() {
                *existing_list = Value::Array(Vec::new());
            }
            let list = existing_list
                .as_array_mut()
                .expect("contacts channel initialized as array above");
            let already: HashSet<String> = list
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.trim().to_lowercase()))
                .collect();
            values.retain(|item| !already.contains(&item.trim().to_lowercase()));
            for value in values {
                list.push(Value::String(value));
            }
        }
        patch.contacts = Some(contacts_entry.clone());
        intel_touched = true;
    }

    if intel_touched {
        patch.intel = Some(intel_value);
        touched = true;
    }

    if touched {
        Ok(Some(patch))
    } else {
        Ok(None)
    }
}
