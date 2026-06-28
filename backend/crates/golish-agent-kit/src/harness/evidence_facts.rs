//! PR2 (设计 2026-06-11-coverage-auto-derive-from-evidence §5.3) · 证据事实标注.
//!
//! 把「被动情报工具的一次运行」确定性地解析为 `(technique, asset)` + outcome,
//! 供 evidence 落库时写进 `audit_log.evidence_technique / evidence_asset /
//! evidence_outcome` 三个 nullable 列 — coverage 矩阵 (PR3) 只从这些事实投影,
//! 模型不再手写矩阵.
//!
//! 完整性约束 (设计 §4, 必须保守):
//! - **只收录无歧义映射**: 解析不出工具或资产 → `None`, 该行不打标、不参与投影
//!   (绝不猜) — 缺事实的格保持 not_attempted, gate 照旧 BLOCK (fail-closed).
//! - **empty 判定高置信**: "跑了→空" 是 I8 的数据层兑现, 只在输出形态确定为
//!   无结果时返 `Empty`; 拿不准 → `Found` (假 found 被 coverage_corroborated
//!   兜住 — found 格必须有对齐 claim/finding 佐证; 假 empty 没有这层网).
//! - technique id 必须是 `technique_taxonomy.json` 注册的 `GOLISH-INTEL-*`.

/// Phase 2 (2026-06-12-redteam-phase2): 子公司 / org 树发现 technique id
/// (scoping 阶段维度; DB 端同名常量是 `golish_db::repo::coverage_truth::TECH_SUBSIDIARY`).
pub const TECH_SUBSIDIARY: &str = "GOLISH-INTEL-SUBSIDIARY";
pub const TECH_EAS_LIVENESS: &str = "GOLISH-EAS-LIVENESS";
pub const TECH_EAS_PORT: &str = "GOLISH-EAS-PORT";
pub const TECH_EAS_SERVICE_FINGERPRINT: &str = "GOLISH-EAS-SERVICE-FINGERPRINT";

/// Coverage gate join key for URL endpoint liveness.
///
/// This intentionally mirrors the gate's asset join semantics for endpoint
/// assets: erase the scheme and casing, but keep port/path. Host-level helpers
/// such as `canonical_asset_key` are still correct for PORT/SERVICE, but they
/// collapse `http://host:90` to `host`, which cannot close a URL:port liveness
/// cell.
pub fn eas_liveness_asset_key(value: &str) -> Option<String> {
    let key = coverage_join_asset_key(value);
    if key.is_empty() {
        return None;
    }
    let host = key.split(['/', '?', '#']).next().unwrap_or(key.as_str());
    let host = strip_port(host);
    if host.is_empty() {
        return None;
    }
    (host.parse::<std::net::IpAddr>().is_ok() || host.contains('.')).then_some(key)
}

fn coverage_join_asset_key(value: &str) -> String {
    let lowered = value.trim().to_ascii_lowercase();
    let no_scheme = match lowered.split_once("://") {
        Some((scheme, rest))
            if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphabetic()) =>
        {
            rest
        }
        _ => lowered.as_str(),
    };
    no_scheme.trim_end_matches('.').to_string()
}

/// Phase 2 · 解析 `recon_discover_subsidiaries` 的结构化 JSON summary →
/// SUBSIDIARY 维度事实 `(technique, asset=公司名, outcome)`.
///
/// I8 数据层兑现 (「跑了→0 合格子」≠「没跑」):
/// - `promoted_children > 0` → `found` (child org 真落库; DB 真值投影会再补强).
/// - `promoted_children == 0` 且 `status == "Completed"` → `empty` (高置信:
///   provider 全部跑完、没有候选清过持股阈值/存续筛).
/// - `Partial` / `Failed` / 字段缺失 → `None` (拿不准不派生, fail-closed —
///   缺事实的格保持 not_attempted, gate 照旧 BLOCK).
///
/// asset 是公司名 (org 级维度的主体是公司, 不是 in-scope 主机); gate 端
/// (`fetch_evidence_facts_for_gate`) 把 SUBSIDIARY 事实展开投影到每个 in-scope
/// asset, 与 coverage_truth 的 `has_subsidiary` org 级投影同构.
pub fn subsidiary_discovery_facts(
    result: &serde_json::Value,
) -> Option<(&'static str, String, &'static str)> {
    let company = result.get("company")?.as_str()?.trim();
    if company.is_empty() {
        return None;
    }
    let promoted = result.get("promoted_children")?.as_u64()?;
    if promoted > 0 {
        return Some((TECH_SUBSIDIARY, company.to_string(), "found"));
    }
    let status = result.get("status")?.as_str()?;
    (status == "Completed").then(|| (TECH_SUBSIDIARY, company.to_string(), "empty"))
}

/// #6 (设计 2026-06-23-expansion-queue): 一条「待扩展线索」(expansion_queue 入队用).
#[derive(Debug, Clone, PartialEq)]
pub struct ExpansionLead {
    /// new_domain / brand / app / github_org / subsidiary / email_domain.
    pub lead_type: &'static str,
    pub lead_value: String,
    pub confidence: Option<f32>,
}

/// #6 · 从 `recon_discover_subsidiaries` 的 JSON summary 抽「待扩展线索」——
/// auto_promote OFF 时 surfaced 的子公司候选 (`subsidiaries[]`, 每个 `name` +
/// `meets_threshold`). 每候选 = 一条 `subsidiary` pending 线索; `meets_threshold`
/// → 高置信 0.9, 否则中置信 0.5. auto_promote ON 时 `subsidiaries` 为空 (候选已升
/// child org), 返回空. 解析不出 `name` / 空名的候选跳过 (保守, 不入噪声线索).
pub fn expansion_leads_from_subsidiary_discovery(result: &serde_json::Value) -> Vec<ExpansionLead> {
    let mut leads = Vec::new();
    let Some(subs) = result.get("subsidiaries").and_then(|s| s.as_array()) else {
        return leads;
    };
    for cand in subs {
        let Some(name) = cand.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let meets = cand
            .get("meets_threshold")
            .and_then(|m| m.as_bool())
            .unwrap_or(false);
        leads.push(ExpansionLead {
            lead_type: "subsidiary",
            lead_value: name.to_string(),
            confidence: Some(if meets { 0.9 } else { 0.5 }),
        });
    }
    leads
}

/// 解析一条被动情报命令行 → `(注册 technique id, 主资产)`.
///
/// 覆盖 target_intel 灰度 (D-scope) 的常见被动工具; 主动扫描工具 (nmap/httpx…)
/// 与解析不出资产的命令一律 `None`. `command` 形如 `"dig moresec.cn A +short"`
/// (run_pty_cmd / background_job 的 shell 命令, 或 pentest_run 的 `"{tool} {args}"`).
pub fn passive_intel_facts_from_command(command: &str) -> Option<(&'static str, String)> {
    // The executable may be a QUOTED absolute path that contains a space: on
    // macOS golish-managed tools live under `~/Library/Application Support/…`,
    // so a backgrounded run's command is e.g.
    // `"/…/Application Support/…/subfinder" -d host`. A bare `split_whitespace`
    // breaks "Application Support" apart and mis-reads the tool name as
    // "Application" → no fact → subfinder's subdomains never become a SUBDOMAIN
    // coverage fact (while bare `/usr/bin/dig|whois` slipped through). Split off
    // a leading double-quoted path first, then fall back to whitespace.
    let (exe, rest_str) = split_executable(command.trim());
    // 路径形式 (`/usr/bin/dig`) 归一到裸名.
    let tool = exe.rsplit('/').next().unwrap_or(exe);
    let rest: Vec<&str> = rest_str.split_whitespace().collect();
    match tool {
        "dig" | "nslookup" | "host" => first_domain_like(&rest).map(|d| ("GOLISH-INTEL-DNS", d)),
        "whois" => first_domain_like(&rest).map(|d| ("GOLISH-INTEL-WHOIS", d)),
        "subfinder" => flag_value(&rest, "-d")
            .or_else(|| flag_value(&rest, "-domain"))
            .map(|d| ("GOLISH-INTEL-SUBDOMAIN", d)),
        // ctfr enumerates subdomains from certificate-transparency logs (crt.sh);
        // a run is the GOLISH-INTEL-CT technique fact for the queried domain — the
        // dedicated CT producer so that column stops reading "never attempted".
        "ctfr" => flag_value(&rest, "-d")
            .or_else(|| flag_value(&rest, "--domain"))
            .map(|d| ("GOLISH-INTEL-CT", d)),
        // asnmap maps a domain/IP/org to its ASN + netblocks via public RIR data;
        // a run is the GOLISH-INTEL-ASN technique fact. Subject is the queried
        // domain (-d), IP (-i), or a bare domain/IP token (ASN-number input has
        // no `.` so it derives nothing — conservative, fail-closed).
        "asnmap" => flag_value(&rest, "-d")
            .or_else(|| flag_value(&rest, "-i"))
            .or_else(|| flag_value(&rest, "--domain"))
            .or_else(|| first_domain_like(&rest))
            .map(|d| ("GOLISH-INTEL-ASN", d)),
        _ => None,
    }
}

/// Parse a command line into any coverage fact the deterministic gate can
/// project. Passive intel mappings stay in [`passive_intel_facts_from_command`];
/// active EAS mappings live here so scan stages can also turn real tool runs
/// into `(asset, technique, outcome)` ledger facts.
pub fn coverage_facts_from_command(command: &str) -> Option<(&'static str, String)> {
    passive_intel_facts_from_command(command).or_else(|| eas_facts_from_command(command))
}

fn eas_facts_from_command(command: &str) -> Option<(&'static str, String)> {
    let (tool, rest) = command_tool_and_rest(command.trim())?;
    let rest_tokens: Vec<&str> = rest.split_whitespace().collect();
    match tool.as_str() {
        "httpx" => target_from_flags(
            &rest_tokens,
            &["-u", "-target"],
            normalize_liveness_target_token,
        )
        .or_else(|| first_target_like(&rest_tokens, normalize_liveness_target_token))
        .map(|asset| (TECH_EAS_LIVENESS, asset)),
        "nmap" => {
            let asset = first_target_like(&rest_tokens, normalize_target_token)?;
            if has_flag(&rest_tokens, &["-sn", "-sP"]) {
                Some((TECH_EAS_LIVENESS, asset))
            } else if has_flag(&rest_tokens, &["-sV", "-A"]) {
                Some((TECH_EAS_SERVICE_FINGERPRINT, asset))
            } else {
                Some((TECH_EAS_PORT, asset))
            }
        }
        "naabu" | "masscan" => target_from_flags(&rest_tokens, &["-host"], normalize_target_token)
            .or_else(|| first_target_like(&rest_tokens, normalize_target_token))
            .map(|asset| (TECH_EAS_PORT, asset)),
        "whatweb" => first_target_like(&rest_tokens, normalize_target_token)
            .map(|asset| (TECH_EAS_SERVICE_FINGERPRINT, asset)),
        _ => None,
    }
}

fn command_tool_and_rest(command: &str) -> Option<(String, &str)> {
    let (exe, rest) = split_executable(command);
    let tool = exe.rsplit('/').next().unwrap_or(exe).to_ascii_lowercase();
    if matches!(
        tool.as_str(),
        "ruby" | "ruby.exe" | "python" | "python3" | "python.exe" | "node" | "node.exe"
    ) {
        let (wrapped, wrapped_rest) = split_executable(rest.trim_start());
        let wrapped_tool = wrapped
            .rsplit('/')
            .next()
            .unwrap_or(wrapped)
            .to_ascii_lowercase();
        if wrapped_tool.is_empty() {
            None
        } else {
            Some((wrapped_tool, wrapped_rest))
        }
    } else if tool.is_empty() {
        None
    } else {
        Some((tool, rest))
    }
}

fn has_flag(tokens: &[&str], flags: &[&str]) -> bool {
    tokens.iter().any(|token| {
        let token = token.trim_matches(|c| c == '"' || c == '\'');
        flags.contains(&token)
    })
}

fn target_from_flags(
    tokens: &[&str],
    flags: &[&str],
    normalize: fn(&str) -> Option<String>,
) -> Option<String> {
    tokens
        .iter()
        .position(|token| flags.contains(token))
        .and_then(|idx| tokens.get(idx + 1))
        .and_then(|token| normalize(token))
}

fn first_target_like(tokens: &[&str], normalize: fn(&str) -> Option<String>) -> Option<String> {
    tokens
        .iter()
        .filter(|token| !token.starts_with('-') && !token.starts_with('+'))
        .find_map(|token| normalize(token))
}

fn normalize_liveness_target_token(token: &str) -> Option<String> {
    let value = clean_target_token(token)?;
    eas_liveness_asset_key(value)
}

fn normalize_target_token(token: &str) -> Option<String> {
    let value = clean_target_token(token)?;

    let host = if let Some((scheme, rest)) = value.split_once("://") {
        if !matches!(scheme, "http" | "https") {
            return None;
        }
        rest.split(['/', '?', '#']).next().unwrap_or(rest)
    } else {
        if value.contains('/') {
            return None;
        }
        value
    };
    let host = strip_port(host);
    if host.is_empty() {
        return None;
    }
    if host.parse::<std::net::IpAddr>().is_ok() || host.contains('.') {
        Some(host.to_string())
    } else {
        None
    }
}

fn clean_target_token(token: &str) -> Option<&str> {
    let value = token
        .trim()
        .trim_matches(',')
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches('.');
    if value.is_empty()
        || value.starts_with('-')
        || value.starts_with('+')
        || value.starts_with('@')
        || value.contains(['|', ';', '&', '$', '`', '<', '>'])
    {
        None
    } else {
        Some(value)
    }
}

fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        return host
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']').map(|(inside, _)| inside))
            .unwrap_or(host);
    }
    match host.rsplit_once(':') {
        Some((without_port, port)) if port.chars().all(|c| c.is_ascii_digit()) => without_port,
        _ => host,
    }
}

/// Split a command line into `(executable, rest_args)`, honoring a leading
/// double-quoted executable path that may contain spaces (e.g. the macOS
/// `~/Library/Application Support/.../subfinder`). An unquoted command splits on
/// the first run of whitespace, matching the prior behaviour for bare tools.
fn split_executable(command: &str) -> (&str, &str) {
    if let Some(after) = command.strip_prefix('"') {
        if let Some(end) = after.find('"') {
            return (&after[..end], after[end + 1..].trim_start());
        }
    }
    match command.split_once(char::is_whitespace) {
        Some((exe, rest)) => (exe, rest.trim_start()),
        None => (command, ""),
    }
}

/// `rest` 里第一个「长得像域名/IP 的资产」: 含 `.`、不是选项 (`-`/`+` 开头)、
/// 不是 `@server` 指定、不含 shell 元字符. 找不到 → None (保守不猜).
fn first_domain_like(rest: &[&str]) -> Option<String> {
    rest.iter()
        .find(|t| {
            t.contains('.')
                && !t.starts_with('-')
                && !t.starts_with('+')
                && !t.starts_with('@')
                && !t.contains(['/', '|', ';', '&', '$', '`', '<', '>'])
        })
        .map(|t| t.trim_end_matches('.').to_string())
}

/// `--flag value` / `-flag value` 形式取值 (subfinder `-d moresec.cn`).
fn flag_value(rest: &[&str], flag: &str) -> Option<String> {
    rest.iter()
        .position(|t| *t == flag)
        .and_then(|i| rest.get(i + 1))
        .filter(|v| !v.starts_with('-'))
        .map(|v| v.to_string())
}

/// 一次带 technique 标注的运行的结局: `"found"` (有产出) / `"empty"` (跑了→空).
///
/// I8 数据层兑现: empty 只在高置信形态下判定 — 输出整体为空, 或 DNS 工具的
/// 确定性「无记录」banner (NXDOMAIN / 有 QUESTION 无 ANSWER section). 其余一律
/// `"found"` (宽 found 被 corroborated gate 兜底, 宽 empty 没有安全网).
pub fn passive_intel_outcome(technique: &str, raw_output: &str) -> &'static str {
    let trimmed = raw_output.trim();
    if trimmed.is_empty() {
        return "empty";
    }
    if technique == "GOLISH-INTEL-DNS" {
        if trimmed.contains("NXDOMAIN") {
            return "empty";
        }
        // dig 全量 banner: 有 QUESTION section 但没有 ANSWER section = 查了无记录.
        if trimmed.contains(";; QUESTION SECTION") && !trimmed.contains(";; ANSWER SECTION") {
            return "empty";
        }
    }
    if technique == "GOLISH-INTEL-WHOIS" {
        // whois 确定性「无注册记录」banner (Phase 1, I8 数据层兑现): 跑了 whois
        // 但该域名/IP 未注册. 多家 whois server 的 not-found 措辞统一在这里收口;
        // 其余 (拿不准) 仍 found (corroborated gate 兜底假 found, 假 empty 无网).
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("no match for")
            || lower.contains("not found")
            || lower.contains("no data found")
            || lower.contains("no entries found")
        {
            return "empty";
        }
    }
    "found"
}

/// 同 [`passive_intel_outcome`], 但把「这次运行是否成功」一并纳入判定.
///
/// 一次**失败**的被动检查 (非零退出 / 超时 / crt.sh 这类外部服务抽风) 必须落**终态**
/// (否则 gate 对永远填不上的格无限重试). `distinguish_failure` (T2, 设计
/// 2026-06-23-failure-outcome-not-checked-empty) 决定失败记什么:
/// - `false` (gray-switch off, 缺省旧行为): 失败记 `"empty"` (checked_empty).
/// - `true`: 失败记 `"error"` —— 「跑了但拿不到」≠「已检查为空」, gate 仍当终态但
///   语义为「失败阻断」, 审计/诊断按 error 区分.
///
/// 成功时两种模式都维持基于输出的原判定 (I8 高置信 empty / found).
pub fn passive_intel_outcome_for_run(
    technique: &str,
    raw_output: &str,
    succeeded: bool,
    distinguish_failure: bool,
) -> &'static str {
    if succeeded {
        passive_intel_outcome(technique, raw_output)
    } else if distinguish_failure {
        "error"
    } else {
        "empty"
    }
}

pub fn coverage_outcome_for_run(
    technique: &str,
    raw_output: &str,
    succeeded: bool,
    distinguish_failure: bool,
) -> &'static str {
    if matches!(
        technique,
        TECH_EAS_LIVENESS | TECH_EAS_PORT | TECH_EAS_SERVICE_FINGERPRINT
    ) {
        eas_outcome_for_run(technique, raw_output, succeeded, distinguish_failure)
    } else {
        passive_intel_outcome_for_run(technique, raw_output, succeeded, distinguish_failure)
    }
}

fn eas_outcome_for_run(
    technique: &str,
    raw_output: &str,
    succeeded: bool,
    distinguish_failure: bool,
) -> &'static str {
    if !succeeded {
        return if distinguish_failure {
            "error"
        } else {
            "empty"
        };
    }
    let trimmed = raw_output.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("failed to resolve")
        || lower.contains("no targets were specified")
        || lower.contains("name or service not known")
        || lower.contains("no such host")
    {
        return "error";
    }
    match technique {
        TECH_EAS_LIVENESS
            if trimmed.is_empty()
                || lower.contains("0 hosts up")
                || lower.contains("no alive hosts")
                || lower.contains("no host found") =>
        {
            "empty"
        }
        TECH_EAS_PORT
            if trimmed.is_empty()
                || lower.contains("0 hosts up")
                || lower.contains("no open ports")
                || lower.contains("found 0 ports")
                || lower.contains("all ")
                    && lower.contains(" scanned ports ")
                    && lower.contains("closed") =>
        {
            "empty"
        }
        TECH_EAS_SERVICE_FINGERPRINT if trimmed.is_empty() => "empty",
        TECH_EAS_LIVENESS | TECH_EAS_PORT | TECH_EAS_SERVICE_FINGERPRINT => "found",
        _ => "found",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── passive_intel_facts_from_command: 每条映射逐一钉死 ──

    #[test]
    fn dig_maps_to_dns_with_domain() {
        assert_eq!(
            passive_intel_facts_from_command("dig moresec.cn A +short"),
            Some(("GOLISH-INTEL-DNS", "moresec.cn".to_string()))
        );
        // type 关键词在前、@server 与选项被跳过, 仍取到域名.
        assert_eq!(
            passive_intel_facts_from_command("dig @8.8.8.8 +short MX moresec.cn"),
            Some(("GOLISH-INTEL-DNS", "moresec.cn".to_string()))
        );
        // FQDN 尾点归一.
        assert_eq!(
            passive_intel_facts_from_command("host www.moresec.cn."),
            Some(("GOLISH-INTEL-DNS", "www.moresec.cn".to_string()))
        );
    }

    #[test]
    fn whois_maps_with_domain() {
        assert_eq!(
            passive_intel_facts_from_command("whois moresec.cn"),
            Some(("GOLISH-INTEL-WHOIS", "moresec.cn".to_string()))
        );
    }

    #[test]
    fn subfinder_takes_dash_d_value() {
        assert_eq!(
            passive_intel_facts_from_command("subfinder -d moresec.cn -silent"),
            Some(("GOLISH-INTEL-SUBDOMAIN", "moresec.cn".to_string()))
        );
        // 路径形式工具名归一.
        assert_eq!(
            passive_intel_facts_from_command("/opt/tools/subfinder -d moresec.cn"),
            Some(("GOLISH-INTEL-SUBDOMAIN", "moresec.cn".to_string()))
        );
    }

    /// Regression (2026-06-16 · SUBDOMAIN coverage gap): golish-managed tools run
    /// from a full, QUOTED executable path that may contain a space — the real
    /// macOS path is `~/Library/Application Support/golish-platform/tools/
    /// subfinder/subfinder`. The old `split_whitespace` tokenizer broke
    /// "Application Support" apart and read the tool name as "Application" →
    /// returned None → subfinder's subdomains never became a SUBDOMAIN coverage
    /// fact (while bare `/usr/bin/dig|whois` slipped through). The parser must
    /// honor a leading double-quoted executable path.
    #[test]
    fn quoted_executable_path_with_space_resolves_tool_name() {
        let cmd = "\"/Users/me/Library/Application Support/golish-platform/tools/subfinder/subfinder\" -d pingan.com -all -recursive -silent";
        assert_eq!(
            passive_intel_facts_from_command(cmd),
            Some(("GOLISH-INTEL-SUBDOMAIN", "pingan.com".to_string()))
        );
        // Same hazard for a path-resolved dig (defensive; dig usually runs bare).
        let dig = "\"/opt/My Tools/dig\" A pingan.com";
        assert_eq!(
            passive_intel_facts_from_command(dig),
            Some(("GOLISH-INTEL-DNS", "pingan.com".to_string()))
        );
    }

    #[test]
    fn ctfr_maps_to_ct() {
        assert_eq!(
            passive_intel_facts_from_command("ctfr -d moresec.cn"),
            Some(("GOLISH-INTEL-CT", "moresec.cn".to_string()))
        );
        // long `--domain` flag + path-resolved executable both normalize.
        assert_eq!(
            passive_intel_facts_from_command("ctfr --domain moresec.cn -o out.txt"),
            Some(("GOLISH-INTEL-CT", "moresec.cn".to_string()))
        );
        assert_eq!(
            passive_intel_facts_from_command("/opt/tools/ctfr -d moresec.cn"),
            Some(("GOLISH-INTEL-CT", "moresec.cn".to_string()))
        );
    }

    #[test]
    fn asnmap_maps_to_asn() {
        assert_eq!(
            passive_intel_facts_from_command("asnmap -d moresec.cn -silent"),
            Some(("GOLISH-INTEL-ASN", "moresec.cn".to_string()))
        );
        // IP subject via -i.
        assert_eq!(
            passive_intel_facts_from_command("asnmap -i 115.28.135.55"),
            Some(("GOLISH-INTEL-ASN", "115.28.135.55".to_string()))
        );
        // bare domain token fallback (no flag).
        assert_eq!(
            passive_intel_facts_from_command("asnmap moresec.cn"),
            Some(("GOLISH-INTEL-ASN", "moresec.cn".to_string()))
        );
    }

    #[test]
    fn unknown_or_ambiguous_returns_none() {
        // 主动扫描工具不在被动映射表 (灰度边界).
        assert_eq!(
            passive_intel_facts_from_command("nmap -sV moresec.cn"),
            None
        );
        // 已知工具但解析不出资产 → 保守 None (歧义即不派生).
        assert_eq!(passive_intel_facts_from_command("dig +short"), None);
        assert_eq!(passive_intel_facts_from_command("subfinder -silent"), None);
        // 资产 token 带 shell 元字符 → 不可信, None.
        assert_eq!(
            passive_intel_facts_from_command("dig moresec.cn/evil A"),
            None
        );
        assert_eq!(passive_intel_facts_from_command(""), None);
    }

    #[test]
    fn coverage_maps_eas_liveness_tools() {
        assert_eq!(
            coverage_facts_from_command("nmap -sn pinganstock.com"),
            Some((TECH_EAS_LIVENESS, "pinganstock.com".to_string()))
        );
        assert_eq!(
            coverage_facts_from_command(
                "\"/Users/me/Library/Application Support/golish-platform/tools/httpx/httpx\" -u http://pinganstock.com -sc -title"
            ),
            Some((TECH_EAS_LIVENESS, "pinganstock.com".to_string()))
        );
        assert_eq!(
            coverage_facts_from_command(
                "\"/Users/me/Library/Application Support/golish-platform/tools/httpx/httpx\" -u http://linquankuaipin.com:90 -json -silent"
            ),
            Some((TECH_EAS_LIVENESS, "linquankuaipin.com:90".to_string()))
        );
        assert_eq!(
            coverage_facts_from_command("naabu -host pinganstock.com -top-ports 100 -silent"),
            Some((TECH_EAS_PORT, "pinganstock.com".to_string()))
        );
    }

    #[test]
    fn eas_liveness_asset_key_preserves_url_endpoint_port() {
        assert_eq!(
            eas_liveness_asset_key("http://LinQuanKuaiPin.com:90").as_deref(),
            Some("linquankuaipin.com:90")
        );
        assert_eq!(
            eas_liveness_asset_key("https://example.com/login").as_deref(),
            Some("example.com/login")
        );
        assert_eq!(eas_liveness_asset_key("not-a-host"), None);
    }

    #[test]
    fn coverage_maps_wrapped_whatweb_service_fingerprint() {
        assert_eq!(
            coverage_facts_from_command(
                "\"/usr/bin/ruby\" \"/opt/tools/whatweb\" -a 1 https://www.example.com/login"
            ),
            Some((TECH_EAS_SERVICE_FINGERPRINT, "www.example.com".to_string()))
        );
    }

    // ── passive_intel_outcome: I8 的 empty 高置信判定 ──

    #[test]
    fn blank_output_is_empty() {
        assert_eq!(
            passive_intel_outcome("GOLISH-INTEL-SUBDOMAIN", "  \n"),
            "empty"
        );
    }

    #[test]
    fn dns_nxdomain_and_no_answer_section_are_empty() {
        assert_eq!(
            passive_intel_outcome("GOLISH-INTEL-DNS", ";; ->>HEADER<<- status: NXDOMAIN"),
            "empty"
        );
        let no_answer = ";; QUESTION SECTION:\n;moresec.cn. IN MX\n;; AUTHORITY SECTION:\nmoresec.cn. 600 IN SOA ...";
        assert_eq!(
            passive_intel_outcome("GOLISH-INTEL-DNS", no_answer),
            "empty"
        );
    }

    #[test]
    fn nonempty_output_is_found() {
        let answered = ";; QUESTION SECTION:\n;moresec.cn. IN A\n;; ANSWER SECTION:\nmoresec.cn. 600 IN A 1.2.3.4";
        assert_eq!(passive_intel_outcome("GOLISH-INTEL-DNS", answered), "found");
        assert_eq!(
            passive_intel_outcome("GOLISH-INTEL-SUBDOMAIN", "www.moresec.cn\nmail.moresec.cn"),
            "found"
        );
        // whois 拿到真注册数据 = found.
        let whois_hit =
            "Domain Name: MORESEC.CN\nRegistrar: Alibaba Cloud\nName Server: dns1.hichina.com";
        assert_eq!(
            passive_intel_outcome("GOLISH-INTEL-WHOIS", whois_hit),
            "found"
        );
    }

    #[test]
    fn failed_run_is_checked_empty_regardless_of_output() {
        // I8: a passive check that FAILED (timeout / non-zero exit / flaky crt.sh)
        // is checked_empty, not unchecked — even if partial/error output leaked, the
        // run did not complete, so the cell must reach a terminal (empty) state
        // instead of looping the gate. CT (ctfr/crt.sh) is the motivating case.
        // gray-switch off（distinguish_failure=false）= 旧行为：失败记 empty。
        assert_eq!(
            passive_intel_outcome_for_run("GOLISH-INTEL-CT", "", false, false),
            "empty"
        );
        assert_eq!(
            passive_intel_outcome_for_run("GOLISH-INTEL-CT", "502 Bad Gateway", false, false),
            "empty"
        );
        // Success keeps the stdout-derived verdict.
        assert_eq!(
            passive_intel_outcome_for_run(
                "GOLISH-INTEL-SUBDOMAIN",
                "www.moresec.cn\nmail.moresec.cn",
                true,
                false
            ),
            "found"
        );
        assert_eq!(
            passive_intel_outcome_for_run("GOLISH-INTEL-DNS", "", true, false),
            "empty"
        );
    }

    #[test]
    fn failed_run_is_error_when_distinguish_failure_on() {
        // T2: gray-switch on（distinguish_failure=true）→ 失败记 error（≠ empty），
        // 区分「失败阻断」与「已查为空」。
        assert_eq!(
            passive_intel_outcome_for_run("GOLISH-INTEL-CT", "", false, true),
            "error"
        );
        assert_eq!(
            passive_intel_outcome_for_run("GOLISH-INTEL-CT", "502 Bad Gateway", false, true),
            "error"
        );
        // 成功路径不受 distinguish_failure 影响（仍走 stdout 判定）。
        assert_eq!(
            passive_intel_outcome_for_run("GOLISH-INTEL-DNS", "", true, true),
            "empty"
        );
        assert_eq!(
            passive_intel_outcome_for_run("GOLISH-INTEL-SUBDOMAIN", "www.moresec.cn", true, true),
            "found"
        );
    }

    #[test]
    fn eas_liveness_dns_failure_is_terminal_error() {
        let raw = "Starting Nmap 7.99\nNmap done: 0 IP addresses (0 hosts up) scanned in 0.51 seconds\n\n[stderr]\nFailed to resolve \"pinganstock.com\".\nWARNING: No targets were specified, so 0 hosts scanned.";
        assert_eq!(
            coverage_outcome_for_run(TECH_EAS_LIVENESS, raw, true, false),
            "error"
        );
    }

    #[test]
    fn eas_liveness_empty_http_probe_is_checked_empty() {
        assert_eq!(
            coverage_outcome_for_run(TECH_EAS_LIVENESS, "", true, false),
            "empty"
        );
    }

    #[test]
    fn eas_liveness_host_up_is_found() {
        assert_eq!(
            coverage_outcome_for_run(
                TECH_EAS_LIVENESS,
                "Nmap scan report for a.example\nHost is up.",
                true,
                false
            ),
            "found"
        );
    }

    #[test]
    fn whois_not_found_banners_are_empty() {
        // Phase 1: whois 确定性「无注册记录」措辞 → empty (跑了→空, I8).
        for banner in [
            "No match for \"NOTREG.CN\".",
            "NOT FOUND",
            "No Data Found",
            "no entries found in the AfriNIC database",
        ] {
            assert_eq!(
                passive_intel_outcome("GOLISH-INTEL-WHOIS", banner),
                "empty",
                "whois not-found banner should be empty: {banner}"
            );
        }
    }

    // ── Phase 2: subsidiary_discovery_facts (recon_discover_subsidiaries JSON) ──

    #[test]
    fn subsidiary_promoted_children_is_found() {
        let v = serde_json::json!({
            "company": "默安科技", "status": "Completed",
            "phase": "subsidiaries", "promoted_children": 2
        });
        assert_eq!(
            subsidiary_discovery_facts(&v),
            Some((TECH_SUBSIDIARY, "默安科技".to_string(), "found"))
        );
    }

    #[test]
    fn subsidiary_completed_zero_promoted_is_empty() {
        // I8: provider 跑完、0 个候选清过阈值 → 高置信 empty (跑了→空 ≠ 没跑).
        let v = serde_json::json!({
            "company": "默安科技", "status": "Completed",
            "phase": "subsidiaries", "promoted_children": 0
        });
        assert_eq!(
            subsidiary_discovery_facts(&v),
            Some((TECH_SUBSIDIARY, "默安科技".to_string(), "empty"))
        );
    }

    #[test]
    fn subsidiary_partial_failed_or_malformed_derives_nothing() {
        // Partial/Failed 跑没跑完拿不准 → 不派生 (fail-closed, 格保持 not_attempted).
        for status in ["Partial", "Failed"] {
            let v = serde_json::json!({
                "company": "默安科技", "status": status, "promoted_children": 0
            });
            assert_eq!(subsidiary_discovery_facts(&v), None, "status={status}");
        }
        // 字段缺失 / 空公司名 → None (绝不猜).
        assert_eq!(
            subsidiary_discovery_facts(&serde_json::json!({"status": "Completed"})),
            None
        );
        assert_eq!(
            subsidiary_discovery_facts(
                &serde_json::json!({"company": " ", "status": "Completed", "promoted_children": 0})
            ),
            None
        );
        assert_eq!(
            subsidiary_discovery_facts(
                &serde_json::json!({"company": "默安科技", "status": "Completed"})
            ),
            None,
            "missing promoted_children must not derive (older summary shape)"
        );
    }

    // ── expansion_leads_from_subsidiary_discovery (#6) ──

    #[test]
    fn expansion_leads_extracts_subsidiary_candidates() {
        let v = serde_json::json!({
            "company": "默安科技", "status": "Completed", "phase": "subsidiaries",
            "subsidiaries": [
                {"name": "子公司A", "meets_threshold": true},
                {"name": "子公司B", "meets_threshold": false}
            ]
        });
        let leads = expansion_leads_from_subsidiary_discovery(&v);
        assert_eq!(leads.len(), 2);
        assert_eq!(
            leads[0],
            ExpansionLead {
                lead_type: "subsidiary",
                lead_value: "子公司A".to_string(),
                confidence: Some(0.9),
            }
        );
        // meets_threshold=false → 中置信 0.5.
        assert_eq!(leads[1].confidence, Some(0.5));
    }

    #[test]
    fn expansion_leads_empty_when_no_subsidiaries_field() {
        // auto_promote ON (subsidiaries 省略) 或 enrich phase → 无线索.
        let v = serde_json::json!({
            "company": "默安科技", "status": "Completed", "promoted_children": 2
        });
        assert!(expansion_leads_from_subsidiary_discovery(&v).is_empty());
    }

    #[test]
    fn expansion_leads_skips_blank_or_nameless_candidates() {
        let v = serde_json::json!({
            "subsidiaries": [
                {"name": "  ", "meets_threshold": true},
                {"meets_threshold": true},
                {"name": "真子公司", "meets_threshold": false}
            ]
        });
        let leads = expansion_leads_from_subsidiary_discovery(&v);
        assert_eq!(leads.len(), 1);
        assert_eq!(leads[0].lead_value, "真子公司");
    }
}
