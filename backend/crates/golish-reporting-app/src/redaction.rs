use serde_json::{Map, Value};

use crate::ReportingAppError;

fn is_secret_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "password" | "secret" | "token" | "api_key" | "private_key" | "cookie"
    )
}

pub fn redact_report_value(value: Value) -> Result<Value, ReportingAppError> {
    match value {
        Value::Object(map) => {
            let mut redacted = Map::new();
            for (key, value) in map {
                if is_secret_key(&key) {
                    redacted.insert(key, Value::String("[REDACTED]".to_string()));
                } else {
                    redacted.insert(key, redact_report_value(value)?);
                }
            }
            Ok(Value::Object(redacted))
        }
        Value::Array(items) => items
            .into_iter()
            .map(redact_report_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn removes_secret_value_but_preserves_vault_reference() {
        let redacted = redact_report_value(json!({
            "password": "hunter2",
            "credentialRef": "vault_ref:00000000-0000-0000-0000-000000000021"
        }))
        .expect("redaction");
        assert!(!redacted.to_string().contains("hunter2"));
        assert!(redacted.to_string().contains("vault_ref:"));
    }
}
