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
        "nmap"
        | "masscan"
        | "rustscan"
        | "naabu"
        | "eas_discover_ports"
        | "eas_fingerprint_services" => ("recon", "port-scan"),
        "httpx"
        | "whatweb"
        | "curl"
        | "wget"
        | "http"
        | "enum_preflight_web_origins"
        | "eas_probe_http_liveness"
        | "eas_fingerprint_web_stack" => ("recon", "http"),
        "amass" | "subfinder" | "assetfinder" | "sublist3r" | "findomain" => ("recon", "subdomain"),
        // Raw crawler CLIs (katana/hakrawler/gospider) are intentionally not
        // stage-allowlisted directly. Enumeration exposes the backend wrapper
        // below so the model cannot bypass same-origin/org-bound landing.
        "enum_crawl_same_origin_urls" | "browser_collect_js_api" | "js_extract_apis" => {
            ("recon", "crawler")
        }
        "gau" | "waybackurls" => ("recon", "url-history"),
        // whois / ASN lookups are zero-touch (query the registrar/RIR, not the
        // target's own hosts). Some stages may opt into this scan wrapper; current
        // target_intel prefers recon_lookup_whois.
        "whois" | "asn" | "whois-asn" => ("recon", "whois"),
        // ctfr queries certificate-transparency logs (crt.sh) — zero-touch on the
        // target. Dedicated `ct` subcategory so any stage that opts in can resolve
        // a concrete runnable tool.
        "ctfr" => ("recon", "ct"),
        // asnmap resolves a domain/IP/org into its ASN + netblock ranges via
        // public RIR data — zero-touch on the target. Dedicated `asn` subcategory
        // so stages that opt into `recon/asn` can resolve to a runnable tool (the
        // `asn` alias above maps to recon/whois).
        "asnmap" => ("recon", "asn"),
        "enscan_go" | "enscan" | "0.zone" | "0zone" | "zero-zone" => ("recon", "osint"),
        // Passive OSINT / leak-hunting / cloud-asset discovery. All ship in
        // `resources/toolsconfig/*.json` as category=recon subcategory=osint
        // (tagged `passive`); they query third-party sources (GitHub, search
        // engines, cloud-provider endpoints, social sites), not the target's own
        // hosts. `cloud_enum` ships as id `cloud-enum` + name `cloud_enum`, so
        // both spellings resolve.
        "trufflehog" | "gitleaks" | "gitdorker" | "go-dork" | "metagoofil" | "theharvester"
        | "maigret" | "holehe" | "sherlock" | "s3scanner" | "cloud_enum" | "cloud-enum"
        | "gcpbucketbrute" => ("recon", "osint"),
        "gowitness" | "aquatone" | "eyewitness" | "cutycapt" => ("recon", "visual"),
        // ── web ──────────────────────────────────────────────────────────────
        "ffuf" | "gobuster" | "dirb" | "dirsearch" | "feroxbuster" => ("web", "dir-fuzzer"),
        "route_probe_paths" => ("web", "route-probe"),
        "paramspider" | "x8" => ("web", "param"),
        "nikto"
        | "nuclei"
        | "dalfox"
        | "vuln_nuclei_general"
        | "vuln_nuclei_fingerprint_targeted" => ("web", "scanner"),
        "vuln_probe_anonymous_access" => ("web", "anonymous-access"),
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
        "verify_execute_candidate_action" => ("exploit", "candidate-verifier"),
        // ── post-exploit ─────────────────────────────────────────────────────
        "netexec" | "crackmapexec" | "cme" | "nxc" => ("post-exploit", "lateral-movement"),
        "impacket" | "bloodhound-python" | "bloodhound" => ("post-exploit", "ad-tools"),
        "post_exploit_validate_access" => ("post-exploit", "access-validation"),
        "post_exploit_record_internal_observation" => ("post-exploit", "internal-observation"),
        "post_exploit_build_objective_path" => ("post-exploit", "objective-path"),
        "post_exploit_execute_action" => ("post-exploit", "cleanup-bound-action"),
        "cleanup_inspect_obligation" => ("post-exploit", "cleanup-inspect"),
        "cleanup_execute_obligation" => ("post-exploit", "cleanup-execute"),
        "cleanup_verify_absence" => ("post-exploit", "cleanup-verify"),
        "cleanup_suggest_waiver" => ("post-exploit", "cleanup-waiver-suggestion"),
        _ => return None,
    };
    Some(pair)
}

/// Representative tool names per `(category, subcategory)` in the taxonomy,
/// used to resolve a stage's `allowed_tool_types` **type-selectors** back into
/// the concrete tool names the agent may actually run in that stage.
///
/// One or two primary tools per subcategory (NOT every alias) — enough for a
/// weak model to see the representative tools for stages that still expose
/// scan-tool selectors instead of having to map a selector onto a tool itself
/// (and mis-map `nmap` into a BLOCK). Keep in sync with the
/// [`tool_category`] match arms above.
const CANONICAL_TOOLS: &[&str] = &[
    // recon
    "dig",
    "nmap",
    "naabu",
    "eas_discover_ports",
    "eas_fingerprint_services",
    "httpx",
    "eas_probe_http_liveness",
    "eas_fingerprint_web_stack",
    "enum_preflight_web_origins",
    "whatweb",
    "subfinder",
    "amass",
    "enum_crawl_same_origin_urls",
    "browser_collect_js_api",
    "js_extract_apis",
    "gau",
    "waybackurls",
    "whois",
    "ctfr",
    "asnmap",
    "enscan_go",
    "gowitness",
    // web
    "ffuf",
    "route_probe_paths",
    "nuclei",
    "vuln_nuclei_general",
    "vuln_nuclei_fingerprint_targeted",
    "vuln_probe_anonymous_access",
    "wpscan",
    "sqlmap",
    // network
    "chisel",
    "responder",
    "tcpdump",
    // brute
    "john",
    "hydra",
    // exploit
    "searchsploit",
    "metasploit",
    "verify_execute_candidate_action",
    // post-exploit
    "netexec",
    "impacket",
    "post_exploit_validate_access",
    "post_exploit_record_internal_observation",
    "post_exploit_build_objective_path",
    "post_exploit_execute_action",
    "cleanup_inspect_obligation",
    "cleanup_execute_obligation",
    "cleanup_verify_absence",
    "cleanup_suggest_waiver",
];

/// Resolve a stage's `allowed_tool_types` type-selectors into the concrete
/// canonical tool names that stage permits (deny-by-default), in taxonomy order.
///
/// This is the reverse of [`stage_allows`]: instead of asking "is THIS tool
/// allowed?", it answers "which tools ARE allowed?". Used to front-load the
/// concrete usable tools into a specialist's objective (Q3 ①+), so a weak model
/// does not have to translate `recon/dns` → `dig` (and wrongly translate `nmap`
/// into a tool it can only get BLOCKED on). Empty `allowed` → no tools.
///
/// The result is consistent with the runtime stage guard (both go through
/// [`stage_allows`]): a name appears here iff the guard would let it run.
pub fn allowed_tool_names(allowed: &[String]) -> Vec<&'static str> {
    if allowed.is_empty() {
        return Vec::new();
    }
    CANONICAL_TOOLS
        .iter()
        .copied()
        .filter(|name| stage_allows(name, &Value::Null, allowed))
        .collect()
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

/// Background-job lifecycle commands are control-plane operations, not scan
/// invocations. They must stay usable inside active stages so a worker can inspect
/// or cancel a stuck background job without widening the stage's scan whitelist.
fn is_background_job_control_tool(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "check_job" | "kill_job" | "list_jobs" | "wait_for_background_jobs"
    )
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
    let inner = underlying_tool_name(tool_name, args);
    if is_background_job_control_tool(&inner) {
        return false;
    }
    matches!(tool_name, "pentest_run" | "run_pty_cmd" | "run_command")
        || tool_category(&inner).is_some()
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
    matches!(
        tool_name,
        "sub_agent_pentester" | "sub_agent_browser" | "sub_agent_vuln_scanner"
    )
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
    fn whois_resolves_to_recon_whois() {
        // P2 (2026-06-11): whois is a zero-touch passive technique (queries the
        // registrar, not the target's own hosts). It was previously absent from
        // the taxonomy, so stages that opted into it saw deny-by-default blocks.
        // Map it to a dedicated recon/whois subcategory so stages can opt in
        // without allowing every recon tool.
        assert_eq!(tool_category("whois"), Some(("recon", "whois")));
        // and a stage that allows recon/whois permits it
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "whois"}),
            &allow(&["recon/whois"])
        ));
    }

    #[test]
    fn category_lookup_known_and_aliases() {
        assert_eq!(tool_category("dig"), Some(("recon", "dns")));
        assert_eq!(tool_category("NMAP"), Some(("recon", "port-scan")));
        assert_eq!(tool_category("masscan"), Some(("recon", "port-scan")));
        assert_eq!(tool_category("subfinder"), Some(("recon", "subdomain")));
        assert_eq!(tool_category("sqlmap"), Some(("web", "injection")));
        assert_eq!(tool_category("nuclei"), Some(("web", "scanner")));
        assert_eq!(
            tool_category("vuln_nuclei_general"),
            Some(("web", "scanner"))
        );
        assert_eq!(
            tool_category("vuln_nuclei_fingerprint_targeted"),
            Some(("web", "scanner"))
        );
        assert_eq!(tool_category("arjun"), None);
        assert_eq!(
            tool_category("browser_collect_js_api"),
            Some(("recon", "crawler"))
        );
        assert_eq!(tool_category("js_extract_apis"), Some(("recon", "crawler")));
        assert_eq!(tool_category("ffuf"), Some(("web", "dir-fuzzer")));
        assert_eq!(
            tool_category("route_probe_paths"),
            Some(("web", "route-probe"))
        );
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
        // stage allow-list is precise: route_probe is allowed without opening
        // external directory fuzzers or injection/scanner tools.
        let allowed = allow(&["recon/port-scan", "recon/http", "web/route-probe"]);
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "nmap"}),
            &allowed
        ));
        assert!(stage_allows(
            "pentest_run",
            &json!({"tool_name": "route_probe_paths"}),
            &allowed
        ));
        assert!(!stage_allows(
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
        )); // external dir-fuzzer not allowed
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
        // background job control travels through pentest_run but is not a scan.
        for ctl in [
            "check_job",
            "kill_job",
            "list_jobs",
            "wait_for_background_jobs",
        ] {
            assert!(
                !is_scan_invocation("pentest_run", &json!({"tool_name": ctl})),
                "{ctl} must be exempt from the scan whitelist"
            );
        }
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
            "vuln_nuclei_general",
            "vuln_nuclei_fingerprint_targeted",
            "vuln_probe_anonymous_access",
            "sqlmap",
            "subfinder",
            "enum_preflight_web_origins",
            "browser_collect_js_api",
            "route_probe_paths",
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
        for off in [
            "sub_agent_pentester",
            "sub_agent_browser",
            "sub_agent_vuln_scanner",
        ] {
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

    #[test]
    fn allowed_tool_names_target_intel_provider_only_boundary() {
        // target_intel is provider/registry-tool backed. Its stage spec exposes
        // no scan-tool selectors, so `allowed_tool_names` must be empty even
        // though the stage still requires the passive intel coverage cells.
        let allowed = allow(&[]);
        let names = allowed_tool_names(&allowed);
        assert!(
            names.is_empty(),
            "target_intel must expose no scan tools: {names:?}"
        );
        for never in [
            "dig",
            "gau",
            "waybackurls",
            "whois",
            "ctfr",
            "asnmap",
            "enscan_go",
            "subfinder",
            "amass",
            "assetfinder",
            "sublist3r",
            "findomain",
            "nmap",
            "naabu",
            "httpx",
            "sqlmap",
            "nuclei",
        ] {
            assert!(
                !names.contains(&never),
                "{never} must NOT be listed: {names:?}"
            );
        }
    }

    #[test]
    fn allowed_tool_names_eas_selectors() {
        // external_attack_surface: active mapping — port-scan / http / visual.
        let allowed = allow(&["recon/port-scan", "recon/http", "recon/visual"]);
        let names = allowed_tool_names(&allowed);
        for must in [
            "nmap",
            "naabu",
            "httpx",
            "eas_fingerprint_web_stack",
            "whatweb",
            "gowitness",
        ] {
            assert!(names.contains(&must), "{must} must be listed: {names:?}");
        }
        // passive-only intel tools are not part of EAS's allowed types
        for never in ["dig", "subfinder", "whois", "enscan_go"] {
            assert!(
                !names.contains(&never),
                "{never} must NOT be listed: {names:?}"
            );
        }
    }

    #[test]
    fn allowed_tool_names_enumeration_selectors_include_direct_enum_tools() {
        let allowed = allow(&[
            "enum_preflight_web_origins",
            "recon/crawler",
            "web/route-probe",
        ]);
        let names = allowed_tool_names(&allowed);
        for must in [
            "enum_preflight_web_origins",
            "enum_crawl_same_origin_urls",
            "browser_collect_js_api",
            "js_extract_apis",
            "route_probe_paths",
        ] {
            assert!(names.contains(&must), "{must} must be listed: {names:?}");
            assert!(stage_allows(must, &json!({}), &allowed));
        }
        for never in [
            "httpx",
            "whatweb",
            "curl",
            "wget",
            "nmap",
            "naabu",
            "katana",
            "sqlmap",
            "ffuf",
            "gobuster",
            "feroxbuster",
            "arjun",
            "nuclei",
            "metasploit",
        ] {
            assert!(
                !names.contains(&never),
                "{never} must NOT be listed: {names:?}"
            );
            assert!(!stage_allows(never, &json!({}), &allowed));
        }
        assert!(!stage_allows(
            "pentest_run",
            &json!({"tool_name": "katana"}),
            &allowed
        ));
        assert!(!stage_allows(
            "run_command",
            &json!({"command": "katana -list roots.txt -jc"}),
            &allowed
        ));
    }

    #[test]
    fn allowed_tool_names_empty_allows_nothing() {
        assert!(allowed_tool_names(&[]).is_empty());
    }

    #[test]
    fn vuln_exact_wrappers_do_not_allow_raw_nuclei_or_pentest_run() {
        let allowed = allow(&[
            "vuln_nuclei_general",
            "vuln_nuclei_fingerprint_targeted",
            "vuln_probe_anonymous_access",
        ]);
        assert_eq!(
            allowed_tool_names(&allowed),
            vec![
                "vuln_nuclei_general",
                "vuln_nuclei_fingerprint_targeted",
                "vuln_probe_anonymous_access",
            ]
        );
        assert!(stage_allows("vuln_nuclei_general", &json!({}), &allowed));
        assert!(stage_allows(
            "vuln_nuclei_fingerprint_targeted",
            &json!({}),
            &allowed
        ));
        assert!(stage_allows(
            "vuln_probe_anonymous_access",
            &json!({}),
            &allowed
        ));
        assert!(!stage_allows("nuclei", &json!({}), &allowed));
        assert!(!stage_allows(
            "pentest_run",
            &json!({"tool_name": "nuclei"}),
            &allowed
        ));
    }

    #[test]
    fn allowed_tool_names_is_consistent_with_stage_allows() {
        // Every name the reverse-lookup returns MUST be one the stage guard
        // (stage_allows) would actually permit — otherwise the objective would
        // advertise a tool that gets BLOCKED at dispatch.
        let allowed = allow(&["recon/dns", "recon/subdomain", "web/scanner"]);
        for name in allowed_tool_names(&allowed) {
            assert!(
                stage_allows(name, &json!({}), &allowed),
                "{name} listed but stage_allows denies it"
            );
        }
    }

    /// The full set of passive-OSINT tool selectors a recon specialist may pass
    /// as `pentest_run`'s `tool_name`. Each is `category=recon subcategory=osint`
    /// in `resources/toolsconfig/*.json` (all tagged `passive`); they were absent
    /// from the taxonomy → `tool_category` None → deny-by-default BLOCK in stages
    /// that opt into recon/osint. `cloud_enum` ships as both id `cloud-enum` and
    /// name `cloud_enum`, so both spellings must resolve.
    const OSINT_TOOLS: &[&str] = &[
        "trufflehog",
        "gitleaks",
        "gitdorker",
        "go-dork",
        "metagoofil",
        "theharvester",
        "maigret",
        "holehe",
        "sherlock",
        "s3scanner",
        "cloud_enum",
        "cloud-enum",
        "gcpbucketbrute",
    ];

    #[test]
    fn osint_tools_resolve_to_recon_osint() {
        // 坑2 fix: mirror toolsconfig (category=recon subcategory=osint) so these
        // passive OSINT / leak / cloud-asset tools classify instead of returning
        // None (which the stage guard treats as deny-by-default).
        for t in OSINT_TOOLS {
            assert_eq!(
                tool_category(t),
                Some(("recon", "osint")),
                "{t} must resolve to recon/osint"
            );
        }
        // case-insensitive (agent may pass the toolsconfig display name)
        assert_eq!(tool_category("TruffleHog"), Some(("recon", "osint")));
        assert_eq!(tool_category("S3Scanner"), Some(("recon", "osint")));
        assert_eq!(tool_category("GitDorker"), Some(("recon", "osint")));
    }

    #[test]
    fn osint_tools_resolve_but_provider_only_target_intel_blocks_scan_wrappers() {
        // OSINT wrappers still resolve to recon/osint for stages that opt in,
        // but target_intel no longer exposes scan-tool selectors at all.
        let target_intel = allow(&[]);
        let eas = allow(&["recon/port-scan", "recon/http", "recon/visual"]);
        for t in OSINT_TOOLS {
            assert!(
                !stage_allows("pentest_run", &json!({ "tool_name": t }), &target_intel),
                "{t} must be blocked in provider-only target_intel"
            );
            assert!(
                !stage_allows("pentest_run", &json!({ "tool_name": t }), &eas),
                "{t} must be blocked in external_attack_surface"
            );
        }
    }
}
