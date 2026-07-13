use golish_memory_domain::{KnowledgeValue, VaultCredentialRef};

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RedactionError {
    #[error("plaintext secret material is forbidden in ContextPack")]
    SecretMaterial,
    #[error("context value is too large")]
    ValueTooLarge,
}

impl RedactionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SecretMaterial => "knowledge_context_secret_material_rejected",
            Self::ValueTooLarge => "knowledge_context_value_too_large",
        }
    }
}

pub fn render_safe_value(value: &KnowledgeValue) -> Result<String, RedactionError> {
    let raw = match value {
        KnowledgeValue::Text(value) => {
            reject_secret_text(value)?;
            value.clone()
        }
        KnowledgeValue::Json(value) => {
            reject_secret_json(value)?;
            value.to_string()
        }
        KnowledgeValue::VaultRef(VaultCredentialRef(reference)) => {
            return Ok(format!("vault_ref:{}", reference.hyphenated()));
        }
    };
    if raw.chars().count() > 32_768 {
        return Err(RedactionError::ValueTooLarge);
    }
    Ok(escape_prompt_markup(&raw))
}

fn reject_secret_text(value: &str) -> Result<(), RedactionError> {
    let lowercase = value.to_ascii_lowercase();
    if [
        "password=",
        "password:",
        "passwd=",
        "secret=",
        "secret:",
        "token=",
        "token:",
        "api_key=",
        "api_key:",
        "authorization:",
        "bearer ",
        "private_key",
        "-----begin private key-----",
        "-----begin rsa private key-----",
        "session_cookie=",
        "sk-",
    ]
    .iter()
    .any(|marker| lowercase.contains(marker))
    {
        return Err(RedactionError::SecretMaterial);
    }
    Ok(())
}

fn reject_secret_json(value: &serde_json::Value) -> Result<(), RedactionError> {
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if [
                    "api_key",
                    "authorization",
                    "cookie",
                    "credential",
                    "password",
                    "passwd",
                    "private_key",
                    "secret",
                    "session_cookie",
                    "token",
                ]
                .contains(&key.to_ascii_lowercase().as_str())
                    && !value.is_null()
                {
                    return Err(RedactionError::SecretMaterial);
                }
                reject_secret_json(value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                reject_secret_json(value)?;
            }
        }
        serde_json::Value::String(value) => reject_secret_text(value)?,
        _ => {}
    }
    Ok(())
}

pub fn escape_prompt_markup(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
