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
}
