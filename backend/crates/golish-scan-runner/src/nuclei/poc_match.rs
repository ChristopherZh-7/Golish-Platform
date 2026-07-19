//! Read-only current-owner fingerprint -> safe Nuclei template selector.
//!
//! This module deliberately does not backfill fingerprints, start a scanner,
//! or publish Findings. It only returns a bounded, deduplicated template plan
//! for the stage-owned Nuclei adapter.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use uuid::Uuid;

use crate::types::{NucleiTemplateRationale, NucleiTemplateSelection};

const MAX_TEMPLATE_ID_BYTES: usize = 128;
const MAX_FINGERPRINTS: usize = 256;
const MAX_FINGERPRINT_NAME_BYTES: usize = 256;
const MAX_FINGERPRINT_VERSION_BYTES: usize = 128;
const MAX_SEARCH_TERM_BYTES: usize = MAX_FINGERPRINT_NAME_BYTES + 1 + MAX_FINGERPRINT_VERSION_BYTES;
const MAX_COMBINED_PATTERN_BYTES: usize = 32 * 1024;
const MAX_POC_ROWS: usize = 500;
const MAX_SELECTIONS: usize = 256;
const MAX_RATIONALES: usize = 4_096;

const NUCLEI_POC_QUERY: &str = r#"SELECT DISTINCT
       id,
       cve_id,
       name,
       poc_type,
       COALESCE(severity, 'unknown') AS severity,
       COALESCE(source, '') AS source,
       COALESCE(content, '') AS content,
       COALESCE(description, '') AS description,
       COALESCE(tags, ARRAY[]::TEXT[]) AS tags
   FROM vuln_kb_pocs
   WHERE LOWER(poc_type) = 'nuclei'
     AND LOWER(source) = 'nuclei_template'
     AND UPPER(cve_id) ~ '^CVE-[0-9]{4}-[0-9]{4,}$'
     AND EXISTS (
         SELECT 1
         FROM unnest(tags) AS required_tag
         WHERE LOWER(required_tag) = 'cve'
     )
     AND (
         LOWER(name) ~* $1
         OR LOWER(cve_id) ~* $1
         OR LOWER(description) ~* $1
         OR EXISTS (
             SELECT 1
             FROM unnest(tags) AS tag
             WHERE LOWER(tag) = ANY($2)
         )
     )
   ORDER BY id
   LIMIT 501"#;

/// Select safe local Nuclei template ids using only fingerprints observed on
/// this exact authorized Web Origin. There is deliberately no target-global
/// fallback: another port/scheme on the same target is a different surface.
pub async fn select_nuclei_templates_for_origin(
    pool: &sqlx::PgPool,
    target_id: Uuid,
    exact_origin: &str,
) -> crate::ScanRunnerResult<Vec<NucleiTemplateSelection>> {
    let guard = golish_db::repo::scoped::load_target_write_guard(pool, target_id)
        .await?
        .ok_or_else(|| {
            crate::ScanRunnerError::Nuclei(
                "template selection requires a current in-scope project-bound target".to_string(),
            )
        })?;

    let web_origin_id =
        golish_db::repo::enumeration_surface_manifest::resolve_guarded_web_origin_id(
            pool,
            &guard,
            exact_origin,
        )
        .await?;
    let organization_id = guard.organization_id.ok_or_else(|| {
        crate::ScanRunnerError::Nuclei(
            "exact-origin template selection requires an organization-bound target".to_string(),
        )
    })?;
    let fingerprints = golish_db::repo::enumeration_surface_manifest::list_fingerprints_for_origin(
        pool,
        organization_id,
        web_origin_id,
    )
    .await?;
    golish_db::repo::scoped::validate_target_write_guard(pool, &guard).await?;
    if fingerprints.len() > MAX_FINGERPRINTS {
        return Err(crate::ScanRunnerError::Nuclei(format!(
            "template selection exceeds the {MAX_FINGERPRINTS} exact-origin fingerprint limit"
        )));
    }

    let fingerprints = fingerprints
        .iter()
        .map(FingerprintCandidate::from)
        .collect::<Vec<_>>();
    let (search_terms, combined_pattern) =
        bounded_search_inputs(target_id, &fingerprints).map_err(crate::ScanRunnerError::Nuclei)?;

    if search_terms.is_empty() {
        golish_db::repo::scoped::validate_target_write_guard(pool, &guard).await?;
        return Ok(Vec::new());
    }

    let tag_terms = search_terms.iter().cloned().collect::<Vec<_>>();
    let pocs = sqlx::query_as::<_, PocCandidate>(NUCLEI_POC_QUERY)
        .bind(combined_pattern)
        .bind(tag_terms)
        .fetch_all(pool)
        .await?;
    if pocs.len() > MAX_POC_ROWS {
        return Err(crate::ScanRunnerError::Nuclei(format!(
            "template selection exceeds the {MAX_POC_ROWS} matching PoC row limit"
        )));
    }

    golish_db::repo::scoped::validate_target_write_guard(pool, &guard).await?;
    let selections = select_safe_nuclei_templates(target_id, &fingerprints, &pocs);
    let rationale_count = selections
        .iter()
        .map(|selection| selection.rationales.len())
        .sum::<usize>();
    if selections.len() > MAX_SELECTIONS || rationale_count > MAX_RATIONALES {
        return Err(crate::ScanRunnerError::Nuclei(format!(
            "template selection exceeds bounded output ({MAX_SELECTIONS} templates / {MAX_RATIONALES} rationales)"
        )));
    }
    Ok(selections)
}

#[derive(Debug, Clone)]
struct FingerprintCandidate {
    id: Uuid,
    target_id: Uuid,
    name: String,
    version: Option<String>,
}

impl From<&golish_db::models::Fingerprint> for FingerprintCandidate {
    fn from(value: &golish_db::models::Fingerprint) -> Self {
        Self {
            id: value.id,
            target_id: value.target_id,
            name: value.name.clone(),
            version: value.version.clone(),
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PocCandidate {
    id: Uuid,
    cve_id: String,
    name: String,
    poc_type: String,
    severity: String,
    source: String,
    content: String,
    description: String,
    tags: Vec<String>,
}

fn select_safe_nuclei_templates(
    target_id: Uuid,
    fingerprints: &[FingerprintCandidate],
    pocs: &[PocCandidate],
) -> Vec<NucleiTemplateSelection> {
    let current_fingerprints = fingerprints
        .iter()
        .filter(|fingerprint| fingerprint.target_id == target_id)
        .collect::<Vec<_>>();
    let mut selections = BTreeMap::<String, Vec<NucleiTemplateRationale>>::new();
    let mut seen_rationales = HashSet::<(String, Uuid, Uuid)>::new();

    for poc in pocs {
        if !poc.poc_type.eq_ignore_ascii_case("nuclei")
            || !poc.source.eq_ignore_ascii_case("nuclei_template")
            || !is_strict_cve_id(&poc.cve_id)
            || !poc.tags.iter().any(|tag| tag.eq_ignore_ascii_case("cve"))
        {
            continue;
        }

        let Some(template_id) = extract_nuclei_template_id(&poc.content) else {
            continue;
        };
        if !is_safe_nuclei_template_id(&template_id) {
            continue;
        }
        if !template_id.eq_ignore_ascii_case(&poc.cve_id) || !is_strict_cve_id(&template_id) {
            continue;
        }
        if !is_safe_nuclei_template_content(&poc.content) {
            continue;
        }

        for fingerprint in &current_fingerprints {
            if !poc_matches_fingerprint(poc, fingerprint) {
                continue;
            }
            if !seen_rationales.insert((template_id.clone(), fingerprint.id, poc.id)) {
                continue;
            }
            selections
                .entry(template_id.clone())
                .or_default()
                .push(NucleiTemplateRationale {
                    fingerprint_id: fingerprint.id,
                    fingerprint_name: fingerprint.name.clone(),
                    fingerprint_version: fingerprint.version.clone(),
                    poc_id: poc.id,
                    cve_id: poc.cve_id.clone(),
                    poc_name: poc.name.clone(),
                    severity: poc.severity.clone(),
                });
        }
    }

    selections
        .into_iter()
        .map(|(template_id, mut rationales)| {
            rationales.sort_by(|left, right| {
                left.fingerprint_id
                    .cmp(&right.fingerprint_id)
                    .then_with(|| left.poc_id.cmp(&right.poc_id))
            });
            NucleiTemplateSelection {
                template_id,
                rationales,
            }
        })
        .collect()
}

fn poc_matches_fingerprint(poc: &PocCandidate, fingerprint: &FingerprintCandidate) -> bool {
    let searchable_text = format!(
        "{} {} {} {}",
        poc.name,
        poc.cve_id,
        poc.description,
        poc.tags.join(" ")
    )
    .to_ascii_lowercase();

    build_search_terms(&fingerprint.name, fingerprint.version.as_deref())
        .into_iter()
        .any(|term| !term.is_empty() && searchable_text.contains(&term))
}

fn build_search_terms(name: &str, version: Option<&str>) -> Vec<String> {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Vec::new();
    }
    let mut terms = vec![lower.clone()];

    let mapped = match lower.as_str() {
        "apache" => Some("apache"),
        "nginx" => Some("nginx"),
        "iis" | "microsoft-iis" => Some("iis"),
        "tomcat" | "apache-tomcat" => Some("tomcat"),
        "wordpress" => Some("wordpress"),
        "drupal" => Some("drupal"),
        "joomla" => Some("joomla"),
        "php" => Some("php"),
        "jquery" => Some("jquery"),
        "spring" | "spring-boot" | "spring-framework" => Some("spring"),
        "struts" | "apache-struts" => Some("struts"),
        "log4j" => Some("log4j"),
        "openssl" => Some("openssl"),
        "jenkins" => Some("jenkins"),
        "gitlab" => Some("gitlab"),
        "grafana" => Some("grafana"),
        "elasticsearch" => Some("elasticsearch"),
        "redis" => Some("redis"),
        "mongodb" | "mongo" => Some("mongodb"),
        _ => None,
    };
    if let Some(mapped) = mapped {
        if mapped != lower {
            terms.push(mapped.to_string());
        }
    }

    if let Some(version) = version.map(str::trim).filter(|version| !version.is_empty()) {
        terms.push(format!("{lower} {version}"));
    }
    terms
}

fn bounded_search_inputs(
    target_id: Uuid,
    fingerprints: &[FingerprintCandidate],
) -> Result<(BTreeSet<String>, String), String> {
    let mut search_terms = BTreeSet::new();
    for fingerprint in fingerprints
        .iter()
        .filter(|fingerprint| fingerprint.target_id == target_id)
    {
        if fingerprint.name.len() > MAX_FINGERPRINT_NAME_BYTES {
            return Err(format!(
                "fingerprint name exceeds the {MAX_FINGERPRINT_NAME_BYTES}-byte template-selection limit"
            ));
        }
        if fingerprint
            .version
            .as_deref()
            .is_some_and(|version| version.len() > MAX_FINGERPRINT_VERSION_BYTES)
        {
            return Err(format!(
                "fingerprint version exceeds the {MAX_FINGERPRINT_VERSION_BYTES}-byte template-selection limit"
            ));
        }
        for term in build_search_terms(&fingerprint.name, fingerprint.version.as_deref()) {
            if term.len() > MAX_SEARCH_TERM_BYTES {
                return Err(format!(
                    "fingerprint search term exceeds the {MAX_SEARCH_TERM_BYTES}-byte template-selection limit"
                ));
            }
            if !term.is_empty() {
                search_terms.insert(term);
            }
        }
    }
    let combined_pattern = search_terms
        .iter()
        .map(|term| regex::escape(term))
        .collect::<Vec<_>>()
        .join("|");
    if combined_pattern.len() > MAX_COMBINED_PATTERN_BYTES {
        return Err(format!(
            "combined fingerprint pattern exceeds the {MAX_COMBINED_PATTERN_BYTES}-byte template-selection limit"
        ));
    }
    Ok((search_terms, combined_pattern))
}

fn extract_nuclei_template_id(content: &str) -> Option<String> {
    content.lines().take(20).find_map(|line| {
        line.trim()
            .strip_prefix("id:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn is_safe_nuclei_template_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEMPLATE_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_strict_cve_id(value: &str) -> bool {
    let mut parts = value.trim().split('-');
    let Some(prefix) = parts.next() else {
        return false;
    };
    let Some(year) = parts.next() else {
        return false;
    };
    let Some(sequence) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && prefix.eq_ignore_ascii_case("CVE")
        && year.len() == 4
        && year.bytes().all(|byte| byte.is_ascii_digit())
        && sequence.len() >= 4
        && sequence.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_safe_nuclei_template_content(content: &str) -> bool {
    let mut has_allowed_protocol = false;
    for line in content.lines() {
        if matches!(line.as_bytes().first(), Some(b' ' | b'\t')) {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
            continue;
        }
        let Some((key, _)) = trimmed.split_once(':') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "http" | "requests" | "ssl" => has_allowed_protocol = true,
            "code" | "javascript" | "headless" | "file" | "tcp" | "network" | "dns"
            | "workflow" | "workflows" | "websocket" | "whois" | "flow" => return false,
            _ => {}
        }
    }
    has_allowed_protocol
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(
        id: u128,
        target_id: Uuid,
        name: &str,
        version: Option<&str>,
    ) -> FingerprintCandidate {
        FingerprintCandidate {
            id: Uuid::from_u128(id),
            target_id,
            name: name.to_string(),
            version: version.map(str::to_string),
        }
    }

    fn poc(id: u128, poc_type: &str, source: &str, content: &str, tags: &[&str]) -> PocCandidate {
        let mut tags = tags
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>();
        if !tags.iter().any(|value| value.eq_ignore_ascii_case("cve")) {
            tags.push("cve".to_string());
        }
        PocCandidate {
            id: Uuid::from_u128(id),
            cve_id: "CVE-2025-0001".to_string(),
            name: "WordPress example".to_string(),
            poc_type: poc_type.to_string(),
            severity: "high".to_string(),
            source: source.to_string(),
            content: content.to_string(),
            description: "WordPress vulnerability".to_string(),
            tags,
        }
    }

    #[test]
    fn selector_accepts_only_current_target_fingerprints_and_nuclei_kb_rows() {
        let current_target = Uuid::from_u128(100);
        let foreign_target = Uuid::from_u128(200);
        let fingerprints = vec![
            fingerprint(1, current_target, "WordPress", Some("6.5")),
            fingerprint(2, foreign_target, "WordPress", Some("6.4")),
        ];
        let pocs = vec![
            poc(
                10,
                "nuclei",
                "nuclei_template",
                "id: CVE-2025-0001\ninfo:\n  name: example\nhttp:\n  - method: GET",
                &["wordpress"],
            ),
            poc(
                11,
                "script",
                "github",
                "id: CVE-2025-0002\ninfo:\n  name: not-a-template",
                &["wordpress"],
            ),
        ];

        let selected = select_safe_nuclei_templates(current_target, &fingerprints, &pocs);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].template_id, "CVE-2025-0001");
        assert_eq!(selected[0].rationales.len(), 1);
        assert_eq!(selected[0].rationales[0].fingerprint_id, Uuid::from_u128(1));
    }

    #[test]
    fn selector_rejects_unsafe_template_ids_and_deduplicates_safe_ids() {
        let target_id = Uuid::from_u128(100);
        let fingerprints = vec![
            fingerprint(1, target_id, "WordPress", Some("6.5")),
            fingerprint(2, target_id, "WordPress", None),
        ];
        let pocs = vec![
            poc(
                10,
                "nuclei",
                "nuclei_template",
                "id: CVE-2025-0001\ninfo:\n  name: first\nhttp:\n  - method: GET",
                &["wordpress"],
            ),
            poc(
                11,
                "nuclei",
                "nuclei_template",
                "id: CVE-2025-0001\ninfo:\n  name: duplicate\nhttp:\n  - method: GET",
                &["wordpress"],
            ),
            poc(
                12,
                "nuclei",
                "nuclei_template",
                "id: ../../unsafe.yaml\ninfo:\n  name: unsafe\nhttp:\n  - method: GET",
                &["wordpress"],
            ),
            poc(
                13,
                "nuclei",
                "nuclei_template",
                &format!(
                    "id: {}\ninfo:\n  name: too-long\nhttp:\n  - method: GET",
                    "a".repeat(129)
                ),
                &["wordpress"],
            ),
        ];

        let selected = select_safe_nuclei_templates(target_id, &fingerprints, &pocs);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].template_id, "CVE-2025-0001");
        assert_eq!(selected[0].rationales.len(), 4);
        assert!(selected[0]
            .rationales
            .windows(2)
            .all(|pair| pair[0].fingerprint_id <= pair[1].fingerprint_id));
    }

    #[test]
    fn selector_rejects_imported_non_cve_template_categories() {
        let target_id = Uuid::from_u128(100);
        let fingerprints = vec![fingerprint(1, target_id, "WordPress", Some("6.5"))];
        let mut non_cve = poc(
            10,
            "nuclei",
            "nuclei_template",
            "id: wordpress-technology-detect\ninfo:\n  name: WordPress detect\nhttp:\n  - method: GET",
            &["wordpress", "tech"],
        );
        non_cve.cve_id = "NUCLEI-WORDPRESS-TECHNOLOGY-DETECT".to_string();
        let mut mismatched = poc(
            11,
            "nuclei",
            "nuclei_template",
            "id: wordpress-exposure\ninfo:\n  name: WordPress exposure\nhttp:\n  - method: GET",
            &["wordpress", "exposure"],
        );
        mismatched.cve_id = "CVE-2025-9999".to_string();

        let selected =
            select_safe_nuclei_templates(target_id, &fingerprints, &[non_cve, mismatched]);

        assert!(selected.is_empty());
    }

    #[test]
    fn selector_rejects_cve_shaped_rows_without_the_cve_tag() {
        let target_id = Uuid::from_u128(100);
        let fingerprints = vec![fingerprint(1, target_id, "WordPress", Some("6.5"))];
        let mut missing_cve_tag = poc(
            10,
            "nuclei",
            "nuclei_template",
            "id: CVE-2025-0001\ninfo:\n  name: WordPress issue\nhttp:\n  - method: GET",
            &["wordpress", "xss"],
        );
        missing_cve_tag.tags.retain(|tag| tag != "cve");

        let selected = select_safe_nuclei_templates(target_id, &fingerprints, &[missing_cve_tag]);

        assert!(selected.is_empty());
    }

    #[test]
    fn selector_bounds_fingerprint_fields_and_combined_regex_before_sql() {
        let target_id = Uuid::from_u128(100);
        let exact = vec![fingerprint(
            1,
            target_id,
            &"n".repeat(MAX_FINGERPRINT_NAME_BYTES),
            Some(&"v".repeat(MAX_FINGERPRINT_VERSION_BYTES)),
        )];
        let (terms, pattern) = bounded_search_inputs(target_id, &exact).expect("bounded input");
        assert!(!terms.is_empty());
        assert!(pattern.len() <= MAX_COMBINED_PATTERN_BYTES);

        let oversized_name = vec![fingerprint(
            1,
            target_id,
            &"n".repeat(MAX_FINGERPRINT_NAME_BYTES + 1),
            None,
        )];
        assert!(bounded_search_inputs(target_id, &oversized_name).is_err());

        let oversized_version = vec![fingerprint(
            1,
            target_id,
            "WordPress",
            Some(&"v".repeat(MAX_FINGERPRINT_VERSION_BYTES + 1)),
        )];
        assert!(bounded_search_inputs(target_id, &oversized_version).is_err());

        let pattern_amplification = (0..MAX_FINGERPRINTS)
            .map(|index| {
                fingerprint(
                    index as u128 + 1,
                    target_id,
                    &format!("{}-{index}", "[x]".repeat(64)),
                    None,
                )
            })
            .collect::<Vec<_>>();
        let error = bounded_search_inputs(target_id, &pattern_amplification).unwrap_err();
        assert!(error.contains("combined fingerprint pattern"));
    }

    #[test]
    fn query_and_id_policy_are_fail_closed() {
        assert!(NUCLEI_POC_QUERY.contains("LOWER(poc_type) = 'nuclei'"));
        assert!(NUCLEI_POC_QUERY.contains("LOWER(source) = 'nuclei_template'"));
        assert!(NUCLEI_POC_QUERY.contains("UPPER(cve_id) ~ '^CVE-[0-9]{4}-[0-9]{4,}$'"));
        assert!(NUCLEI_POC_QUERY.contains("LOWER(required_tag) = 'cve'"));
        assert!(NUCLEI_POC_QUERY.contains("LIMIT 501"));
        assert_eq!(MAX_SELECTIONS, 256);
        assert!(is_safe_nuclei_template_id("CVE-2025-0001"));
        assert!(!is_safe_nuclei_template_id("../template"));
        assert!(!is_safe_nuclei_template_id("template/path"));
        assert!(!is_safe_nuclei_template_id(&"a".repeat(129)));
        assert!(is_strict_cve_id("CVE-2025-1234"));
        assert!(is_strict_cve_id("cve-2025-123456"));
        assert!(!is_strict_cve_id("NUCLEI-WORDPRESS-DETECT"));
        assert!(!is_strict_cve_id("CVE-25-1234"));
    }

    #[test]
    fn selector_accepts_only_explicit_safe_http_or_ssl_protocol_templates() {
        assert!(is_safe_nuclei_template_content(
            "id: http-safe\ninfo:\n  name: safe\nhttp:\n  - method: GET"
        ));
        assert!(is_safe_nuclei_template_content(
            "id: legacy-http\ninfo:\n  name: safe\nrequests:\n  - method: GET"
        ));
        assert!(is_safe_nuclei_template_content(
            "id: ssl-safe\ninfo:\n  name: safe\nssl:\n  - address: '{{Host}}:{{Port}}'"
        ));
        for unsafe_content in [
            "id: code-only\ninfo:\n  name: unsafe\ncode:\n  - engine: [python3]",
            "id: tcp-only\ninfo:\n  name: unsafe\ntcp:\n  - inputs: []",
            "id: workflow\ninfo:\n  name: unsafe\nworkflows:\n  - template: child",
            "id: mixed\ninfo:\n  name: unsafe\nhttp:\n  - method: GET\nheadless:\n  - steps: []",
            "id: unknown\ninfo:\n  name: unsafe",
        ] {
            assert!(!is_safe_nuclei_template_content(unsafe_content));
        }
    }
}
