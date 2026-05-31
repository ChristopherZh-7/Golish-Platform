//! Pure normalization + `when`-filter engine for asset-intel provider output.
//!
//! No DB / Tauri / IO: these helpers operate purely on `serde_json` values plus
//! the JSON-driven descriptor rules, so they can be unit-tested without a
//! database or an event loop. They are re-exported from the parent module so
//! existing call sites keep using the bare function names.

use serde_json::Value;

use super::{normalize_asn, ProfileFieldEntry};

/// Walk the descriptor's `profile_fields` rules over a single raw JSON
/// document and return every resolved (target_kind, target_field, value)
/// triple. Caller is responsible for deduping / merging across providers.
pub(crate) fn extract_profile_field_entries(
    rules: &[golish_pentest::models::AssetIntelProfileFieldRule],
    raw: &Value,
) -> Vec<ProfileFieldEntry> {
    let mut out = Vec::new();
    for rule in rules {
        for item in select_json_values(raw, &rule.path) {
            // `when` clauses are AND'd; empty = always keep. Typical use:
            // drop ENScan's "-" placeholder before it lands in contacts.
            if !filter_passes(item, &rule.when) {
                continue;
            }
            let Some(raw_value) = resolve_field_ref(item, &rule.source_field) else {
                continue;
            };
            let value = apply_profile_transform(&raw_value, &rule.transform);
            if value.trim().is_empty() {
                continue;
            }
            out.push(ProfileFieldEntry {
                target_kind: rule.target_kind.clone(),
                target_field: rule.target_field.clone(),
                value,
            });
        }
    }
    out
}

/// Returns true when every filter clause matches the given JSON item.
///
/// Operators apply via [`apply_filter_op`]:
/// numeric ops (`gte`, `gt`, `lte`, `lt`) try f64 first then string compare;
/// equality ops (`eq`, `ne`) try f64 first then case-insensitive string compare;
/// `exists` / `missing` only check field presence + non-empty value;
/// `contains` does case-insensitive substring compare.
pub(crate) fn filter_passes(
    item: &Value,
    filters: &[golish_pentest::models::AssetIntelNormalizeFilter],
) -> bool {
    filters.iter().all(|clause| {
        let raw = resolve_value_field(item, &clause.field);
        apply_filter_op(&clause.op, raw.as_deref(), &clause.value)
    })
}

fn apply_filter_op(
    op: &golish_pentest::models::AssetIntelNormalizeFilterOp,
    raw: Option<&str>,
    compare_to: &str,
) -> bool {
    use golish_pentest::models::AssetIntelNormalizeFilterOp as Op;
    let value = raw.unwrap_or("").trim();
    let compare_to_trimmed = compare_to.trim();
    let parse = |s: &str| -> Option<f64> {
        s.trim()
            .trim_end_matches('%')
            .replace(',', "")
            .parse::<f64>()
            .ok()
    };

    match op {
        Op::Exists => !value.is_empty(),
        Op::Missing => value.is_empty(),
        Op::Contains => {
            !value.is_empty()
                && value
                    .to_lowercase()
                    .contains(&compare_to_trimmed.to_lowercase())
        }
        Op::Eq | Op::Ne => {
            let equal = match (parse(value), parse(compare_to_trimmed)) {
                (Some(a), Some(b)) => (a - b).abs() < f64::EPSILON,
                _ => value.eq_ignore_ascii_case(compare_to_trimmed),
            };
            matches!(op, Op::Eq) == equal
        }
        Op::Gte | Op::Gt | Op::Lte | Op::Lt => {
            let (Some(a), Some(b)) = (parse(value), parse(compare_to_trimmed)) else {
                // Non-numeric comparison falls through to string ordering so
                // descriptors that compare e.g. dates "2024-01-01" still work.
                let ord = value.cmp(compare_to_trimmed);
                return match op {
                    Op::Gte => !matches!(ord, std::cmp::Ordering::Less),
                    Op::Gt => matches!(ord, std::cmp::Ordering::Greater),
                    Op::Lte => !matches!(ord, std::cmp::Ordering::Greater),
                    Op::Lt => matches!(ord, std::cmp::Ordering::Less),
                    _ => unreachable!(),
                };
            };
            match op {
                Op::Gte => a >= b,
                Op::Gt => a > b,
                Op::Lte => a <= b,
                Op::Lt => a < b,
                _ => unreachable!(),
            }
        }
    }
}

pub(crate) fn select_json_values<'a>(raw: &'a Value, path: &str) -> Vec<&'a Value> {
    if path == "$" {
        return vec![raw];
    }
    let Some(field) = path
        .strip_prefix("$..")
        .and_then(|rest| rest.strip_suffix("[*]"))
    else {
        return Vec::new();
    };

    fn visit<'a>(value: &'a Value, field: &str, out: &mut Vec<&'a Value>) {
        match value {
            Value::Object(map) => {
                if let Some(found) = map.get(field) {
                    match found {
                        Value::Array(items) => out.extend(items),
                        other => out.push(other),
                    }
                }
                for child in map.values() {
                    visit(child, field, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item, field, out);
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    visit(raw, field, &mut out);
    out
}

pub(crate) fn resolve_field_ref(
    value: &Value,
    field_ref: &golish_pentest::models::AssetIntelFieldRef,
) -> Option<String> {
    match field_ref {
        golish_pentest::models::AssetIntelFieldRef::Field(field) => {
            resolve_value_field(value, field)
        }
        golish_pentest::models::AssetIntelFieldRef::FirstOf(fields) => fields
            .iter()
            .find_map(|field| resolve_value_field(value, field)),
    }
}

fn resolve_value_field(value: &Value, field: &str) -> Option<String> {
    let resolved = if field.is_empty() {
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        }
    } else {
        golish_core::utils::resolve_json_path(value, field)
    }?;
    let trimmed = resolved.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn apply_profile_transform(
    raw: &str,
    transform: &golish_pentest::models::AssetIntelProfileFieldTransform,
) -> String {
    use golish_pentest::models::AssetIntelProfileFieldTransform as T;
    match transform {
        T::None => raw.to_string(),
        T::Trim => raw.trim().to_string(),
        T::Lower => raw.trim().to_lowercase(),
        T::Upper => raw.trim().to_uppercase(),
        T::Asn => normalize_asn(raw),
    }
}
