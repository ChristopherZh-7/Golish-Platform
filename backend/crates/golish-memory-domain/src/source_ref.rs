use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CanonicalRowId {
    Uuid(Uuid),
    Int64(i64),
    Text(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredCanonicalRowId {
    pub kind: String,
    pub value: String,
}

impl StoredCanonicalRowId {
    pub fn from_domain(id: &CanonicalRowId) -> Result<Self, SourceRefError> {
        let (kind, value) = match id {
            CanonicalRowId::Uuid(value) => ("uuid", value.hyphenated().to_string()),
            CanonicalRowId::Int64(value) => ("int64", value.to_string()),
            CanonicalRowId::Text(value) => {
                let value = value.trim();
                validate_text(value)?;
                ("text", value.to_string())
            }
        };
        Ok(Self {
            kind: kind.to_string(),
            value,
        })
    }

    pub fn into_domain(self) -> Result<CanonicalRowId, SourceRefError> {
        match self.kind.as_str() {
            "uuid" => {
                let parsed = Uuid::from_str(&self.value)
                    .map_err(|_| SourceRefError::CorruptUuid(self.value.clone()))?;
                if parsed.hyphenated().to_string() != self.value {
                    return Err(SourceRefError::CorruptUuid(self.value));
                }
                Ok(CanonicalRowId::Uuid(parsed))
            }
            "int64" => {
                let parsed = self
                    .value
                    .parse::<i64>()
                    .map_err(|_| SourceRefError::CorruptInt64(self.value.clone()))?;
                if parsed.to_string() != self.value {
                    return Err(SourceRefError::CorruptInt64(self.value));
                }
                Ok(CanonicalRowId::Int64(parsed))
            }
            "text" => {
                validate_text(&self.value)?;
                if self.value.trim() != self.value {
                    return Err(SourceRefError::CorruptText(self.value));
                }
                Ok(CanonicalRowId::Text(self.value))
            }
            other => Err(SourceRefError::UnknownKind(other.to_string())),
        }
    }
}

fn validate_text(value: &str) -> Result<(), SourceRefError> {
    if value.is_empty() {
        return Err(SourceRefError::EmptyText);
    }
    if value.len() > 512 {
        return Err(SourceRefError::TextTooLong(value.len()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalSourceKind {
    StageEpisode,
    Finding,
    CandidateAttempt,
    TechniqueOutcome,
    FactDelta,
    PostExploitAction,
    Foothold,
    ObjectiveOutcome,
    CleanupObligation,
    ResidualRisk,
    ReportRevision,
}

impl CanonicalSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StageEpisode => "stage_episode",
            Self::Finding => "finding",
            Self::CandidateAttempt => "candidate_attempt",
            Self::TechniqueOutcome => "technique_outcome",
            Self::FactDelta => "fact_delta",
            Self::PostExploitAction => "post_exploit_action",
            Self::Foothold => "foothold",
            Self::ObjectiveOutcome => "objective_outcome",
            Self::CleanupObligation => "cleanup_obligation",
            Self::ResidualRisk => "residual_risk",
            Self::ReportRevision => "report_revision",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SourceRef {
    pub source_kind: CanonicalSourceKind,
    pub row_id: CanonicalRowId,
    pub source_stream_key: String,
    pub version: i64,
}

impl SourceRef {
    pub fn validate(&self) -> Result<(), SourceRefError> {
        StoredCanonicalRowId::from_domain(&self.row_id)?;
        if self.source_stream_key.trim().is_empty() {
            return Err(SourceRefError::EmptyStreamKey);
        }
        if self.version <= 0 {
            return Err(SourceRefError::NonPositiveVersion(self.version));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SourceRefError {
    #[error("canonical text row id cannot be empty")]
    EmptyText,
    #[error("canonical text row id exceeds 512 bytes: {0}")]
    TextTooLong(usize),
    #[error("unknown canonical row id kind: {0}")]
    UnknownKind(String),
    #[error("stored UUID row id is not canonical: {0}")]
    CorruptUuid(String),
    #[error("stored int64 row id is not canonical: {0}")]
    CorruptInt64(String),
    #[error("stored text row id is not canonical: {0}")]
    CorruptText(String),
    #[error("source stream key cannot be empty")]
    EmptyStreamKey,
    #[error("source version must be positive: {0}")]
    NonPositiveVersion(i64),
}

impl SourceRefError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownKind(_)
            | Self::CorruptUuid(_)
            | Self::CorruptInt64(_)
            | Self::CorruptText(_) => "memory_source_id_corrupt",
            Self::EmptyText
            | Self::TextTooLong(_)
            | Self::EmptyStreamKey
            | Self::NonPositiveVersion(_) => "memory_source_ref_invalid",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_noncanonical_stored_values_instead_of_coercing_to_text() {
        for stored in [
            StoredCanonicalRowId {
                kind: "int64".to_string(),
                value: "+42".to_string(),
            },
            StoredCanonicalRowId {
                kind: "int64".to_string(),
                value: "042".to_string(),
            },
            StoredCanonicalRowId {
                kind: "uuid".to_string(),
                value: "550E8400-E29B-41D4-A716-446655440000".to_string(),
            },
            StoredCanonicalRowId {
                kind: "text".to_string(),
                value: " padded ".to_string(),
            },
        ] {
            let error = stored
                .into_domain()
                .expect_err("stored value must fail closed");
            assert_eq!(error.code(), "memory_source_id_corrupt");
        }
    }
}
