//! Lightweight JavaScript signal extraction beyond API call-sites.
//!
//! This module deliberately stays deterministic and redacted: it emits source
//! file + line + hashed/previewed candidates that an outer AI agent can inspect
//! with `read_file` if needed, without dumping full secrets into prompts.

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

const JS_SIGNAL_RULES_YAML: &str =
    include_str!("../../../../resources/js-analysis/js-signal-rules.yml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    Jwt,
    BearerToken,
    ApiKey,
    CloudAccessKey,
    PrivateKey,
    BasicAuthUrl,
    Password,
    InternalUrl,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecretCandidate {
    pub kind: SecretKind,
    pub source_file: String,
    pub line: usize,
    pub key: Option<String>,
    pub value_preview: String,
    pub value_sha256: String,
    pub confidence: f32,
    pub is_likely_test_value: bool,
    pub context: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigKind {
    ApiBaseUrl,
    AuthUrl,
    UploadUrl,
    PublicPath,
    RuntimeEnv,
    InternalUrl,
    OtherUrl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigCandidate {
    pub kind: ConfigKind,
    pub source_file: String,
    pub line: usize,
    pub key: String,
    pub value_preview: String,
    pub value_sha256: String,
    pub confidence: f32,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrameworkCandidate {
    pub name: String,
    pub version: Option<String>,
    pub source_file: String,
    pub line: usize,
    pub confidence: f32,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LibraryCandidate {
    pub name: String,
    pub version: Option<String>,
    pub source_file: String,
    pub line: usize,
    pub confidence: f32,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleMatchKind {
    Secret,
    Config,
    Framework,
    Interesting,
    Pii,
    Route,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleMatchSeverity {
    High,
    Medium,
    Low,
    #[default]
    Info,
}

fn default_rule_color() -> String {
    "gray".to_string()
}

fn default_rule_scope() -> String {
    "response body".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleMatchCandidate {
    pub rule_name: String,
    pub group: String,
    pub source_rule: Option<String>,
    pub kind: RuleMatchKind,
    #[serde(default = "default_rule_color")]
    pub color: String,
    #[serde(default = "default_rule_scope")]
    pub scope: String,
    #[serde(default)]
    pub severity: RuleMatchSeverity,
    pub source_file: String,
    pub line: usize,
    pub match_preview: String,
    pub match_sha256: String,
    pub confidence: f32,
    pub ai_review: bool,
    pub context: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct JsSignalReport {
    pub secrets: Vec<SecretCandidate>,
    pub configs: Vec<ConfigCandidate>,
    pub frameworks: Vec<FrameworkCandidate>,
    pub libraries: Vec<LibraryCandidate>,
    pub rule_matches: Vec<RuleMatchCandidate>,
}

impl JsSignalReport {
    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
            && self.configs.is_empty()
            && self.frameworks.is_empty()
            && self.libraries.is_empty()
            && self.rule_matches.is_empty()
    }
}

pub fn analyze_signals_from_source(source_file: &str, source: &str) -> JsSignalReport {
    let mut report = JsSignalReport {
        secrets: extract_secrets(source_file, source),
        configs: extract_configs(source_file, source),
        frameworks: detect_frameworks(source_file, source),
        libraries: detect_libraries(source_file, source),
        rule_matches: extract_rule_matches(source_file, source),
    };

    report.secrets.sort_by_key(|c| {
        (
            c.source_file.clone(),
            c.line,
            format!("{:?}", c.kind),
            c.value_sha256.clone(),
        )
    });
    report.configs.sort_by_key(|c| {
        (
            c.source_file.clone(),
            c.line,
            c.key.clone(),
            c.value_sha256.clone(),
        )
    });
    report
        .frameworks
        .sort_by_key(|c| (c.name.clone(), c.source_file.clone(), c.line));
    report
        .libraries
        .sort_by_key(|c| (c.name.clone(), c.source_file.clone(), c.line));
    report.rule_matches.sort_by_key(|c| {
        (
            c.source_file.clone(),
            c.line,
            c.rule_name.clone(),
            c.match_sha256.clone(),
        )
    });

    report
}

pub fn analyze_signals_from_files<I, S1, S2>(files: I) -> JsSignalReport
where
    I: IntoIterator<Item = (S1, S2)>,
    S1: AsRef<str>,
    S2: AsRef<str>,
{
    let mut combined = JsSignalReport::default();
    let mut seen_secrets = HashSet::new();
    let mut seen_configs = HashSet::new();
    let mut seen_frameworks = HashSet::new();
    let mut seen_libraries = HashSet::new();
    let mut seen_rule_matches = HashSet::new();

    for (path, source) in files {
        let path_ref = path.as_ref();
        let source_ref = source.as_ref();
        let report = analyze_signals_from_source(path_ref, source_ref);

        for candidate in report.secrets {
            let key = (
                candidate.source_file.clone(),
                candidate.line,
                candidate.kind,
                candidate.value_sha256.clone(),
            );
            if seen_secrets.insert(key) {
                combined.secrets.push(candidate);
            }
        }
        for candidate in report.configs {
            let key = (
                candidate.source_file.clone(),
                candidate.line,
                candidate.key.clone(),
                candidate.value_sha256.clone(),
            );
            if seen_configs.insert(key) {
                combined.configs.push(candidate);
            }
        }
        for candidate in report.frameworks {
            let key = (candidate.source_file.clone(), candidate.name.clone());
            if seen_frameworks.insert(key) {
                combined.frameworks.push(candidate);
            }
        }
        for candidate in report.libraries {
            let key = (candidate.source_file.clone(), candidate.name.clone());
            if seen_libraries.insert(key) {
                combined.libraries.push(candidate);
            }
        }
        for candidate in report.rule_matches {
            let key = (
                candidate.source_file.clone(),
                candidate.line,
                candidate.rule_name.clone(),
                candidate.match_sha256.clone(),
            );
            if seen_rule_matches.insert(key) {
                combined.rule_matches.push(candidate);
            }
        }
    }

    combined
}

#[derive(Debug, Deserialize)]
struct SignalRuleSet {
    rules: Vec<SignalRuleDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
struct SignalRuleDefinition {
    name: String,
    group: String,
    source_rule: Option<String>,
    kind: RuleMatchKind,
    #[serde(default)]
    color: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    severity: Option<RuleMatchSeverity>,
    regex: String,
    #[serde(default = "default_true")]
    case_sensitive: bool,
    confidence: f32,
    #[serde(default)]
    ai_review: bool,
}

fn default_true() -> bool {
    true
}

fn signal_rules() -> Vec<SignalRuleDefinition> {
    let ruleset: SignalRuleSet =
        serde_yaml::from_str(JS_SIGNAL_RULES_YAML).expect("embedded JS signal rules YAML is valid");
    ruleset.rules
}

fn extract_rule_matches(source_file: &str, source: &str) -> Vec<RuleMatchCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for rule in signal_rules() {
        let re = RegexBuilder::new(&rule.regex)
            .case_insensitive(!rule.case_sensitive)
            .multi_line(true)
            .build()
            .unwrap_or_else(|e| panic!("embedded JS signal regex `{}` is valid: {e}", rule.name));

        for cap in re.captures_iter(source) {
            let value_match = cap.get(1).or_else(|| cap.get(0));
            let Some(value_match) = value_match else {
                continue;
            };
            let value = value_match.as_str().trim();
            if value.is_empty() {
                continue;
            }

            let hash = sha256_hex(value);
            let line = line_of(source, value_match.start());
            let dedupe_key = (rule.name.clone(), hash.clone(), line);
            if !seen.insert(dedupe_key) {
                continue;
            }
            let color = rule_color(&rule);
            let severity = rule
                .severity
                .unwrap_or_else(|| rule_severity(&color, rule.kind, rule.confidence));

            out.push(RuleMatchCandidate {
                rule_name: rule.name.clone(),
                group: rule.group.clone(),
                source_rule: rule.source_rule.clone(),
                kind: rule.kind,
                color,
                scope: rule_scope(&rule),
                severity,
                source_file: source_file.to_string(),
                line,
                match_preview: rule_match_preview(rule.kind, value),
                match_sha256: hash,
                confidence: rule.confidence,
                ai_review: rule.ai_review,
                context: redacted_line_context(source, value_match.start(), value),
            });
        }
    }

    out
}

fn rule_color(rule: &SignalRuleDefinition) -> String {
    if !rule.color.trim().is_empty() {
        return rule.color.clone();
    }
    match rule.kind {
        RuleMatchKind::Secret => "yellow",
        RuleMatchKind::Pii => "orange",
        RuleMatchKind::Framework => "green",
        RuleMatchKind::Config => "cyan",
        RuleMatchKind::Interesting => "yellow",
        RuleMatchKind::Route => "gray",
    }
    .to_string()
}

fn rule_scope(rule: &SignalRuleDefinition) -> String {
    if rule.scope.trim().is_empty() {
        "response body".to_string()
    } else {
        rule.scope.clone()
    }
}

fn rule_severity(color: &str, kind: RuleMatchKind, confidence: f32) -> RuleMatchSeverity {
    match color {
        "red" => RuleMatchSeverity::High,
        "orange" | "yellow" if matches!(kind, RuleMatchKind::Secret | RuleMatchKind::Pii) => {
            RuleMatchSeverity::Medium
        }
        "orange" => RuleMatchSeverity::Medium,
        "yellow" if confidence >= 0.75 => RuleMatchSeverity::Medium,
        _ if matches!(kind, RuleMatchKind::Secret) && confidence >= 0.85 => {
            RuleMatchSeverity::Medium
        }
        _ => RuleMatchSeverity::Low,
    }
}

fn extract_secrets(source_file: &str, source: &str) -> Vec<SecretCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let assignment_re = Regex::new(
        r#"(?im)["']?([A-Za-z_$][A-Za-z0-9_$-]*)["']?\s*[:=]\s*["'`]([^"'`\n]{6,})["'`]"#,
    )
    .expect("secret assignment regex valid");
    for cap in assignment_re.captures_iter(source) {
        let Some(key_match) = cap.get(1) else {
            continue;
        };
        let Some(value_match) = cap.get(2) else {
            continue;
        };
        let key = key_match.as_str();
        let value = value_match.as_str();
        let kind = secret_kind_for_key(key);
        if kind == SecretKind::Unknown {
            continue;
        }
        if !should_keep_secret_value(value) {
            continue;
        }
        push_secret(
            &mut out,
            &mut seen,
            SecretBuild {
                kind,
                source_file,
                source,
                offset: value_match.start(),
                key: Some(key.to_string()),
                value,
                confidence: confidence_for_secret(kind, value),
            },
        );
    }

    let patterns: [(SecretKind, &str, f32); 6] = [
        (
            SecretKind::Jwt,
            r#"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b"#,
            0.9,
        ),
        (
            SecretKind::CloudAccessKey,
            r#"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"#,
            0.92,
        ),
        (
            SecretKind::BearerToken,
            r#"(?i)\bBearer\s+([A-Za-z0-9._~+/=-]{16,})"#,
            0.82,
        ),
        (
            SecretKind::PrivateKey,
            r#"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----"#,
            0.98,
        ),
        (
            SecretKind::BasicAuthUrl,
            r#"https?://[^/\s"'`:@]+:[^@\s"'`]+@[^/\s"'`)]+"#,
            0.86,
        ),
        (
            SecretKind::InternalUrl,
            r#"(?i)\bhttps?://(?:localhost|127\.0\.0\.1|10\.\d{1,3}\.\d{1,3}\.\d{1,3}|192\.168\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[0-1])\.\d{1,3}\.\d{1,3}|[A-Za-z0-9.-]+\.(?:internal|local|corp|lan))(?::\d+)?(?:/[^\s"'`<>)]*)?"#,
            0.72,
        ),
    ];

    for (kind, pattern, confidence) in patterns {
        let re = Regex::new(pattern).expect("secret regex valid");
        for cap in re.captures_iter(source) {
            let value_match = cap.get(1).or_else(|| cap.get(0));
            let Some(value_match) = value_match else {
                continue;
            };
            let value = value_match.as_str();
            if kind != SecretKind::PrivateKey && !should_keep_secret_value(value) {
                continue;
            }
            push_secret(
                &mut out,
                &mut seen,
                SecretBuild {
                    kind,
                    source_file,
                    source,
                    offset: value_match.start(),
                    key: None,
                    value,
                    confidence,
                },
            );
        }
    }

    out
}

struct SecretBuild<'a> {
    kind: SecretKind,
    source_file: &'a str,
    source: &'a str,
    offset: usize,
    key: Option<String>,
    value: &'a str,
    confidence: f32,
}

fn push_secret(
    out: &mut Vec<SecretCandidate>,
    seen: &mut HashSet<(SecretKind, String, usize)>,
    build: SecretBuild<'_>,
) {
    let hash = sha256_hex(build.value);
    let line = line_of(build.source, build.offset);
    if !seen.insert((build.kind, hash.clone(), line)) {
        return;
    }
    let is_likely_test_value = is_likely_test_value(build.value);
    let confidence = if is_likely_test_value {
        (build.confidence - 0.28).max(0.2)
    } else {
        build.confidence
    };
    out.push(SecretCandidate {
        kind: build.kind,
        source_file: build.source_file.to_string(),
        line,
        key: build.key,
        value_preview: redacted_preview(build.value),
        value_sha256: hash,
        confidence,
        is_likely_test_value,
        context: redacted_line_context(build.source, build.offset, build.value),
    });
}

fn extract_configs(source_file: &str, source: &str) -> Vec<ConfigCandidate> {
    let config_re = Regex::new(
        r#"(?im)["']?([A-Za-z_$][A-Za-z0-9_$]*)["']?\s*[:=]\s*["'`]([^"'`\n]{1,220})["'`]"#,
    )
    .expect("config regex valid");

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for cap in config_re.captures_iter(source) {
        let Some(key_match) = cap.get(1) else {
            continue;
        };
        let Some(value_match) = cap.get(2) else {
            continue;
        };
        let key = key_match.as_str();
        let value = value_match.as_str().trim();
        if value.is_empty() || secret_kind_for_key(key) != SecretKind::Unknown {
            continue;
        }
        if !is_config_key(key) {
            continue;
        }
        if !looks_like_config_value(value) {
            continue;
        }

        let hash = sha256_hex(value);
        let line = line_of(source, value_match.start());
        let dedupe_key = (key.to_ascii_lowercase(), hash.clone(), line);
        if !seen.insert(dedupe_key) {
            continue;
        }
        out.push(ConfigCandidate {
            kind: config_kind_for_key_value(key, value),
            source_file: source_file.to_string(),
            line,
            key: key.to_string(),
            value_preview: config_preview(value),
            value_sha256: hash,
            confidence: config_confidence(value),
            context: redacted_line_context(source, value_match.start(), value),
        });
    }

    out
}

fn detect_frameworks(source_file: &str, source: &str) -> Vec<FrameworkCandidate> {
    let rules = [
        (
            "Next.js",
            &["__NEXT_DATA__", "_next/static", "self.__next_f"][..],
            0.93,
        ),
        ("Nuxt", &["__NUXT__", "_nuxt/", "nuxtApp"][..], 0.9),
        (
            "Vite",
            &["__vite__mapDeps", "import.meta.env", "/@vite/"][..],
            0.85,
        ),
        (
            "Webpack",
            &["__webpack_require__", "webpackChunk"][..],
            0.82,
        ),
        (
            "React",
            &["React.createElement", "react-dom", "jsx-runtime"][..],
            0.84,
        ),
        ("Vue", &["Vue.createApp", "__VUE__", "vue-router"][..], 0.84),
        (
            "Angular",
            &["ɵɵdefineComponent", "ng-version", "@angular/core"][..],
            0.86,
        ),
        (
            "Svelte",
            &["svelte/internal", "__sveltekit", "sveltekit"][..],
            0.84,
        ),
    ];
    detect_named(source_file, source, &rules)
        .into_iter()
        .map(|(name, line, confidence, evidence)| FrameworkCandidate {
            name,
            version: None,
            source_file: source_file.to_string(),
            line,
            confidence,
            evidence,
        })
        .collect()
}

fn detect_libraries(source_file: &str, source: &str) -> Vec<LibraryCandidate> {
    let rules = [
        ("axios", &["axios.", "axios("][..], 0.86),
        ("jQuery", &["jQuery.", "$.ajax"][..], 0.82),
        ("ECharts", &["echarts.", "echarts.init"][..], 0.8),
        ("Lodash", &["lodash", "_.debounce", "_.merge"][..], 0.72),
        ("dayjs", &["dayjs("][..], 0.72),
        ("moment", &["moment("][..], 0.72),
        ("Pinia", &["defineStore(", "pinia"][..], 0.76),
        ("React Router", &["react-router", "useNavigate("][..], 0.76),
        ("Element Plus", &["element-plus", "ElMessage"][..], 0.76),
        ("Ant Design", &["antd/", "ant-design", "Antd"][..], 0.74),
    ];
    detect_named(source_file, source, &rules)
        .into_iter()
        .map(|(name, line, confidence, evidence)| LibraryCandidate {
            name,
            version: None,
            source_file: source_file.to_string(),
            line,
            confidence,
            evidence,
        })
        .collect()
}

fn detect_named(
    source_file: &str,
    source: &str,
    rules: &[(&str, &[&str], f32)],
) -> Vec<(String, usize, f32, String)> {
    let mut by_name = BTreeMap::new();
    let lower = source.to_ascii_lowercase();
    for (name, needles, confidence) in rules {
        for needle in *needles {
            let needle_lower = needle.to_ascii_lowercase();
            if let Some(offset) = lower.find(&needle_lower) {
                let line = line_of(source, offset);
                by_name.entry((*name).to_string()).or_insert_with(|| {
                    (
                        (*name).to_string(),
                        line,
                        *confidence,
                        format!("matched `{}` in {}", needle, source_file),
                    )
                });
                break;
            }
        }
    }
    by_name.into_values().collect()
}

fn secret_kind_for_key(key: &str) -> SecretKind {
    let lower = key.to_ascii_lowercase().replace(['-', '_'], "");
    if lower.contains("apikey") || lower.contains("xapikey") {
        SecretKind::ApiKey
    } else if lower.contains("accesstoken")
        || lower.contains("refreshtoken")
        || lower.contains("authtoken")
    {
        SecretKind::BearerToken
    } else if lower.contains("clientsecret") || lower.contains("secret") {
        SecretKind::ApiKey
    } else if lower.contains("password") || lower.contains("passwd") || lower.ends_with("pwd") {
        SecretKind::Password
    } else {
        SecretKind::Unknown
    }
}

fn confidence_for_secret(kind: SecretKind, value: &str) -> f32 {
    match kind {
        SecretKind::Jwt | SecretKind::CloudAccessKey | SecretKind::PrivateKey => 0.92,
        SecretKind::BearerToken | SecretKind::ApiKey => {
            if value.len() >= 24 {
                0.82
            } else {
                0.68
            }
        }
        SecretKind::Password => 0.62,
        SecretKind::BasicAuthUrl => 0.86,
        SecretKind::InternalUrl => 0.72,
        SecretKind::Unknown => 0.5,
    }
}

fn should_keep_secret_value(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() >= 8 && !trimmed.chars().all(|c| c.is_ascii_digit())
}

fn looks_like_config_value(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("http://")
        || value.starts_with("https://")
        || value.contains("${")
        || value.contains("%")
}

fn is_config_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.contains("API_URL")
        || upper.contains("BASE_URL")
        || upper.contains("BASE_API")
        || upper.contains("SERVER_URL")
        || upper.contains("SERVICE_URL")
        || upper.contains("AUTH_URL")
        || upper.contains("LOGIN_URL")
        || upper.contains("UPLOAD_URL")
        || upper.contains("PUBLIC_PATH")
        || upper.contains("ASSET_URL")
        || upper.contains("ENDPOINT")
        || (upper.starts_with("VITE_") && upper.contains("URL"))
        || (upper.starts_with("REACT_APP_") && upper.contains("URL"))
        || (upper.starts_with("NEXT_PUBLIC_") && upper.contains("URL"))
}

fn config_kind_for_key_value(key: &str, value: &str) -> ConfigKind {
    let lower_key = key.to_ascii_lowercase();
    let lower_value = value.to_ascii_lowercase();
    if is_internal_url(value) {
        ConfigKind::InternalUrl
    } else if lower_key.contains("upload") {
        ConfigKind::UploadUrl
    } else if lower_key.contains("auth") || lower_key.contains("login") {
        ConfigKind::AuthUrl
    } else if lower_key.contains("public") || lower_key.contains("asset") {
        ConfigKind::PublicPath
    } else if lower_key.contains("api")
        || lower_key.contains("base")
        || lower_value.contains("/api")
    {
        ConfigKind::ApiBaseUrl
    } else if lower_key.starts_with("vite_")
        || lower_key.starts_with("react_app_")
        || lower_key.starts_with("next_public_")
    {
        ConfigKind::RuntimeEnv
    } else {
        ConfigKind::OtherUrl
    }
}

fn config_confidence(value: &str) -> f32 {
    if value.starts_with("http://") || value.starts_with("https://") || value.starts_with('/') {
        0.84
    } else {
        0.62
    }
}

fn is_internal_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("://localhost")
        || lower.contains("://127.")
        || lower.contains("://10.")
        || lower.contains("://192.168.")
        || lower.contains(".internal")
        || lower.contains(".local")
        || lower.contains(".corp")
        || lower.contains(".lan")
}

fn is_likely_test_value(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let markers = [
        "example",
        "changeme",
        "placeholder",
        "dummy",
        "mock",
        "fake",
        "your_",
        "test",
        "xxxx",
        "123456",
        "abcdef",
    ];
    markers.iter().any(|marker| lower.contains(marker)) || repeated_char_ratio(value) > 0.75
}

fn repeated_char_ratio(value: &str) -> f32 {
    let total = value.chars().count();
    if total == 0 {
        return 0.0;
    }
    let mut counts = BTreeMap::new();
    for ch in value.chars() {
        *counts.entry(ch).or_insert(0usize) += 1;
    }
    let max = counts.into_values().max().unwrap_or(0);
    max as f32 / total as f32
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn redacted_preview(value: &str) -> String {
    let trimmed = value.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 8 {
        return "***".to_string();
    }
    let prefix: String = chars.iter().take(4).collect();
    let suffix: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

fn config_preview(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= 120 {
        trimmed.to_string()
    } else {
        let prefix: String = trimmed.chars().take(117).collect();
        format!("{prefix}...")
    }
}

fn rule_match_preview(kind: RuleMatchKind, value: &str) -> String {
    match kind {
        RuleMatchKind::Secret | RuleMatchKind::Pii => redacted_preview(value),
        RuleMatchKind::Config
        | RuleMatchKind::Framework
        | RuleMatchKind::Interesting
        | RuleMatchKind::Route => config_preview(value),
    }
}

fn redacted_line_context(source: &str, offset: usize, value: &str) -> String {
    let line = line_text(source, offset);
    let redacted = line.replace(value, &redacted_preview(value));
    redact_sensitive_context(&collapse_ws(&redacted))
}

fn redact_sensitive_context(context: &str) -> String {
    let mut redacted = context.to_string();
    let assignment_patterns = [
        r#"(?i)(["']?[A-Za-z0-9_.-]*(?:api[-_]?key|access[-_]?token|auth[-_]?token|token|secret|password|passwd|pwd)["']?\s*[:=]\s*["'`])([^"'`\n]{4,})(["'`])"#,
        r#"(?i)(authorization["']?\s*[:=]\s*["'`]\s*(?:bearer|basic)\s+)([^"'`\n]{4,})(["'`])"#,
    ];
    for pattern in assignment_patterns {
        let re = Regex::new(pattern).expect("redaction regex is valid");
        redacted = re.replace_all(&redacted, "$1***REDACTED***$3").to_string();
    }

    let token_patterns = [
        r#"(?i)\b(bearer|basic)\s+[A-Za-z0-9_.=:+/\-]{5,160}"#,
        r#"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9._/-]{10,}(?:\.[A-Za-z0-9._/-]{8,})?\b"#,
        r#"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"#,
        r#"\bLTAI[a-zA-Z0-9]{12,20}\b"#,
        r#"\bsk_(?:live|test)_[A-Za-z0-9_-]{8,}\b"#,
    ];
    for pattern in token_patterns {
        let re = Regex::new(pattern).expect("redaction regex is valid");
        redacted = re.replace_all(&redacted, "***REDACTED***").to_string();
    }

    redacted
}

fn line_text(source: &str, offset: usize) -> &str {
    let start = source[..offset.min(source.len())]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let end = source[offset.min(source.len())..]
        .find('\n')
        .map(|idx| offset.min(source.len()) + idx)
        .unwrap_or(source.len());
    &source[start..end]
}

fn collapse_ws(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= 180 {
        compact
    } else {
        let prefix: String = compact.chars().take(177).collect();
        format!("{prefix}...")
    }
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_redacted_secret_and_config_candidates() {
        let source = r#"
            window._CONF_ = {"VITE_GLOB_API_URL":"/admin-api"};
            const accessToken = "sk_live_1234567890abcdefXYZ";
            const internal = "http://10.0.0.5:8080/admin";
        "#;

        let report = analyze_signals_from_source("app.js", source);

        assert_eq!(report.configs.len(), 1);
        assert_eq!(report.configs[0].key, "VITE_GLOB_API_URL");
        assert_eq!(report.configs[0].kind, ConfigKind::ApiBaseUrl);
        assert_eq!(report.configs[0].value_preview, "/admin-api");

        assert!(report
            .secrets
            .iter()
            .any(|c| c.kind == SecretKind::BearerToken && c.key.as_deref() == Some("accessToken")));
        assert!(report
            .secrets
            .iter()
            .any(|c| c.kind == SecretKind::InternalUrl));
        assert!(report
            .secrets
            .iter()
            .all(|c| !c.context.contains("sk_live_1234567890abcdefXYZ")));
    }

    #[test]
    fn detects_frameworks_and_common_libraries() {
        let source = r#"
            self.__next_f = [];
            const app = React.createElement("div");
            axios.post('/api/login');
            echarts.init(document.getElementById('chart'));
        "#;

        let report = analyze_signals_from_source("chunk.js", source);

        assert!(report.frameworks.iter().any(|c| c.name == "Next.js"));
        assert!(report.frameworks.iter().any(|c| c.name == "React"));
        assert!(report.libraries.iter().any(|c| c.name == "axios"));
        assert!(report.libraries.iter().any(|c| c.name == "ECharts"));
    }

    #[test]
    fn flags_likely_test_values_without_dropping_them() {
        let source = r#"const apiKey = "example-api-key-xxxx";"#;
        let report = analyze_signals_from_source("test.js", source);

        assert_eq!(report.secrets.len(), 1);
        assert!(report.secrets[0].is_likely_test_value);
        assert!(report.secrets[0].confidence < 0.7);
    }

    #[test]
    fn embedded_signal_rules_parse_and_compile() {
        let rules = signal_rules();
        assert!(rules.len() >= 35);

        for rule in rules {
            RegexBuilder::new(&rule.regex)
                .case_insensitive(!rule.case_sensitive)
                .multi_line(true)
                .build()
                .unwrap_or_else(|e| panic!("rule `{}` should compile: {e}", rule.name));
        }
    }

    #[test]
    fn extracts_rule_matches_for_ai_review() {
        let source = r#"
            fetch("/api/users");
            const auth = "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature123";
            const swaggerVersion = "3.0";
            const vite = "/@vite/client";
            const ip = "http://192.168.1.20/admin";
        "#;

        let report = analyze_signals_from_source("bundle.js", source);

        assert!(report
            .rule_matches
            .iter()
            .any(|c| c.rule_name == "Authorization Header" && c.kind == RuleMatchKind::Secret));
        assert!(report
            .rule_matches
            .iter()
            .any(|c| c.rule_name == "Swagger UI" && c.kind == RuleMatchKind::Framework));
        assert!(report
            .rule_matches
            .iter()
            .any(|c| c.rule_name == "Vite DevMode" && c.ai_review));
        assert!(report
            .rule_matches
            .iter()
            .any(|c| c.rule_name == "Internal IP Address" && c.kind == RuleMatchKind::Config));
    }

    #[test]
    fn hae_style_rules_emit_classification_metadata() {
        let source = r#"
            const shiroCookie = "rememberMe=abc";
            const druidTitle = "Druid Stat Index";
            const idcard = "110105199001011234";
            const mac = "aa:bb:cc:dd:ee:ff";
            const winPath = "C:\Windows\win.ini";
            router.$router.push('/admin');
            const redirect = "Location: /login";
            const oskey = "<Key>abcdef</Key>";
        "#;

        let report = analyze_signals_from_source("hae.js", source);

        let shiro = report
            .rule_matches
            .iter()
            .find(|hit| hit.rule_name == "Shiro RememberMe")
            .expect("Shiro fingerprint");
        assert_eq!(shiro.group, "Fingerprint");
        assert_eq!(shiro.color, "green");
        assert_eq!(shiro.scope, "any header");
        assert_eq!(shiro.severity, RuleMatchSeverity::Low);

        let druid = report
            .rule_matches
            .iter()
            .find(|hit| hit.rule_name == "Druid")
            .expect("Druid fingerprint");
        assert_eq!(druid.severity, RuleMatchSeverity::Medium);
        assert_eq!(druid.color, "orange");

        assert!(report.rule_matches.iter().any(|hit| {
            hit.rule_name == "Chinese IDCard"
                && hit.kind == RuleMatchKind::Pii
                && hit.severity == RuleMatchSeverity::Medium
        }));
        assert!(report
            .rule_matches
            .iter()
            .any(|hit| hit.rule_name == "MAC Address" && hit.color == "green"));
        assert!(report
            .rule_matches
            .iter()
            .any(|hit| hit.rule_name == "Windows File Or Dir Path"));
        assert!(report.rule_matches.iter().any(|hit| {
            hit.rule_name == "Router Push"
                && hit.kind == RuleMatchKind::Route
                && hit.color == "magenta"
        }));
        assert!(report
            .rule_matches
            .iter()
            .any(|hit| hit.rule_name == "302 Location" && hit.scope == "response header"));
        assert!(report
            .rule_matches
            .iter()
            .any(|hit| hit.rule_name == "OSKeys" && hit.severity == RuleMatchSeverity::Info));
    }

    #[test]
    fn rule_match_sensitive_context_is_redacted() {
        let secret = "Bearer super-secret-token-abcdef";
        let source = format!(r#"const headers = {{ Authorization: "{secret}" }};"#);
        let report = analyze_signals_from_source("auth.js", &source);

        let hit = report
            .rule_matches
            .iter()
            .find(|c| c.rule_name == "Authorization Header")
            .expect("authorization header hit");

        assert_eq!(hit.kind, RuleMatchKind::Secret);
        assert!(!hit.match_preview.contains("super-secret-token"));
        assert!(!hit.context.contains("super-secret-token"));
    }

    #[test]
    fn rule_match_context_redacts_neighboring_sensitive_values() {
        let source = r#"fetch('/api/users?page=1', { headers: { Authorization: 'Bearer runtime-token-12345' }});"#;
        let report = analyze_signals_from_source("bundle.js", source);

        assert!(report
            .rule_matches
            .iter()
            .any(|hit| hit.rule_name == "Pagination Or Size Parameter"));
        assert!(report
            .rule_matches
            .iter()
            .all(|hit| !hit.context.contains("runtime-token-12345")));
    }
}
