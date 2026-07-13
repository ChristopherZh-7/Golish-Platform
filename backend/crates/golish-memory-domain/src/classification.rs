use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::scope::ProjectScopeId;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeClassification {
    Public,
    Internal,
    CustomerConfidential,
    Restricted,
}

impl KnowledgeClassification {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::CustomerConfidential => "customer_confidential",
            Self::Restricted => "restricted",
        }
    }

    pub const fn allows_global_sanitized(self) -> bool {
        matches!(self, Self::Public | Self::Internal)
    }

    pub const fn rank(self) -> u8 {
        match self {
            Self::Public => 0,
            Self::Internal => 1,
            Self::CustomerConfidential => 2,
            Self::Restricted => 3,
        }
    }

    pub const fn allows(self, candidate: Self) -> bool {
        candidate.rank() <= self.rank()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "visibility", rename_all = "snake_case")]
pub enum AssertionVisibility {
    OrganizationLongTerm {
        project_scope_id: ProjectScopeId,
        organization_id_at_time: Uuid,
    },
    GlobalSanitized,
}

impl AssertionVisibility {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::OrganizationLongTerm { .. } => "organization_long_term",
            Self::GlobalSanitized => "global_sanitized",
        }
    }

    pub const fn project_scope_id(&self) -> Option<ProjectScopeId> {
        match self {
            Self::OrganizationLongTerm {
                project_scope_id, ..
            } => Some(*project_scope_id),
            Self::GlobalSanitized => None,
        }
    }

    pub const fn organization_id_at_time(&self) -> Option<Uuid> {
        match self {
            Self::OrganizationLongTerm {
                organization_id_at_time,
                ..
            } => Some(*organization_id_at_time),
            Self::GlobalSanitized => None,
        }
    }
}
