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

/// 解析一条被动情报命令行 → `(注册 technique id, 主资产)`.
///
/// 覆盖 target_intel 灰度 (D-scope) 的常见被动工具; 主动扫描工具 (nmap/httpx…)
/// 与解析不出资产的命令一律 `None`. `command` 形如 `"dig moresec.cn A +short"`
/// (run_pty_cmd / background_job 的 shell 命令, 或 pentest_run 的 `"{tool} {args}"`).
pub fn passive_intel_facts_from_command(command: &str) -> Option<(&'static str, String)> {
    let mut tokens = command.split_whitespace();
    let tool = tokens.next()?;
    // 路径形式 (`/usr/bin/dig`) 归一到裸名.
    let tool = tool.rsplit('/').next().unwrap_or(tool);
    let rest: Vec<&str> = tokens.collect();
    match tool {
        "dig" | "nslookup" | "host" => first_domain_like(&rest).map(|d| ("GOLISH-INTEL-DNS", d)),
        "whois" => first_domain_like(&rest).map(|d| ("GOLISH-INTEL-WHOIS", d)),
        "subfinder" => flag_value(&rest, "-d")
            .or_else(|| flag_value(&rest, "-domain"))
            .map(|d| ("GOLISH-INTEL-SUBDOMAIN", d)),
        _ => None,
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
        // 拿不准的形态一律 found (corroborated gate 兜底; 假 empty 无安全网).
        assert_eq!(
            passive_intel_outcome("GOLISH-INTEL-WHOIS", "No match for domain"),
            "found"
        );
    }
}
