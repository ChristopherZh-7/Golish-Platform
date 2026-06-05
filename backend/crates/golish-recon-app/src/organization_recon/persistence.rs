use golish_app_core::GolishError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::types::{NormalizedReconRecord, ReconRecordKind};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistenceSummary {
    pub record_count: usize,
    pub target_inserted: usize,
    pub target_existing: usize,
    pub profile_updates: usize,
    pub unsupported_records: usize,
    pub record_results: Vec<PersistenceRecordResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistenceRecordResult {
    pub record_id: String,
    pub kind: ReconRecordKind,
    pub key: String,
    pub value: String,
    pub status: PersistenceRecordStatus,
    pub action: String,
    pub evidence_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PersistenceRecordStatus {
    Inserted,
    Existing,
    ProfileUpdated,
    Unsupported,
}

impl PersistenceSummary {
    fn push_result(
        &mut self,
        record: &NormalizedReconRecord,
        status: PersistenceRecordStatus,
        action: impl Into<String>,
        target_type: Option<&str>,
        error: Option<String>,
    ) {
        self.record_results.push(PersistenceRecordResult {
            record_id: record.record_id.clone(),
            kind: record.kind.clone(),
            key: record.key.clone(),
            value: record.value.clone(),
            status,
            action: action.into(),
            evidence_count: record.evidence.len(),
            target_type: target_type.map(str::to_string),
            error,
        });
    }
}

pub(crate) async fn persist_normalized_records(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
    run_id: &str,
    records: &[NormalizedReconRecord],
    manifest_path: &str,
) -> Result<PersistenceSummary, GolishError> {
    let mut tx = pool.begin().await?;
    let mut summary = PersistenceSummary {
        record_count: records.len(),
        ..PersistenceSummary::default()
    };

    let mut profile = ProfileAccumulator::from_organization(organization);
    for record in records {
        if let Some(target_type) = target_type_for_record(organization, record) {
            let existed = persist_target_record(&mut tx, organization, record, target_type).await?;
            if existed {
                summary.target_existing += 1;
                summary.push_result(
                    record,
                    PersistenceRecordStatus::Existing,
                    "target_link_existing",
                    Some(target_type),
                    None,
                );
            } else {
                summary.target_inserted += 1;
                summary.push_result(
                    record,
                    PersistenceRecordStatus::Inserted,
                    "target_insert",
                    Some(target_type),
                    None,
                );
            }
            continue;
        }

        if profile.merge_record(record) {
            summary.profile_updates += 1;
            summary.push_result(
                record,
                PersistenceRecordStatus::ProfileUpdated,
                "organization_profile_merge",
                None,
                None,
            );
        } else {
            summary.unsupported_records += 1;
            summary.push_result(
                record,
                PersistenceRecordStatus::Unsupported,
                "unsupported_record",
                None,
                Some(format!(
                    "no persistence mapping for {} record",
                    record_kind_label(&record.kind)
                )),
            );
        }
    }

    profile.write(&mut tx, organization.id).await?;
    write_audit(&mut tx, organization, run_id, &summary, manifest_path).await?;
    tx.commit().await?;
    Ok(summary)
}

fn target_type_for_record(
    organization: &golish_db::models::Organization,
    record: &NormalizedReconRecord,
) -> Option<&'static str> {
    match record.kind {
        ReconRecordKind::Domain if record_belongs_to_organization(organization, record) => {
            Some("domain")
        }
        ReconRecordKind::Ip => Some("ip"),
        ReconRecordKind::Url if record_belongs_to_organization(organization, record) => Some("url"),
        ReconRecordKind::Site
            if url::Url::parse(&record.value).is_ok()
                && record_belongs_to_organization(organization, record) =>
        {
            Some("url")
        }
        _ => None,
    }
}

fn record_belongs_to_organization(
    organization: &golish_db::models::Organization,
    record: &NormalizedReconRecord,
) -> bool {
    match record.kind {
        ReconRecordKind::Domain | ReconRecordKind::Url | ReconRecordKind::Site => {
            value_belongs_to_organization(organization, &record.value)
        }
        _ => true,
    }
}

fn value_belongs_to_organization(
    organization: &golish_db::models::Organization,
    value: &str,
) -> bool {
    if value.trim().parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let Some(host) = normalized_host(value) else {
        return false;
    };
    if is_known_public_non_asset_host(&host) {
        return false;
    }
    let domains = organization_owned_domains(organization);
    if domains.is_empty() {
        return false;
    }
    domains
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn organization_owned_domains(organization: &golish_db::models::Organization) -> Vec<String> {
    let mut domains = Vec::new();
    collect_owned_domain_values(&mut domains, &organization.domains);
    if let Some(intel) = organization.intel.as_object() {
        if let Some(value) = intel.get("app_domains") {
            collect_owned_domain_values(&mut domains, value);
        }
    }
    domains.sort();
    domains.dedup();
    domains
}

fn collect_owned_domain_values(domains: &mut Vec<String>, value: &Value) {
    for item in json_atom_values(value) {
        if let Some(host) = normalized_host(&item) {
            if !is_known_public_non_asset_host(&host) {
                domains.push(host);
            }
        }
    }
}

fn json_atom_values(value: &Value) -> Vec<String> {
    match value {
        Value::Null => Vec::new(),
        Value::String(text) => {
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![text.trim().to_string()]
            }
        }
        Value::Array(items) => items.iter().flat_map(json_atom_values).collect(),
        Value::Object(map) => {
            for key in ["domain", "url", "host", "value", "name"] {
                if let Some(value) = map.get(key).and_then(Value::as_str) {
                    if !value.trim().is_empty() {
                        return vec![value.trim().to_string()];
                    }
                }
            }
            map.values().flat_map(json_atom_values).collect()
        }
        other => vec![other.to_string()],
    }
}

fn normalized_host(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty() || value.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    if let Ok(url) = url::Url::parse(&value) {
        return url
            .host_str()
            .map(|host| host.trim_start_matches("www.").to_string());
    }
    if looks_like_domain(&value) {
        return Some(value.trim_start_matches("www.").to_string());
    }
    None
}

fn looks_like_domain(value: &str) -> bool {
    let value = value.trim().trim_end_matches('.');
    if value.contains(char::is_whitespace) || !value.contains('.') {
        return false;
    }
    value.split('.').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    })
}

fn is_known_public_non_asset_host(host: &str) -> bool {
    const PUBLIC_HOSTS: &[&str] = &[
        "github.com",
        "gitlab.com",
        "bitbucket.org",
        "gitee.com",
        "126.com",
        "163.com",
        "gmail.com",
        "hotmail.com",
        "outlook.com",
        "qq.com",
    ];
    PUBLIC_HOSTS
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn record_kind_label(kind: &ReconRecordKind) -> &'static str {
    match kind {
        ReconRecordKind::Organization => "organization",
        ReconRecordKind::Domain => "domain",
        ReconRecordKind::Ip => "ip",
        ReconRecordKind::Port => "port",
        ReconRecordKind::Service => "service",
        ReconRecordKind::Url => "url",
        ReconRecordKind::Site => "site",
        ReconRecordKind::App => "app",
        ReconRecordKind::MiniProgram => "mini_program",
        ReconRecordKind::Wechat => "wechat",
        ReconRecordKind::Certificate => "certificate",
        ReconRecordKind::Contact => "contact",
        ReconRecordKind::Leak => "leak",
    }
}

async fn persist_target_record(
    tx: &mut Transaction<'_, Postgres>,
    organization: &golish_db::models::Organization,
    record: &NormalizedReconRecord,
    target_type: &str,
) -> Result<bool, GolishError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM targets
           WHERE value = $1
             AND project_path IS NOT DISTINCT FROM $2
           LIMIT 1"#,
    )
    .bind(&record.value)
    .bind(&organization.project_path)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(id) = existing {
        sqlx::query(
            r#"UPDATE targets
               SET organization_id = COALESCE(organization_id, $2),
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(organization.id)
        .execute(&mut **tx)
        .await?;
        return Ok(true);
    }

    sqlx::query(
        r#"INSERT INTO targets
              (name, target_type, value, tags, notes, scope, grp, owner,
               organization_id, project_path, source, parent_id)
           VALUES
              ($1, $2::target_type, $3, '[]', '', 'in'::scope_type, 'default', '',
               $4, $5, 'organization_recon', NULL)"#,
    )
    .bind(&record.value)
    .bind(target_type)
    .bind(&record.value)
    .bind(organization.id)
    .bind(&organization.project_path)
    .execute(&mut **tx)
    .await?;
    Ok(false)
}

async fn write_audit(
    tx: &mut Transaction<'_, Postgres>,
    organization: &golish_db::models::Organization,
    run_id: &str,
    summary: &PersistenceSummary,
    manifest_path: &str,
) -> Result<(), GolishError> {
    let run_uuid = Uuid::parse_str(run_id).ok();
    let detail = json!({
        "runId": run_id,
        "organizationId": organization.id,
        "recordCount": summary.record_count,
        "targetInserted": summary.target_inserted,
        "targetExisting": summary.target_existing,
        "profileUpdates": summary.profile_updates,
        "unsupportedRecords": summary.unsupported_records,
        "recordResults": summary.record_results,
        "manifestPath": manifest_path,
    });
    sqlx::query(
        r#"INSERT INTO audit_log
              (action, category, details, project_path, source,
               target_id, session_id, tool_name, status, detail, run_id)
           VALUES
              ('organization_recon_persisted', 'recon',
               'Organization recon records persisted',
               $1, 'organization_recon', NULL, $2, 'organization_recon',
               'completed', $3, $4)"#,
    )
    .bind(&organization.project_path)
    .bind(run_id)
    .bind(detail)
    .bind(run_uuid)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

struct ProfileAccumulator {
    domains: Value,
    ip_ranges: Value,
    email_domains: Value,
    intel: Value,
    certificates: Value,
    business_systems: Value,
    social_accounts: Value,
    historical_vulns: Value,
    contacts: Value,
}

impl ProfileAccumulator {
    fn from_organization(organization: &golish_db::models::Organization) -> Self {
        Self {
            domains: array_or_empty(&organization.domains),
            ip_ranges: array_or_empty(&organization.ip_ranges),
            email_domains: array_or_empty(&organization.email_domains),
            intel: object_or_empty(&organization.intel),
            certificates: array_or_empty(&organization.certificates),
            business_systems: array_or_empty(&organization.business_systems),
            social_accounts: array_or_empty(&organization.social_accounts),
            historical_vulns: array_or_empty(&organization.historical_vulns),
            contacts: object_or_empty(&organization.contacts),
        }
    }

    fn merge_record(&mut self, record: &NormalizedReconRecord) -> bool {
        let field = record
            .attributes
            .get("profileField")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match record.kind {
            ReconRecordKind::App => push_intel_array(&mut self.intel, "mobile_apps", &record.value),
            ReconRecordKind::MiniProgram => {
                push_intel_array(&mut self.intel, "mini_programs", &record.value)
            }
            ReconRecordKind::Wechat => push_json_array(&mut self.social_accounts, &record.value),
            ReconRecordKind::Certificate => push_json_array(&mut self.certificates, &record.value),
            ReconRecordKind::Contact => {
                let channel = contact_channel(field, &record.value);
                push_contact(&mut self.contacts, channel, &record.value)
            }
            ReconRecordKind::Leak => {
                if field.contains("historical_vulns") {
                    push_json_array(&mut self.historical_vulns, &record.value)
                } else {
                    push_intel_array(&mut self.intel, "leaks", &record.value)
                }
            }
            ReconRecordKind::Domain => {
                if field.contains("email_domains") {
                    push_json_array(&mut self.email_domains, &record.value)
                } else if field.contains("mail_mx") {
                    push_intel_array(&mut self.intel, "mail_mx", &record.value)
                } else {
                    push_json_array(&mut self.domains, &record.value)
                }
            }
            ReconRecordKind::Ip => push_json_array(&mut self.ip_ranges, &record.value),
            ReconRecordKind::Url | ReconRecordKind::Site => {
                push_json_array(&mut self.business_systems, &record.value)
            }
            ReconRecordKind::Organization | ReconRecordKind::Port | ReconRecordKind::Service => {
                false
            }
        }
    }

    async fn write(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> Result<(), GolishError> {
        sqlx::query(
            r#"UPDATE organizations
               SET domains = $1,
                   ip_ranges = $2,
                   email_domains = $3,
                   intel = $4,
                   certificates = $5,
                   business_systems = $6,
                   social_accounts = $7,
                   historical_vulns = $8,
                   contacts = $9,
                   updated_at = NOW()
               WHERE id = $10"#,
        )
        .bind(&self.domains)
        .bind(&self.ip_ranges)
        .bind(&self.email_domains)
        .bind(&self.intel)
        .bind(&self.certificates)
        .bind(&self.business_systems)
        .bind(&self.social_accounts)
        .bind(&self.historical_vulns)
        .bind(&self.contacts)
        .bind(organization_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}

fn array_or_empty(value: &Value) -> Value {
    if value.is_array() {
        value.clone()
    } else {
        Value::Array(Vec::new())
    }
}

fn object_or_empty(value: &Value) -> Value {
    if value.is_object() {
        value.clone()
    } else {
        Value::Object(Map::new())
    }
}

fn push_json_array(target: &mut Value, value: &str) -> bool {
    if !target.is_array() {
        *target = Value::Array(Vec::new());
    }
    let Some(items) = target.as_array_mut() else {
        return false;
    };
    push_unique_string(items, value)
}

fn push_intel_array(intel: &mut Value, key: &str, value: &str) -> bool {
    if !intel.is_object() {
        *intel = Value::Object(Map::new());
    }
    let Some(map) = intel.as_object_mut() else {
        return false;
    };
    let entry = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(vec![entry.clone()]);
    }
    let Some(items) = entry.as_array_mut() else {
        return false;
    };
    push_unique_string(items, value)
}

fn push_contact(contacts: &mut Value, channel: &str, value: &str) -> bool {
    if !contacts.is_object() {
        *contacts = Value::Object(Map::new());
    }
    let Some(map) = contacts.as_object_mut() else {
        return false;
    };
    let entry = map
        .entry(channel.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    let Some(items) = entry.as_array_mut() else {
        return false;
    };
    push_unique_string(items, value)
}

fn push_unique_string(items: &mut Vec<Value>, value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let key = value.to_lowercase();
    if items
        .iter()
        .filter_map(Value::as_str)
        .any(|existing| existing.trim().to_lowercase() == key)
    {
        return true;
    }
    items.push(Value::String(value.into()));
    true
}

fn contact_channel(field: &str, value: &str) -> &'static str {
    if field.contains("phone") || value.chars().filter(|ch| ch.is_ascii_digit()).count() >= 7 {
        "phone"
    } else if field.contains("email") || value.contains('@') {
        "email"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organization_recon::types::ReconEvidenceRef;

    fn record(kind: ReconRecordKind, value: &str, field: &str) -> NormalizedReconRecord {
        NormalizedReconRecord {
            record_id: format!("id:{value}"),
            kind,
            key: format!("key:{value}"),
            value: value.into(),
            attributes: json!({ "profileField": field }),
            evidence: vec![ReconEvidenceRef {
                source_id: "fixture".into(),
                run_id: "run".into(),
                task_id: "processing".into(),
                raw_artifact_path: "raw/profile.json".into(),
            }],
        }
    }

    #[test]
    fn profile_merge_routes_asset_record_types() {
        let org = golish_db::models::Organization {
            id: Uuid::new_v4(),
            project_path: "/tmp/project".into(),
            name: "Org".into(),
            parent_id: None,
            description: String::new(),
            owner: String::new(),
            sort_order: 0,
            aliases: Vec::new(),
            industry: String::new(),
            tier: String::new(),
            credit_code: String::new(),
            domains: json!(["example.com"]),
            ip_ranges: json!([]),
            asns: json!([]),
            email_domains: json!([]),
            scope_rules: json!([]),
            intel: json!({}),
            notes: String::new(),
            certificates: json!([]),
            subsidiaries: json!([]),
            business_systems: json!([]),
            cloud_assets: json!([]),
            github_orgs: json!([]),
            social_accounts: json!([]),
            historical_vulns: json!([]),
            contacts: json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let mut profile = ProfileAccumulator::from_organization(&org);

        assert!(profile.merge_record(&record(
            ReconRecordKind::App,
            "平安金管家",
            "intel.mobile_apps"
        )));
        assert!(profile.merge_record(&record(
            ReconRecordKind::MiniProgram,
            "平安好车主",
            "intel.mini_programs",
        )));
        assert!(profile.merge_record(&record(
            ReconRecordKind::Wechat,
            "pingan",
            "social_accounts",
        )));
        assert!(profile.merge_record(&record(
            ReconRecordKind::Contact,
            "security@example.com",
            "contacts.email",
        )));

        assert_eq!(profile.intel["mobile_apps"], json!(["平安金管家"]));
        assert_eq!(profile.intel["mini_programs"], json!(["平安好车主"]));
        assert_eq!(profile.social_accounts, json!(["pingan"]));
        assert_eq!(profile.contacts["email"], json!(["security@example.com"]));
    }

    #[test]
    fn target_type_rejects_public_code_host_outside_org_domains() {
        let org = golish_db::models::Organization {
            id: Uuid::new_v4(),
            project_path: "/tmp/project".into(),
            name: "Org".into(),
            parent_id: None,
            description: String::new(),
            owner: String::new(),
            sort_order: 0,
            aliases: Vec::new(),
            industry: String::new(),
            tier: String::new(),
            credit_code: String::new(),
            domains: json!(["pingan.com.cn"]),
            ip_ranges: json!([]),
            asns: json!([]),
            email_domains: json!(["126.com"]),
            scope_rules: json!({}),
            intel: json!({ "app_domains": ["app.pingan.com.cn"] }),
            notes: String::new(),
            certificates: json!([]),
            subsidiaries: json!([]),
            business_systems: json!([]),
            cloud_assets: json!([]),
            github_orgs: json!([]),
            social_accounts: json!([]),
            historical_vulns: json!([]),
            contacts: json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        assert_eq!(
            target_type_for_record(
                &org,
                &record(
                    ReconRecordKind::Url,
                    "https://github.com/example/leak/blob/main/key.txt",
                    ""
                )
            ),
            None
        );
        assert_eq!(
            target_type_for_record(&org, &record(ReconRecordKind::Domain, "126.com", "")),
            None
        );
        assert_eq!(
            target_type_for_record(
                &org,
                &record(ReconRecordKind::Url, "https://www.pingan.com.cn/", "")
            ),
            Some("url")
        );
    }

    #[test]
    fn persistence_summary_serializes_per_record_results() {
        let domain = record(ReconRecordKind::Domain, "PingAn.COM", "domains");
        let app = record(ReconRecordKind::App, "平安金管家", "intel.mobile_apps");
        let port = record(ReconRecordKind::Port, "example.com:443/tcp", "");
        let mut summary = PersistenceSummary {
            record_count: 3,
            ..PersistenceSummary::default()
        };

        summary.target_inserted += 1;
        summary.push_result(
            &domain,
            PersistenceRecordStatus::Inserted,
            "target_insert",
            Some("domain"),
            None,
        );
        summary.profile_updates += 1;
        summary.push_result(
            &app,
            PersistenceRecordStatus::ProfileUpdated,
            "organization_profile_merge",
            None,
            None,
        );
        summary.unsupported_records += 1;
        summary.push_result(
            &port,
            PersistenceRecordStatus::Unsupported,
            "unsupported_record",
            None,
            Some("no persistence mapping for port record".into()),
        );

        let json = serde_json::to_value(&summary).unwrap();

        assert_eq!(summary.record_results.len(), summary.record_count);
        assert_eq!(json["recordResults"][0]["status"], "inserted");
        assert_eq!(json["recordResults"][0]["targetType"], "domain");
        assert_eq!(json["recordResults"][1]["status"], "profile_updated");
        assert_eq!(json["recordResults"][2]["status"], "unsupported");
        assert_eq!(
            json["recordResults"][2]["error"],
            "no persistence mapping for port record"
        );
    }
}
