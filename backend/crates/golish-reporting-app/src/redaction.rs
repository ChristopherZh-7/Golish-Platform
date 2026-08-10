use serde_json::{Map, Value};

use crate::ReportingAppError;

fn is_secret_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().replace('-', "_").as_str(),
        "password"
            | "secret"
            | "token"
            | "api_key"
            | "private_key"
            | "cookie"
            | "authorization"
            | "credential"
            | "request_body"
            | "response_body"
            | "stdout"
            | "stderr"
            | "raw_request"
            | "raw_response"
            | "payload"
            | "email"
            | "phone"
            | "session_id"
    )
}

fn string_is_forbidden(value: &str) -> bool {
    value.len() > 16 * 1024
        || value.chars().any(|character| {
            character == '\0'
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        })
}

pub fn redact_report_value(value: Value) -> Result<Value, ReportingAppError> {
    match value {
        Value::Object(map) => {
            let mut redacted = Map::new();
            for (key, value) in map {
                if is_secret_key(&key) {
                    return Err(ReportingAppError::Validation(
                        "report_projection_forbidden_value".to_owned(),
                    ));
                }
                redacted.insert(key, redact_report_value(value)?);
            }
            Ok(Value::Object(redacted))
        }
        Value::Array(items) => items
            .into_iter()
            .map(redact_report_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::String(value) if string_is_forbidden(&value) => Err(ReportingAppError::Validation(
            "report_projection_forbidden_value".to_owned(),
        )),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rejects_secret_value_instead_of_publishing_a_replacement() {
        let error = redact_report_value(json!({
            "password": "hunter2",
            "safeReferenceHash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }))
        .expect_err("secret-bearing projection must fail closed");
        assert_eq!(
            error,
            ReportingAppError::Validation("report_projection_forbidden_value".to_owned())
        );
    }
}
