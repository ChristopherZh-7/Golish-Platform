//! Q3 ③ · Stage-aware annotation of `pentest_list_tools` results.
//!
//! `pentest_list_tools` returns the full installed catalogue with no notion of
//! the active harness stage, so a worker only learns a tool is out-of-stage by
//! calling it and hitting a `BLOCK`. This helper marks each listed tool with a
//! `stage_allowed` flag (and a stage summary) so the boundary is visible up
//! front.
//!
//! The verdict is computed with [`golish_agent_kit::harness::stage_allows`] —
//! the SAME predicate the dispatch guard enforces with — so the annotation can
//! never disagree with what the gate will actually do (a tool marked
//! `stage_allowed: true` is one that will run; `false` is one that would be
//! BLOCKED).

use serde_json::{json, Value};

/// Annotate a `pentest_list_tools` result JSON in place for the active stage.
///
/// Adds, per entry in `tools[]`, a boolean `stage_allowed`; and at the top
/// level `stage`, `stage_allowed_tools` (the permitted names) and a human
/// `stage_note`. No-op when the value has no `tools` array.
pub(crate) fn annotate_pentest_list_tools(
    value: &mut Value,
    stage_id: &str,
    allowed_types: &[String],
) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };

    let mut allowed_names: Vec<String> = Vec::new();
    if let Some(arr) = obj.get_mut("tools").and_then(|t| t.as_array_mut()) {
        for entry in arr.iter_mut() {
            let name = entry
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            // Probe exactly as the model would run it (via the pentest_run
            // wrapper) so the verdict matches the dispatch guard.
            let allowed = golish_agent_kit::harness::stage_allows(
                "pentest_run",
                &json!({ "tool_name": name }),
                allowed_types,
            );
            if let Some(entry_obj) = entry.as_object_mut() {
                entry_obj.insert("stage_allowed".to_string(), json!(allowed));
            }
            if allowed {
                allowed_names.push(name);
            }
        }
    }

    obj.insert("stage".to_string(), json!(stage_id));
    obj.insert("stage_allowed_tools".to_string(), json!(allowed_names));
    obj.insert(
        "stage_note".to_string(),
        json!(format!(
            "In the '{stage_id}' stage only tools with stage_allowed=true are usable; calling any \
             other tool here is out-of-stage and will be BLOCKED — do not call it."
        )),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "tools": [
                { "name": "dig", "category": "recon", "subcategory": "dns" },
                { "name": "subfinder", "category": "recon", "subcategory": "subdomain" },
                { "name": "nmap", "category": "recon", "subcategory": "port-scan" },
                { "name": "sqlmap", "category": "web", "subcategory": "injection" }
            ],
            "total": 4
        })
    }

    fn allow(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn marks_each_tool_for_target_intel() {
        let mut v = sample();
        annotate_pentest_list_tools(
            &mut v,
            "target_intel",
            &allow(&["recon/dns", "recon/subdomain"]),
        );

        let tools = v["tools"].as_array().unwrap();
        let by_name = |n: &str| {
            tools
                .iter()
                .find(|t| t["name"] == n)
                .and_then(|t| t["stage_allowed"].as_bool())
                .unwrap()
        };
        assert!(by_name("dig"), "dig is recon/dns → allowed");
        assert!(
            by_name("subfinder"),
            "subfinder is recon/subdomain → allowed"
        );
        assert!(!by_name("nmap"), "nmap is recon/port-scan → blocked");
        assert!(!by_name("sqlmap"), "sqlmap is web/injection → blocked");

        let allowed: Vec<&str> = v["stage_allowed_tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        assert_eq!(allowed, vec!["dig", "subfinder"]);
        assert_eq!(v["stage"], "target_intel");
        assert!(v["stage_note"].as_str().unwrap().contains("target_intel"));
    }

    #[test]
    fn empty_allowed_marks_all_blocked() {
        let mut v = sample();
        annotate_pentest_list_tools(&mut v, "scoping", &[]);
        let tools = v["tools"].as_array().unwrap();
        assert!(tools.iter().all(|t| t["stage_allowed"] == json!(false)));
        assert_eq!(v["stage_allowed_tools"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn no_tools_array_is_noop() {
        let mut v = json!({ "error": "boom" });
        annotate_pentest_list_tools(&mut v, "target_intel", &allow(&["recon/dns"]));
        // still annotates top-level stage metadata, never panics
        assert_eq!(v["stage"], "target_intel");
        assert_eq!(v["error"], "boom");
    }
}
