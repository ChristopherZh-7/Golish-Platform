//! Category-based stage tool whitelist (deny-by-default).
//!
//! Replaces the per-stage `forbidden_tools` blacklist (fragile: a missed entry
//! leaks) with a per-stage `allowed_tool_types` whitelist whose entries are
//! **tool-type selectors** instead of a long list of individual tool names:
//!
//! - a bare **category**           → `"recon"`         (matches any recon tool)
//! - a **category/subcategory**    → `"recon/dns"`     (only that subcategory)
//! - a specific **tool name**      → `"nmap"`          (one tool)
//!
//! A tool call is allowed iff its (resolved) tool name, its category, or its
//! `category/subcategory` appears in the stage's `allowed` list. Anything not
//! matched is denied (deny-by-default). This is the security-correct posture
//! (whitelist) and auto-blocks newly added dangerous tools.
//!
//! The category mapping mirrors `resources/toolsconfig/*.json`
//! (`category` + `subcategory`). It is embedded here (pure, no cross-crate dep /
//! IO, same pattern as [`super::tool_capability`]); keep it in sync when tools
//! are added to `toolsconfig`. Common CLI aliases (e.g. `msfconsole` →
//! metasploit) are folded in so wrapper calls (`pentest_run` / `run_pty_cmd`)
//! resolve correctly.
//!
//! Meta / orchestration tools (`query_target_data`, `submit_stage_deliverable`,
//! `sub_agent_*`, `log_operation`, …) are NOT in the taxonomy and are exempt
//! from the whitelist at the guard layer (they are never scan tools); this
//! matcher intentionally returns `false` for them — callers keep the existing
//! meta/orchestration exemption.
//!
//! See `docs/design/2026-06-02-stage-tool-whitelist-enforcement.md` and the
//! category-whitelist evolution note.

use serde_json::Value;

/// Map a (lowercased) tool / CLI name onto its `(category, subcategory)` from
/// the tool taxonomy. Returns `None` for unknown / meta tools.
///
/// Mirrors `resources/toolsconfig/*.json`; common CLI aliases are folded onto
/// the canonical tool so wrapper invocations resolve.
pub fn tool_category(name: &str) -> Option<(&'static str, &'static str)> {
    let n = name.trim().to_ascii_lowercase();
    let pair = match n.as_str() {
        // ── recon ────────────────────────────────────────────────────────────
        "dig" | "nslookup" | "host" | "dnsx" | "dnsrecon" => ("recon", "dns"),
        "nmap" | "masscan" | "rustscan" | "naabu" => ("recon", "port-scan"),
        "httpx" | "whatweb" | "curl" | "wget" | "http" => ("recon", "http"),
        "amass" | "subfinder" | "assetfinder" | "sublist3r" | "findomain" => ("recon", "subdomain"),
        "katana" | "hakrawler" | "gospider" => ("recon", "crawler"),
        "gau" | "waybackurls" => ("recon", "url-history"),
        "enscan_go" | "enscan" | "0.zone" | "0zone" | "zero-zone" => ("recon", "osint"),
        "gowitness" | "aquatone" | "eyewitness" | "cutycapt" => ("recon", "visual"),
        // ── web ──────────────────────────────────────────────────────────────
        "ffuf" | "gobuster" | "dirb" | "dirsearch" | "feroxbuster" => ("web", "fuzzer"),
        "arjun" | "paramspider" | "x8" => ("web", "param"),
        "nikto" | "nuclei" | "dalfox" => ("web", "scanner"),
        "wpscan" => ("web", "cms"),
        "sqlmap" => ("web", "injection"),
        // ── network ──────────────────────────────────────────────────────────
        "chisel" | "ligolo" => ("network", "tunnel"),
        "responder" => ("network", "mitm"),
        "wireshark" | "tcpdump" | "tshark" => ("network", "sniffer"),
        // ── brute ────────────────────────────────────────────────────────────
        "john" | "hashcat" => ("brute", "offline"),
        "hydra" | "medusa" | "ncrack" => ("brute", "online"),
        // ── exploit ──────────────────────────────────────────────────────────
        "searchsploit" => ("exploit", "edb"),
        "metasploit" | "metasploit-framework" | "msfconsole" | "msfvenom" | "msf" => {
            ("exploit", "framework")
        }
        // ── post-exploit ─────────────────────────────────────────────────────
        "netexec" | "crackmapexec" | "cme" | "nxc" => ("post-exploit", "lateral-movement"),
        "impacket" | "bloodhound-python" | "bloodhound" => ("post-exploit", "ad-tools"),
        _ => return None,
    };
    Some(pair)
}

/// Resolve a tool call to the underlying **tool name** (the raw inner tool, not
/// a canonical capability alias). Meta /
/// shell wrappers (`pentest_run`, `run_pty_cmd`, `run_command`) hide the real
/// tool in their args; this unwraps them so category lookup sees the real tool.
pub fn underlying_tool_name(tool_name: &str, args: &Value) -> String {
    match tool_name {
        "pentest_run" => {
            let inner = args
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if inner.is_empty() {
                tool_name.to_ascii_lowercase()
            } else {
                inner.to_ascii_lowercase()
            }
        }
        "run_pty_cmd" | "run_command" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let first = cmd.split_whitespace().next().unwrap_or("");
            let base = first.rsplit('/').next().unwrap_or(first);
            if base.is_empty() {
                tool_name.to_ascii_lowercase()
            } else {
                base.to_ascii_lowercase()
            }
        }
        other => other.to_ascii_lowercase(),
    }
}

/// Whether a tool call is a **scan invocation** that the per-stage whitelist
/// should govern, vs an agent / meta / control-plane tool that is exempt.
///
/// Returns `true` for the scan-execution wrappers (`pentest_run`, `run_pty_cmd`,
/// `run_command`) and for any tool that resolves to a known scan
/// [`tool_category`]. Returns `false` for agent/meta tools (`sub_agent_*`,
/// `submit_stage_deliverable`, `query_target_data`, `record_finding`,
/// `manage_targets`, `log_*`, memory/graph tools, …) which are NOT in the scan
/// taxonomy — those are governed by other policy, not the stage tool whitelist.
///
/// This is the key that lets the whitelist be **deny-by-default for scans**
/// without blocking the agent's legitimate control-plane tools. (A scan wrapper
/// with an unknown inner tool still returns `true`, so [`stage_allows`] denies it
/// — safe deny-by-default for unrecognized scans.)
pub fn is_scan_invocation(tool_name: &str, args: &Value) -> bool {
    matches!(tool_name, "pentest_run" | "run_pty_cmd" | "run_command")
        || tool_category(&underlying_tool_name(tool_name, args)).is_some()
}

/// Name-only variant of [`is_scan_invocation`] for **tool-list filtering** (where
/// no call args are available yet). A tool definition is a scan tool iff it is a
/// scan-execution wrapper (`pentest_run` / `run_pty_cmd` / `run_command`) or its
/// name resolves to a known scan [`tool_category`]. Used to hide scan tools from
/// the model in stages whose `allowed_tool_types` is empty (e.g. scoping /
/// reporting), so the model never sees a scan tool it could only be blocked on.
pub fn is_scan_tool_name(tool_name: &str) -> bool {
    matches!(tool_name, "pentest_run" | "run_pty_cmd" | "run_command")
        || tool_category(tool_name).is_some()
}

/// Whether a tool is an **offensive / active sub-agent dispatcher** that a
/// confirm-only stage must not expose.
///
/// `sub_agent_*` tools are exempt from the scan whitelist (they are
/// control-plane dispatchers, see the module note), but a stage that permits
/// no scans (`allowed_tool_types` empty, e.g. scoping / reporting) also has no
/// business delegating active recon / exploitation. These offensive
/// dispatchers are therefore hidden from the model in such stages; the
/// non-offensive helpers (`sub_agent_reporter` / `_researcher` / `_memorist` /
/// …) stay available so e.g. reporting can still delegate write-ups.
pub fn is_offensive_sub_agent(tool_name: &str) -> bool {
    matches!(tool_name, "sub_agent_pentester" | "sub_agent_browser")
}

/// Whether the (resolved) tool call is permitted by a stage's `allowed_tool_types`
/// **type-selector** list (deny-by-default).
///
/// A selector matches when it equals (case-insensitively) the resolved tool
/// name, the tool's category, or its `category/subcategory`. An empty `allowed`
/// list permits nothing (e.g. scoping / reporting → zero scan tools). Unknown
/// tools (not in the taxonomy) are permitted only by an explicit name selector.
///
/// NOTE: meta / orchestration tools are exempt at the guard layer, not here.
pub fn stage_allows(tool_name: &str, args: &Value, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return false;
    }
    let name = underlying_tool_name(tool_name, args);
    let selectors: Vec<String> = allowed
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();
    if selectors.iter().any(|s| s == &name) {
        return true;
    }
    if let Some((cat, sub)) = tool_category(&name) {
        let cat_sub = format!("{cat}/{sub}");
        if selectors.iter().any(|s| s == cat || s == &cat_sub) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn allow(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn category_lookup_known_and_aliases() {
        assert_eq!(tool_category("dig"), Some(("recon", "dns")));
        assert_eq!(tool_category("NMAP"), Some(("recon", "port-scan")));
        assert_eq!(tool_category("masscan"), Some(("recon", "port-scan")));
        assert_eq!(tool_category("subfinder"), Some(("recon", "subdomain")));
        assert_eq!(tool_category("sqlmap"), Some(("web", "injection")));
        assert_eq!(tool_category("nuclei"), Some(("web", "scanner")));
        assert_eq!(tool_category("arjun"), Some(("web", "param")));
        assert_eq!(tool_category("msfconsole"), Some(("exploit", "framework")));
        assert_eq!(
            tool_category("bloodhound-python"),
            Some(("post-exploit", "ad-tools"))
        );
        // meta / unknown → None
        assert_eq!(tool_category("query_target_data"), None);
        assert_eq!(tool_category("submit_stage_deliverable"), None);
    }

    #[test]
    fn underlying_unwraps_wrappers() {
        assert_eq!(
            underlying_tool_name("pentest_run", &json!({"tool_name": "Dig", "args": "x"})),
            "dig"
        );
        assert_eq!(
            underlying_tool_name("run_pty_cmd", &json!({"command": "/usr/bin/nmap -p- x"})),
            "nmap"
        );
        assert_eq!(underlying_tool_name("amass", &json!({})), "amass");
        // missing inner → wrapper name (nothing to resolve)
        assert_eq!(
            underlying_tool_name("pentest_run", &json!({})),
            "pentest_run"
        );
    }

    #[test]
    fn category_selector_allows_whole_category() {
        let allowed = allow(&["recon"]);
        // any recon tool passes
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "dig"}),
            &allowed
        ));
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "nmap"}),
            &allowed
        ));
        // a web tool does not
        assert!(!stage_allows(
            "pentest_run",
            &json!({"tool_name": "sqlmap"}),
            &allowed
        ));
    }

    #[test]
    fn subcategory_selector_is_precise() {
        // eas-style: passive recon subcategories only — dig ok, nmap (port-scan) blocked
        let allowed = allow(&["recon/dns", "recon/subdomain", "recon/http"]);
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "dig"}),
            &allowed
        ));
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "subfinder"}),
            &allowed
        ));
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "httpx"}),
            &allowed
        ));
        // nmap is recon/port-scan → NOT in this subcategory allow → blocked
        assert!(!stage_allows(
            "pentest_run",
            &json!({"tool_name": "nmap"}),
            &allowed
        ));
    }

    #[test]
    fn specific_tool_name_selector() {
        let allowed = allow(&["nmap"]);
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "nmap"}),
            &allowed
        ));
        assert!(!stage_allows(
            "pentest_run",
            &json!({"tool_name": "masscan"}),
            &allowed
        ));
    }

    #[test]
    fn empty_allowed_denies_everything() {
        // scoping / reporting → no scan tool permitted
        assert!(!stage_allows(
            "pentest_run",
            &json!({"tool_name": "dig"}),
            &[]
        ));
        assert!(!stage_allows("sqlmap", &json!({}), &[]));
    }

    #[test]
    fn deny_by_default_for_unmatched() {
        // enumeration-style allow; sqlmap (web/injection) is not in it → denied
        let allowed = allow(&["recon/port-scan", "recon/http", "web/fuzzer"]);
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "nmap"}),
            &allowed
        ));
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "gobuster"}),
            &allowed
        ));
        assert!(!stage_allows(
            "pentest_run",
            &json!({"tool_name": "sqlmap"}),
            &allowed
        ));
        assert!(!stage_allows(
            "pentest_run",
            &json!({"tool_name": "metasploit"}),
            &allowed
        ));
    }

    #[test]
    fn mixed_selectors_category_subcat_and_name() {
        let allowed = allow(&["web/scanner", "exploit", "nmap"]);
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "nuclei"}),
            &allowed
        )); // web/scanner
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "searchsploit"}),
            &allowed
        )); // exploit category
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "nmap"}),
            &allowed
        )); // by name
        assert!(!stage_allows(
            "pentest_run",
            &json!({"tool_name": "ffuf"}),
            &allowed
        )); // web/fuzzer not allowed
    }

    #[test]
    fn is_scan_invocation_distinguishes_scan_from_meta() {
        // scan wrappers + known scan tools → governed by the whitelist
        assert!(is_scan_invocation(
            "pentest_run",
            &json!({"tool_name": "dig"})
        ));
        assert!(is_scan_invocation(
            "run_pty_cmd",
            &json!({"command": "nmap x"})
        ));
        assert!(is_scan_invocation("nuclei", &json!({})));
        // scan wrapper with unknown inner → still governed (deny-by-default scan)
        assert!(is_scan_invocation(
            "pentest_run",
            &json!({"tool_name": "weirdtool"})
        ));
        // agent / meta / control-plane tools → exempt (not scan)
        for meta in [
            "sub_agent_pentester",
            "submit_stage_deliverable",
            "query_target_data",
            "record_finding",
            "manage_targets",
            "log_operation",
            "search_memories",
            "graph_add_entity",
            "ask_human",
        ] {
            assert!(
                !is_scan_invocation(meta, &json!({})),
                "{meta} must be exempt"
            );
        }
    }

    #[test]
    fn run_pty_cmd_resolves_for_category_match() {
        let allowed = allow(&["recon/dns"]);
        assert!(stage_allows(
            "run_pty_cmd",
            &json!({"command": "dig +short example.com"}),
            &allowed
        ));
        assert!(!stage_allows(
            "run_pty_cmd",
            &json!({"command": "sqlmap -u http://x"}),
            &allowed
        ));
    }

    #[test]
    fn is_scan_tool_name_classifies_by_name_only() {
        // scan wrappers + known scan tools → true (governed by the whitelist)
        for scan in [
            "pentest_run",
            "run_pty_cmd",
            "run_command",
            "nmap",
            "nuclei",
            "sqlmap",
            "subfinder",
        ] {
            assert!(is_scan_tool_name(scan), "{scan} must be a scan tool");
        }
        // meta / control-plane tools → false (exempt; never hidden)
        for meta in [
            "sub_agent_pentester",
            "submit_stage_deliverable",
            "read_file",
            "ask_human",
            "query_target_data",
        ] {
            assert!(!is_scan_tool_name(meta), "{meta} must NOT be a scan tool");
        }
    }

    #[test]
    fn offensive_sub_agents_are_flagged_for_confirm_only_stages() {
        // Active/offensive dispatchers a zero-scan stage (scoping/reporting) hides.
        for off in ["sub_agent_pentester", "sub_agent_browser"] {
            assert!(is_offensive_sub_agent(off), "{off} must be offensive");
        }
        // Non-offensive helpers + meta tools must stay available.
        for keep in [
            "sub_agent_reporter",
            "sub_agent_researcher",
            "sub_agent_memorist",
            "submit_stage_deliverable",
            "ask_human",
            "nmap",
        ] {
            assert!(
                !is_offensive_sub_agent(keep),
                "{keep} must NOT be flagged offensive"
            );
        }
    }
}
