//! Strongly-typed entity / relation models for the graph knowledge base.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::GraphError;

/// A node in the security knowledge graph.
///
/// Identity is `(entity_type, name, project_id)`; the database enforces this
/// via a unique index so [`crate::client::GraphClient::upsert_entity`] is
/// idempotent.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GraphEntity {
    /// Server-generated UUID primary key.
    pub id: Uuid,
    /// One of the [`EntityType`] variants, persisted as a lowercase string.
    pub entity_type: String,
    /// Human-readable name (e.g. `"10.0.0.5"`, `"http/8080"`, `"CVE-2024-1234"`).
    pub name: String,
    /// Free-form JSON payload (banner, CVSS, evidence, ...).
    pub properties: Value,
    /// Optional session this entity was discovered in.
    pub session_id: Option<Uuid>,
    /// Optional project scope used for dedupe / multi-tenant isolation.
    pub project_id: Option<String>,
    /// First time this entity was inserted.
    pub created_at: DateTime<Utc>,
    /// Last time the entity row was modified (UPSERT bumps this).
    pub updated_at: DateTime<Utc>,
}

/// A directed, typed edge between two [`GraphEntity`] nodes.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GraphRelation {
    /// Server-generated UUID primary key.
    pub id: Uuid,
    /// Source endpoint of the edge.
    pub from_entity_id: Uuid,
    /// Destination endpoint of the edge.
    pub to_entity_id: Uuid,
    /// One of the [`RelationType`] variants, persisted as a lowercase string.
    pub relation_type: String,
    /// Edge metadata (port, exploit reference, evidence, ...).
    pub properties: Value,
    /// Insertion timestamp.
    pub created_at: DateTime<Utc>,
}

/// Materialized result of a graph query: the set of nodes and edges touched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphQueryResult {
    /// Entities included in the result set.
    pub entities: Vec<GraphEntity>,
    /// Relations connecting the entities above.
    pub relations: Vec<GraphRelation>,
}

/// The closed set of entity kinds the graph accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    /// A network-reachable host (IP, domain, subnet member).
    Host,
    /// A service exposed on a host (`http/443`, `ssh/22`, ...).
    Service,
    /// A vulnerability instance (`CVE-2024-1234`, custom finding ID, ...).
    Vulnerability,
    /// A captured or guessed credential.
    Credential,
    /// An attacker technique (MITRE ATT&CK, custom playbook step, ...).
    Technique,
    /// An application endpoint (URL path, API route).
    Endpoint,
}

impl EntityType {
    /// Stable lowercase string used in the database `entity_type` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityType::Host => "host",
            EntityType::Service => "service",
            EntityType::Vulnerability => "vulnerability",
            EntityType::Credential => "credential",
            EntityType::Technique => "technique",
            EntityType::Endpoint => "endpoint",
        }
    }
}

impl fmt::Display for EntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EntityType {
    type Err = GraphError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "host" => Ok(EntityType::Host),
            "service" => Ok(EntityType::Service),
            "vulnerability" => Ok(EntityType::Vulnerability),
            "credential" => Ok(EntityType::Credential),
            "technique" => Ok(EntityType::Technique),
            "endpoint" => Ok(EntityType::Endpoint),
            other => Err(GraphError::UnknownEntityType(other.to_string())),
        }
    }
}

/// The closed set of relation kinds the graph natively understands.
///
/// Storage allows arbitrary `relation_type` strings, but using a known variant
/// keeps queries portable across agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    /// `host -> service`: the host exposes the given service.
    RunsService,
    /// `service|host -> vulnerability`: the target carries a known vulnerability.
    HasVulnerability,
    /// `vulnerability -> technique|credential`: the vuln was leveraged.
    ExploitedBy,
    /// `host -> host`: lateral movement edge from the first host to the second.
    LateralMove,
    /// `credential -> service|host`: the credential authenticates against the target.
    AuthenticatesTo,
    /// `service -> endpoint`: the service exposes the given endpoint.
    ExposesEndpoint,
}

impl RelationType {
    /// Stable lowercase string used in the database `relation_type` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            RelationType::RunsService => "runs_service",
            RelationType::HasVulnerability => "has_vulnerability",
            RelationType::ExploitedBy => "exploited_by",
            RelationType::LateralMove => "lateral_move",
            RelationType::AuthenticatesTo => "authenticates_to",
            RelationType::ExposesEndpoint => "exposes_endpoint",
        }
    }
}

impl fmt::Display for RelationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RelationType {
    type Err = GraphError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "runs_service" => Ok(RelationType::RunsService),
            "has_vulnerability" => Ok(RelationType::HasVulnerability),
            "exploited_by" => Ok(RelationType::ExploitedBy),
            "lateral_move" => Ok(RelationType::LateralMove),
            "authenticates_to" => Ok(RelationType::AuthenticatesTo),
            "exposes_endpoint" => Ok(RelationType::ExposesEndpoint),
            other => Err(GraphError::UnknownRelationType(other.to_string())),
        }
    }
}
