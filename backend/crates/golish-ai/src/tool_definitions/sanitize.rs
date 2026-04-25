//! JSON-Schema sanitisation for LLM provider compatibility.
//!
//! [`sanitize_schema`] recursively:
//! - Strips `anyOf` / `allOf` / `oneOf` (Anthropic doesn't support them).
//! - Simplifies nested `oneOf` in properties to use the first option.
//! - For OpenAI Responses API strict mode: adds
//!   `additionalProperties: false`, makes optional properties nullable
//!   (`type` becomes `[t, "null"]`), and includes all properties in
//!   `required`.

use std::collections::HashSet;

use serde_json::json;

/// Sanitise a JSON schema for LLM provider compatibility. See module-level
/// docs for the full list of transformations.
pub fn sanitize_schema(schema: serde_json::Value) -> serde_json::Value {
    sanitize_schema_recursive(schema, &HashSet::new())
}

/// Internal recursive sanitisation with context about which properties were
/// originally required at the parent level.
fn sanitize_schema_recursive(
    mut schema: serde_json::Value,
    _parent_required: &HashSet<String>,
) -> serde_json::Value {
    if let Some(obj) = schema.as_object_mut() {
        // Strip top-level anyOf/allOf/oneOf
        obj.remove("anyOf");
        obj.remove("allOf");
        obj.remove("oneOf");

        // Detect object-typed schemas
        let is_object_type = obj
            .get("type")
            .map(|t| {
                t == "object" || (t.is_array() && t.as_array().unwrap().contains(&json!("object")))
            })
            .unwrap_or(false);

        // Add additionalProperties: false for object types (OpenAI strict mode).
        if is_object_type || obj.contains_key("properties") {
            obj.insert(
                "additionalProperties".to_string(),
                serde_json::Value::Bool(false),
            );
        }

        // Capture the originally-required properties at this level.
        let originally_required: HashSet<String> = obj
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Collect all property keys (used to rewrite the required array later).
        let mut all_property_keys: Vec<String> = Vec::new();

        // Recursively sanitise properties and make optional ones nullable.
        if let Some(props) = obj.get_mut("properties") {
            if let Some(props_obj) = props.as_object_mut() {
                all_property_keys = props_obj.keys().cloned().collect();

                for (key, prop_value) in props_obj.iter_mut() {
                    // Simplify oneOf in property descriptors first.
                    if let Some(prop_obj) = prop_value.as_object_mut() {
                        if prop_obj.contains_key("oneOf") {
                            if let Some(one_of) = prop_obj.remove("oneOf") {
                                if let Some(arr) = one_of.as_array() {
                                    if let Some(first) = arr.first() {
                                        if let Some(first_obj) = first.as_object() {
                                            for (k, v) in first_obj {
                                                prop_obj.insert(k.clone(), v.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        prop_obj.remove("anyOf");
                        prop_obj.remove("allOf");
                    }

                    // Recurse into nested schemas.
                    *prop_value =
                        sanitize_schema_recursive(prop_value.take(), &originally_required);

                    // For optional properties (not in original required array), make them
                    // nullable by extending `type` to include "null".
                    if !originally_required.contains(key) {
                        if let Some(prop_obj) = prop_value.as_object_mut() {
                            if let Some(type_val) = prop_obj.get_mut("type") {
                                if let Some(type_str) = type_val.as_str() {
                                    *type_val = json!([type_str, "null"]);
                                } else if let Some(type_arr) = type_val.as_array_mut() {
                                    if !type_arr.iter().any(|v| v == "null") {
                                        type_arr.push(json!("null"));
                                    }
                                }
                            } else if !prop_obj.contains_key("properties")
                                && !prop_obj.contains_key("items")
                            {
                                // Only inject a default type for non-complex schemas.
                                prop_obj.insert("type".to_string(), json!(["string", "null"]));
                            }
                        }
                    }
                }
            }
        }

        // Recurse into array `items`.
        if let Some(items) = obj.get_mut("items") {
            *items = sanitize_schema_recursive(items.take(), &HashSet::new());
        }

        // Set all properties as required (OpenAI Responses API strict mode).
        if !all_property_keys.is_empty() {
            let required_array: Vec<serde_json::Value> = all_property_keys
                .into_iter()
                .map(serde_json::Value::String)
                .collect();
            obj.insert(
                "required".to_string(),
                serde_json::Value::Array(required_array),
            );
        }
    }
    schema
}
