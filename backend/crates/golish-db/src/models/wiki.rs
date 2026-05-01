//! Wiki knowledge base models and vulnerability research tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WikiPage {
    pub id: Uuid,
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub content: String,
    pub word_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VulnKbLink {
    pub id: Uuid,
    pub cve_id: String,
    pub wiki_path: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VulnKbPoc {
    pub id: Uuid,
    pub cve_id: String,
    pub name: String,
    pub poc_type: String,
    pub language: String,
    pub content: String,
    pub source: String,
    pub source_url: String,
    pub severity: String,
    pub verified: bool,
    pub description: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CvePocSummary {
    pub cve_id: String,
    pub poc_count: i64,
    pub max_severity: Option<String>,
    pub any_verified: Option<bool>,
    pub has_research: Option<bool>,
    pub has_wiki: Option<bool>,
}

#[derive(Debug)]
pub struct NewWikiPage {
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct KbResearchLog {
    pub id: Uuid,
    pub cve_id: String,
    pub session_id: String,
    pub turns: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VulnScanHistory {
    pub id: Uuid,
    pub cve_id: String,
    pub target: String,
    pub result: String,
    pub details: Option<String>,
    pub scanned_at: DateTime<Utc>,
}

// ── Cross-References & Changelog ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WikiPageRef {
    pub id: Uuid,
    pub source_path: String,
    pub target_path: String,
    pub context: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WikiChangelog {
    pub id: i64,
    pub page_path: String,
    pub action: String,
    pub title: String,
    pub category: String,
    pub actor: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct NewWikiChangelog {
    pub page_path: String,
    pub action: String,
    pub title: String,
    pub category: String,
    pub actor: String,
    pub summary: String,
}

/// Lightweight page info returned by category-grouped queries.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WikiPageSummary {
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub word_count: i32,
    pub updated_at: DateTime<Utc>,
}
