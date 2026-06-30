use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use golish_js_analyzer::{
    analyze_signals_from_files, extract_from_files, Endpoint, JsSignalReport, RuleMatchCandidate,
    RuleMatchKind, SecretCandidate, UrlKind,
};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};

const DEFAULT_ENDPOINT_LIMIT: usize = 200;
const DEFAULT_SIGNAL_LIMIT: usize = 160;
const DEFAULT_CONTEXT_LIMIT: usize = 32;
const DEFAULT_MAX_FILE_BYTES: u64 = 1_500_000;

type JsSource = (String, String);
type JsSources = Vec<JsSource>;

#[derive(Debug)]
struct Args {
    js_dir: PathBuf,
    target_url: Option<String>,
    min_confidence: f32,
    endpoint_limit: usize,
    signal_limit: usize,
    context_limit: usize,
    max_file_bytes: u64,
}

#[derive(Debug, Serialize)]
struct SkippedJsSource {
    source_file: String,
    reason: String,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct EndpointView<'a> {
    #[serde(flatten)]
    endpoint: &'a Endpoint,
    resolved_path: String,
}

#[derive(Debug, Serialize)]
struct ContextSnippet {
    source_file: String,
    absolute_path: String,
    line_start: usize,
    line_end: usize,
    reason: String,
    snippet: String,
}

fn main() -> Result<()> {
    let args = parse_args(env::args().skip(1).collect())?;
    let (sources, read_errors, skipped_js_files) =
        load_js_sources(&args.js_dir, args.max_file_bytes)?;
    if sources.is_empty() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "status": if skipped_js_files.is_empty() && read_errors.is_empty() { "empty" } else { "partial" },
                "js_dir": args.js_dir,
                "files_scanned": 0,
                "read_errors": read_errors,
                "files_skipped": skipped_js_files.len(),
                "skipped_js_files": skipped_js_files,
                "max_file_bytes": args.max_file_bytes,
            }))?
        );
        return Ok(());
    }

    let report = extract_from_files(sources.iter().map(|(p, s)| (p.as_str(), s.as_str())));
    let signals = analyze_signals_from_files(sources.iter().map(|(p, s)| (p.as_str(), s.as_str())));
    let api_base_path = detect_api_base_path(&sources);
    let filtered: Vec<&Endpoint> = report
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.confidence >= args.min_confidence)
        .collect();
    let endpoint_views: Vec<EndpointView<'_>> = filtered
        .iter()
        .take(args.endpoint_limit)
        .map(|endpoint| EndpointView {
            endpoint,
            resolved_path: endpoint_path_with_api_base(&endpoint.path, api_base_path.as_deref()),
        })
        .collect();
    let context_snippets = context_snippets(&args.js_dir, &filtered, &signals, args.context_limit);

    let output = json!({
        "status": if read_errors.is_empty() && skipped_js_files.is_empty() { "ok" } else { "partial" },
        "target_url": args.target_url,
        "js_dir": args.js_dir,
        "files_scanned": sources.len(),
        "files_skipped": skipped_js_files.len(),
        "read_errors": read_errors,
        "skipped_js_files": skipped_js_files,
        "max_file_bytes": args.max_file_bytes,
        "api_base_path": api_base_path,
        "endpoints_total": filtered.len(),
        "endpoints_unique": unique_endpoint_count(&filtered),
        "endpoints_sampled": endpoint_views.len(),
        "endpoints": endpoint_views,
        "skipped_files_total": report.skipped.len(),
        "skipped_files_sample": report.skipped.iter().take(80).collect::<Vec<_>>(),
        "secret_candidates_total": signals.secrets.len(),
        "config_candidates_total": signals.configs.len(),
        "frameworks_total": signals.frameworks.len(),
        "libraries_total": signals.libraries.len(),
        "rule_matches_total": signals.rule_matches.len(),
        "summary": {
            "by_method": method_counts(&filtered),
            "by_url_kind": url_kind_counts(&filtered),
            "by_secret_kind": secret_kind_counts(&signals.secrets),
            "by_rule_kind": rule_match_kind_counts(&signals.rule_matches),
        },
        "secret_candidates": signals.secrets.iter().take(args.signal_limit).collect::<Vec<_>>(),
        "config_candidates": signals.configs.iter().take(args.signal_limit).collect::<Vec<_>>(),
        "frameworks": signals.frameworks.iter().take(80).collect::<Vec<_>>(),
        "libraries": signals.libraries.iter().take(80).collect::<Vec<_>>(),
        "rule_matches": signals.rule_matches.iter().take(args.signal_limit).collect::<Vec<_>>(),
        "ai_review": {
            "recommended": ai_review_recommended(&filtered, &signals),
            "reasons": ai_review_reasons(&filtered, &signals),
            "candidate_files": candidate_file_summaries(&filtered, &signals),
            "context_snippets": context_snippets,
        },
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn parse_args(raw: Vec<String>) -> Result<Args> {
    let mut js_dir = None;
    let mut target_url = None;
    let mut min_confidence = 0.0_f32;
    let mut endpoint_limit = DEFAULT_ENDPOINT_LIMIT;
    let mut signal_limit = DEFAULT_SIGNAL_LIMIT;
    let mut context_limit = DEFAULT_CONTEXT_LIMIT;
    let mut max_file_bytes = DEFAULT_MAX_FILE_BYTES;

    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--js-dir" => {
                i += 1;
                js_dir = raw.get(i).map(PathBuf::from);
            }
            "--target-url" => {
                i += 1;
                target_url = raw.get(i).cloned();
            }
            "--min-confidence" => {
                i += 1;
                min_confidence = raw
                    .get(i)
                    .context("--min-confidence requires a value")?
                    .parse()
                    .context("invalid --min-confidence")?;
            }
            "--endpoint-limit" => {
                i += 1;
                endpoint_limit = parse_usize(raw.get(i), "--endpoint-limit")?;
            }
            "--signal-limit" => {
                i += 1;
                signal_limit = parse_usize(raw.get(i), "--signal-limit")?;
            }
            "--context-limit" => {
                i += 1;
                context_limit = parse_usize(raw.get(i), "--context-limit")?;
            }
            "--max-file-bytes" => {
                i += 1;
                max_file_bytes = parse_u64(raw.get(i), "--max-file-bytes")?;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
        i += 1;
    }

    let js_dir = js_dir.context("--js-dir is required")?;
    Ok(Args {
        js_dir,
        target_url,
        min_confidence,
        endpoint_limit,
        signal_limit,
        context_limit,
        max_file_bytes,
    })
}

fn parse_usize(value: Option<&String>, name: &str) -> Result<usize> {
    value
        .with_context(|| format!("{name} requires a value"))?
        .parse()
        .with_context(|| format!("invalid {name}"))
}

fn parse_u64(value: Option<&String>, name: &str) -> Result<u64> {
    value
        .with_context(|| format!("{name} requires a value"))?
        .parse()
        .with_context(|| format!("invalid {name}"))
}

fn print_help() {
    eprintln!(
        "Usage: js_api_extract --js-dir <capture-js-dir> [--target-url <url>] \
         [--min-confidence 0.0] [--endpoint-limit 200] [--signal-limit 160] \
         [--context-limit 32] [--max-file-bytes 1500000]"
    );
}

fn load_js_sources(
    js_dir: &Path,
    max_file_bytes: u64,
) -> Result<(JsSources, Vec<String>, Vec<SkippedJsSource>)> {
    if !js_dir.exists() {
        bail!("JS capture directory does not exist: {}", js_dir.display());
    }
    let mut files = Vec::new();
    collect_js_files(js_dir, js_dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut sources = Vec::new();
    let mut read_errors = Vec::new();
    let mut skipped_js_files = Vec::new();
    for (source_name, path) in files {
        let size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or_default();
        if size_bytes > max_file_bytes {
            skipped_js_files.push(SkippedJsSource {
                source_file: source_name,
                reason: "exceeds_max_file_bytes".to_string(),
                size_bytes,
            });
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(content) => sources.push((source_name, content)),
            Err(error) => read_errors.push(format!("{}: {}", path.display(), error)),
        }
    }
    Ok((sources, read_errors, skipped_js_files))
}

fn collect_js_files(root: &Path, current: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for entry in fs::read_dir(current).with_context(|| format!("read_dir {}", current.display()))? {
        let entry = entry?;
        let path = entry.path();
        let meta = entry.metadata()?;
        if meta.is_dir() {
            collect_js_files(root, &path, out)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let is_js = matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("js" | "mjs")
        );
        if !is_js {
            continue;
        }
        let source_name = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        out.push((source_name, path));
    }
    Ok(())
}

fn detect_api_base_path(sources: &[(String, String)]) -> Option<String> {
    let patterns = [
        r#"VITE_GLOB_API_URL["']?\s*:\s*["']([^"']+)["']"#,
        r#"VITE_GLOB_API_URL\s*:\s*[A-Za-z_$][A-Za-z0-9_$]*\s*=\s*["']([^"']+)["']"#,
        r#"\bapiURL\s*:\s*["']([^"']+)["']"#,
    ];

    for pattern in patterns {
        let Ok(re) = Regex::new(pattern) else {
            continue;
        };
        for (_, source) in sources {
            if let Some(value) = re
                .captures(source)
                .and_then(|cap| cap.get(1).map(|m| m.as_str()))
                .and_then(normalize_api_base_path)
            {
                return Some(value);
            }
        }
    }
    None
}

fn normalize_api_base_path(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if !trimmed.starts_with('/') || trimmed.starts_with("//") {
        return None;
    }
    let without_query = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let compact = without_query.trim_matches('/');
    if compact.is_empty() {
        None
    } else {
        Some(format!("/{compact}"))
    }
}

fn endpoint_path_with_api_base(raw: &str, api_base_path: Option<&str>) -> String {
    let Some(base_path) = api_base_path else {
        return raw.to_string();
    };
    if !raw.starts_with('/') || raw.starts_with("//") || raw.starts_with(base_path) {
        return raw.to_string();
    }

    format!(
        "{}/{}",
        base_path.trim_end_matches('/'),
        raw.trim_start_matches('/')
    )
}

fn unique_endpoint_count(endpoints: &[&Endpoint]) -> usize {
    endpoints
        .iter()
        .map(|endpoint| (endpoint.method.as_str(), endpoint.path.as_str()))
        .collect::<HashSet<_>>()
        .len()
}

fn method_counts(endpoints: &[&Endpoint]) -> Value {
    let mut counts = BTreeMap::new();
    for endpoint in endpoints {
        *counts.entry(endpoint.method.as_str()).or_insert(0usize) += 1;
    }
    json!(counts)
}

fn url_kind_counts(endpoints: &[&Endpoint]) -> Value {
    let mut counts = BTreeMap::new();
    for endpoint in endpoints {
        let key = match endpoint.url_kind {
            UrlKind::Literal => "literal",
            UrlKind::Concatenated => "concatenated",
            UrlKind::TemplateLiteral => "template_literal",
        };
        *counts.entry(key).or_insert(0usize) += 1;
    }
    json!(counts)
}

fn secret_kind_counts(secrets: &[SecretCandidate]) -> Value {
    let mut counts = BTreeMap::new();
    for secret in secrets {
        let key = serde_json::to_value(secret.kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{:?}", secret.kind));
        *counts.entry(key).or_insert(0usize) += 1;
    }
    json!(counts)
}

fn rule_match_kind_counts(rule_matches: &[RuleMatchCandidate]) -> Value {
    let mut counts = BTreeMap::new();
    for hit in rule_matches {
        let key = serde_json::to_value(hit.kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{:?}", hit.kind));
        *counts.entry(key).or_insert(0usize) += 1;
    }
    json!(counts)
}

fn ai_review_recommended(endpoints: &[&Endpoint], signals: &JsSignalReport) -> bool {
    !ai_review_reasons(endpoints, signals).is_empty()
}

fn ai_review_reasons(endpoints: &[&Endpoint], signals: &JsSignalReport) -> Vec<String> {
    let mut reasons = Vec::new();
    if !signals.secrets.is_empty() {
        reasons.push(format!(
            "{} redacted secret/sensitive candidate(s) found",
            signals.secrets.len()
        ));
    }
    if !signals.configs.is_empty() {
        reasons.push(format!(
            "{} runtime config/API base candidate(s) found",
            signals.configs.len()
        ));
    }
    if !signals.rule_matches.is_empty() {
        let review_count = signals
            .rule_matches
            .iter()
            .filter(|hit| hit.ai_review)
            .count();
        reasons.push(format!(
            "{} rule-based signal candidate(s) found; {} need AI context review",
            signals.rule_matches.len(),
            review_count
        ));
    }
    if endpoints
        .iter()
        .any(|endpoint| endpoint.confidence < 0.8 || endpoint.path.contains("${"))
    {
        reasons.push("some endpoints are low-confidence, wrapper-based, or templated".to_string());
    }
    reasons
}

fn candidate_file_summaries(endpoints: &[&Endpoint], signals: &JsSignalReport) -> Vec<Value> {
    let mut by_file: BTreeMap<String, serde_json::Map<String, Value>> = BTreeMap::new();
    for endpoint in endpoints {
        let entry = file_summary_entry(&mut by_file, &endpoint.source_file);
        increment(entry, "endpoints");
        push_line_hint(entry, endpoint.line);
    }
    for secret in &signals.secrets {
        let entry = file_summary_entry(&mut by_file, &secret.source_file);
        increment(entry, "secrets");
        push_line_hint(entry, secret.line);
    }
    for config in &signals.configs {
        let entry = file_summary_entry(&mut by_file, &config.source_file);
        increment(entry, "configs");
        push_line_hint(entry, config.line);
    }
    for hit in &signals.rule_matches {
        let entry = file_summary_entry(&mut by_file, &hit.source_file);
        increment(entry, "rule_matches");
        if hit.ai_review {
            push_line_hint(entry, hit.line);
        }
    }

    by_file.into_values().take(40).map(Value::Object).collect()
}

fn file_summary_entry<'a>(
    by_file: &'a mut BTreeMap<String, serde_json::Map<String, Value>>,
    source_file: &str,
) -> &'a mut serde_json::Map<String, Value> {
    by_file.entry(source_file.to_string()).or_insert_with(|| {
        let mut map = serde_json::Map::new();
        map.insert("source_file".to_string(), json!(source_file));
        map.insert("endpoints".to_string(), json!(0));
        map.insert("secrets".to_string(), json!(0));
        map.insert("configs".to_string(), json!(0));
        map.insert("rule_matches".to_string(), json!(0));
        map.insert("line_hints".to_string(), json!([]));
        map
    })
}

fn increment(map: &mut serde_json::Map<String, Value>, key: &str) {
    let next = map.get(key).and_then(Value::as_u64).unwrap_or(0) + 1;
    map.insert(key.to_string(), json!(next));
}

fn push_line_hint(map: &mut serde_json::Map<String, Value>, line: usize) {
    let Some(lines) = map.get_mut("line_hints").and_then(Value::as_array_mut) else {
        return;
    };
    if lines.len() >= 12 {
        return;
    }
    let hint = json!({
        "line_start": line.saturating_sub(3).max(1),
        "line_end": line + 3,
    });
    if !lines.contains(&hint) {
        lines.push(hint);
    }
}

fn context_snippets(
    js_dir: &Path,
    endpoints: &[&Endpoint],
    signals: &JsSignalReport,
    limit: usize,
) -> Vec<ContextSnippet> {
    let mut requests = Vec::new();
    for secret in &signals.secrets {
        requests.push((
            secret.source_file.clone(),
            secret.line,
            format!("secret candidate: {:?}", secret.kind),
        ));
    }
    for config in &signals.configs {
        requests.push((
            config.source_file.clone(),
            config.line,
            "runtime config candidate".to_string(),
        ));
    }
    for hit in signals.rule_matches.iter().filter(|hit| hit.ai_review) {
        requests.push((
            hit.source_file.clone(),
            hit.line,
            format!("rule match needs review: {}", rule_kind_label(hit.kind)),
        ));
    }
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| endpoint.confidence < 0.8 || endpoint.path.contains("${"))
    {
        requests.push((
            endpoint.source_file.clone(),
            endpoint.line,
            format!("endpoint candidate: {} {}", endpoint.method, endpoint.path),
        ));
    }

    let mut seen = HashSet::new();
    let mut snippets = Vec::new();
    for (source_file, line, reason) in requests {
        if snippets.len() >= limit {
            break;
        }
        let key = (source_file.clone(), line);
        if !seen.insert(key) {
            continue;
        }
        let path = js_dir.join(&source_file);
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let (line_start, line_end, snippet) = slice_lines(&source, line, 3);
        let snippet = redact_sensitive_snippet(&snippet);
        snippets.push(ContextSnippet {
            source_file,
            absolute_path: path.to_string_lossy().to_string(),
            line_start,
            line_end,
            reason,
            snippet,
        });
    }
    snippets
}

fn rule_kind_label(kind: RuleMatchKind) -> &'static str {
    match kind {
        RuleMatchKind::Route => "route",
        RuleMatchKind::Secret => "secret",
        RuleMatchKind::Config => "config",
        RuleMatchKind::Framework => "framework",
        RuleMatchKind::Pii => "pii",
        RuleMatchKind::Interesting => "interesting",
    }
}

fn slice_lines(source: &str, center_line: usize, radius: usize) -> (usize, usize, String) {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return (1, 1, String::new());
    }
    let start = center_line.saturating_sub(radius).max(1);
    let end = (center_line + radius).min(lines.len());
    let snippet = lines[start - 1..end]
        .iter()
        .enumerate()
        .map(|(idx, line)| format!("{:>6}: {}", start + idx, line))
        .collect::<Vec<_>>()
        .join("\n");
    (start, end, snippet)
}

fn redact_sensitive_snippet(snippet: &str) -> String {
    let mut redacted = snippet.to_string();
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

#[cfg(test)]
mod tests {
    use super::redact_sensitive_snippet;

    #[test]
    fn redacts_sensitive_values_from_context_snippets() {
        let snippet = r#"
             1: const accessToken = "sk_live_SECRET_REAL_ABCDEFG1234567890";
             2: const dbPassword = "P@ssw0rd!Realish98765";
             3: fetch("/api", { headers: { Authorization: "Bearer runtime-token-12345" }});
        "#;

        let redacted = redact_sensitive_snippet(snippet);

        assert!(!redacted.contains("sk_live_SECRET_REAL_ABCDEFG1234567890"));
        assert!(!redacted.contains("P@ssw0rd!Realish98765"));
        assert!(!redacted.contains("runtime-token-12345"));
        assert!(redacted.contains("***REDACTED***"));
    }
}
