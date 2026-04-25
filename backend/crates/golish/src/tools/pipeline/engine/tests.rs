use super::*;
use crate::tools::pipeline::*;
use crate::tools::pipeline::templates::*;
use crate::tools::output_parser::StoreStats;

#[test]
fn test_detect_target_type() {
    assert_eq!(detect_target_type("example.com"), "domain");
    assert_eq!(detect_target_type("sub.example.com"), "domain");
    assert_eq!(detect_target_type("192.168.1.1"), "ip");
    assert_eq!(detect_target_type("10.0.0.1"), "ip");
    assert_eq!(detect_target_type("https://example.com"), "url");
    assert_eq!(detect_target_type("http://example.com/path"), "url");
    assert_eq!(detect_target_type("8.138.1.100"), "ip");
}

#[test]
fn test_recon_basic_has_requires() {
    let pipeline = recon_basic_template();
    let dig = pipeline.steps.iter().find(|s| s.tool_name == "dig").unwrap();
    assert_eq!(dig.requires.as_deref(), Some("domain"));

    let subfinder = pipeline.steps.iter().find(|s| s.tool_name == "subfinder").unwrap();
    assert_eq!(subfinder.requires.as_deref(), Some("domain"));

    let httpx = pipeline.steps.iter().find(|s| s.tool_name == "httpx").unwrap();
    assert_eq!(httpx.requires, None);
    assert_eq!(httpx.input_from, None);
    assert_eq!(httpx.iterate_over.as_deref(), Some("ports"));

    let naabu = pipeline.steps.iter().find(|s| s.tool_name == "naabu").unwrap();
    assert_eq!(naabu.requires, None);
}

#[test]
fn test_recon_basic_step_order() {
    let pipeline = recon_basic_template();
    let names: Vec<&str> = pipeline.steps.iter().map(|s| s.tool_name.as_str()).collect();
    assert_eq!(names, &["dig", "subfinder", "naabu", "httpx", "whatweb", "katana"]);

    let naabu_idx = names.iter().position(|n| *n == "naabu").unwrap();
    let httpx_idx = names.iter().position(|n| *n == "httpx").unwrap();
    let whatweb_idx = names.iter().position(|n| *n == "whatweb").unwrap();
    assert!(naabu_idx < httpx_idx, "naabu must run before httpx");
    assert!(naabu_idx < whatweb_idx, "naabu must run before whatweb");
}

#[test]
fn test_recon_basic_iterate_over() {
    let pipeline = recon_basic_template();
    let httpx = pipeline.steps.iter().find(|s| s.tool_name == "httpx").unwrap();
    assert_eq!(httpx.iterate_over.as_deref(), Some("ports"));

    let whatweb = pipeline.steps.iter().find(|s| s.tool_name == "whatweb").unwrap();
    assert_eq!(whatweb.iterate_over.as_deref(), Some("ports"));

    let katana = pipeline.steps.iter().find(|s| s.tool_name == "katana").unwrap();
    assert_eq!(katana.iterate_over.as_deref(), Some("ports"));
    assert_eq!(katana.db_action.as_deref(), Some("target_add"));

    let naabu = pipeline.steps.iter().find(|s| s.tool_name == "naabu").unwrap();
    assert_eq!(naabu.iterate_over, None);
}

// ── Helpers ──

fn conn(from: &str, to: &str) -> PipelineConnection {
    PipelineConnection { from_step: from.into(), to_step: to.into(), condition: None }
}

fn conn_if(from: &str, to: &str, cond: &str) -> PipelineConnection {
    PipelineConnection { from_step: from.into(), to_step: to.into(), condition: Some(cond.into()) }
}

fn make_step(id: &str) -> PipelineStep {
    PipelineStep {
        id: id.to_string(),
        step_type: "shell_command".to_string(),
        tool_name: id.to_string(),
        tool_id: String::new(),
        command_template: "echo".to_string(),
        args: vec![],
        params: serde_json::json!({}),
        input_from: None,
        exec_mode: "sequential".to_string(),
        requires: None,
        iterate_over: None,
        db_action: None,
        on_failure: "continue".to_string(),
        timeout_secs: None,
        sub_pipeline: None,
        inline_pipeline: None,
        foreach_source: None,
        max_parallel: None,
        x: 0.0,
        y: 0.0,
    }
}

#[test]
fn test_topo_layers_empty_connections_is_sequential() {
    let steps = vec![make_step("a"), make_step("b"), make_step("c")];
    let layers = topo_layers(&steps, &[]);
    assert_eq!(layers.len(), 3, "empty connections → one layer per step");
    assert_eq!(layers[0][0].id, "a");
    assert_eq!(layers[1][0].id, "b");
    assert_eq!(layers[2][0].id, "c");
}

#[test]
fn test_topo_layers_linear_chain() {
    let steps = vec![make_step("a"), make_step("b"), make_step("c")];
    let conns = vec![conn("a", "b"), conn("b", "c")];
    let layers = topo_layers(&steps, &conns);
    assert_eq!(layers.len(), 3);
    assert_eq!(layers[0].len(), 1);
    assert_eq!(layers[0][0].id, "a");
    assert_eq!(layers[1][0].id, "b");
    assert_eq!(layers[2][0].id, "c");
}

#[test]
fn test_topo_layers_parallel_fan_out() {
    // a → b, a → c (b and c should be in the same layer)
    let steps = vec![make_step("a"), make_step("b"), make_step("c")];
    let conns = vec![conn("a", "b"), conn("a", "c")];
    let layers = topo_layers(&steps, &conns);
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].len(), 1);
    assert_eq!(layers[0][0].id, "a");
    assert_eq!(layers[1].len(), 2, "b and c should run in parallel");
    let ids: Vec<&str> = layers[1].iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"b"));
    assert!(ids.contains(&"c"));
}

#[test]
fn test_topo_layers_diamond() {
    // a → b, a → c, b → d, c → d
    let steps = vec![make_step("a"), make_step("b"), make_step("c"), make_step("d")];
    let conns = vec![conn("a", "b"), conn("a", "c"), conn("b", "d"), conn("c", "d")];
    let layers = topo_layers(&steps, &conns);
    assert_eq!(layers.len(), 3);
    assert_eq!(layers[0][0].id, "a");
    assert_eq!(layers[1].len(), 2, "b and c parallel");
    assert_eq!(layers[2].len(), 1);
    assert_eq!(layers[2][0].id, "d");
}

#[test]
fn test_topo_layers_recon_basic_dag() {
    let pipeline = recon_basic_template();
    let layers = topo_layers(&pipeline.steps, &pipeline.connections);

    // Layer 0: dig, subfinder, naabu (no incoming connections)
    assert_eq!(layers[0].len(), 3, "layer 0 should have dig, subfinder, naabu");
    let l0_ids: Vec<&str> = layers[0].iter().map(|s| s.id.as_str()).collect();
    assert!(l0_ids.contains(&"dns_lookup"));
    assert!(l0_ids.contains(&"subdomain_enum"));
    assert!(l0_ids.contains(&"port_scan"));

    // Layer 1: httpx, whatweb (depend on port_scan)
    assert_eq!(layers[1].len(), 2, "layer 1 should have httpx, whatweb");
    let l1_ids: Vec<&str> = layers[1].iter().map(|s| s.id.as_str()).collect();
    assert!(l1_ids.contains(&"http_probe"));
    assert!(l1_ids.contains(&"tech_fingerprint"));

    // Layer 2: katana (depends on httpx and whatweb)
    assert_eq!(layers[2].len(), 1);
    assert_eq!(layers[2][0].id, "web_crawl");
}

#[test]
fn test_topo_layers_disconnected_steps_at_start() {
    // Steps e and f have no connections, should appear in layer 0
    let steps = vec![
        make_step("a"), make_step("b"), make_step("e"), make_step("f"),
    ];
    let conns = vec![conn("a", "b")];
    let layers = topo_layers(&steps, &conns);
    // a, e, f all have in_degree 0 → layer 0
    assert!(layers[0].len() >= 3);
    let l0_ids: Vec<&str> = layers[0].iter().map(|s| s.id.as_str()).collect();
    assert!(l0_ids.contains(&"a"));
    assert!(l0_ids.contains(&"e"));
    assert!(l0_ids.contains(&"f"));
    // b depends on a → later layer
    let all_later: Vec<&str> = layers[1..].iter().flat_map(|l| l.iter().map(|s| s.id.as_str())).collect();
    assert!(all_later.contains(&"b"));
}

#[test]
fn test_new_fields_have_defaults() {
    let json = r#"{"id":"test","tool_name":"echo","steps":[],"connections":[],"name":"t","created_at":0,"updated_at":0}"#;
    let pipeline: Pipeline = serde_json::from_str(json).unwrap();
    assert!(pipeline.steps.is_empty());

    let step_json = r#"{"id":"s1","tool_name":"nmap"}"#;
    let step: PipelineStep = serde_json::from_str(step_json).unwrap();
    assert_eq!(step.on_failure, "abort");
    assert_eq!(step.timeout_secs, None);
    assert!(step.sub_pipeline.is_none());
    assert!(step.inline_pipeline.is_none());
    assert!(step.foreach_source.is_none());
    assert!(step.max_parallel.is_none());
}

#[test]
fn test_connection_condition_default() {
    let json = r#"{"from_step":"a","to_step":"b"}"#;
    let c: PipelineConnection = serde_json::from_str(json).unwrap();
    assert!(c.condition.is_none());

    let json2 = r#"{"from_step":"a","to_step":"b","condition":"exit_ok"}"#;
    let c2: PipelineConnection = serde_json::from_str(json2).unwrap();
    assert_eq!(c2.condition.as_deref(), Some("exit_ok"));
}

#[test]
fn test_step_sub_pipeline_fields_deser() {
    let json = r#"{"id":"s1","step_type":"sub_pipeline","tool_name":"web","sub_pipeline":"web_vuln_v1"}"#;
    let step: PipelineStep = serde_json::from_str(json).unwrap();
    assert_eq!(step.step_type, "sub_pipeline");
    assert_eq!(step.sub_pipeline.as_deref(), Some("web_vuln_v1"));
    assert!(step.inline_pipeline.is_none());
}

#[test]
fn test_step_foreach_fields_deser() {
    let json = r#"{"id":"s1","step_type":"foreach","tool_name":"scan","foreach_source":"subfinder","max_parallel":3}"#;
    let step: PipelineStep = serde_json::from_str(json).unwrap();
    assert_eq!(step.step_type, "foreach");
    assert_eq!(step.foreach_source.as_deref(), Some("subfinder"));
    assert_eq!(step.max_parallel, Some(3));
}

// ── evaluate_condition tests ──

fn make_result(exit: Option<i32>, lines: usize) -> StepResult {
    StepResult {
        step_id: "test".into(),
        tool_name: "test".into(),
        command: String::new(),
        exit_code: exit,
        stdout_lines: lines,
        stderr_preview: String::new(),
        store_stats: None,
        duration_ms: 0,
    }
}

#[test]
fn test_evaluate_condition_exit_ok() {
    let tmp = std::env::temp_dir().join("test_cond_exit_ok.txt");
    std::fs::write(&tmp, "some output").unwrap();

    assert!(evaluate_condition("exit_ok", &make_result(Some(0), 1), &tmp));
    assert!(!evaluate_condition("exit_ok", &make_result(Some(1), 1), &tmp));
    assert!(!evaluate_condition("exit_ok", &make_result(None, 0), &tmp));
}

#[test]
fn test_evaluate_condition_exit_fail() {
    let tmp = std::env::temp_dir().join("test_cond_exit_fail.txt");
    std::fs::write(&tmp, "").unwrap();

    assert!(evaluate_condition("exit_fail", &make_result(Some(1), 0), &tmp));
    assert!(!evaluate_condition("exit_fail", &make_result(Some(0), 0), &tmp));
    assert!(!evaluate_condition("exit_fail", &make_result(None, 0), &tmp));
}

#[test]
fn test_evaluate_condition_output_not_empty() {
    let tmp = std::env::temp_dir().join("test_cond_not_empty.txt");
    std::fs::write(&tmp, "data").unwrap();

    assert!(evaluate_condition("output_not_empty", &make_result(Some(0), 3), &tmp));
    assert!(!evaluate_condition("output_not_empty", &make_result(Some(0), 0), &tmp));
}

#[test]
fn test_evaluate_condition_output_contains() {
    let tmp = std::env::temp_dir().join("test_cond_contains.txt");
    std::fs::write(&tmp, "80/tcp open http\n22/tcp open ssh").unwrap();

    assert!(evaluate_condition("output_contains:80", &make_result(Some(0), 2), &tmp));
    assert!(evaluate_condition("output_contains:22", &make_result(Some(0), 2), &tmp));
    assert!(!evaluate_condition("output_contains:443", &make_result(Some(0), 2), &tmp));
}

#[test]
fn test_evaluate_condition_output_lines_gt() {
    let tmp = std::env::temp_dir().join("test_cond_lines_gt.txt");
    std::fs::write(&tmp, "").unwrap();

    assert!(evaluate_condition("output_lines_gt:5", &make_result(Some(0), 10), &tmp));
    assert!(!evaluate_condition("output_lines_gt:5", &make_result(Some(0), 3), &tmp));
    assert!(!evaluate_condition("output_lines_gt:5", &make_result(Some(0), 5), &tmp));
}

#[test]
fn test_evaluate_condition_stored_gt() {
    let tmp = std::env::temp_dir().join("test_cond_stored_gt.txt");
    std::fs::write(&tmp, "").unwrap();

    let mut res = make_result(Some(0), 0);
    res.store_stats = Some(StoreStats {
        parsed_count: 12,
        stored_count: 10,
        new_count: 10,
        skipped_count: 2,
        errors: vec![],
    });
    assert!(evaluate_condition("stored_gt:5", &res, &tmp));
    assert!(!evaluate_condition("stored_gt:15", &res, &tmp));

    let res2 = make_result(Some(0), 0);
    assert!(!evaluate_condition("stored_gt:0", &res2, &tmp));
}

#[test]
fn test_evaluate_condition_unknown_passes() {
    let tmp = std::env::temp_dir().join("test_cond_unknown.txt");
    std::fs::write(&tmp, "").unwrap();
    assert!(evaluate_condition("some_future_condition", &make_result(Some(0), 0), &tmp));
}

// ── Condition-based DAG skipping (via topo) ──

#[test]
fn test_topo_with_conditional_connections() {
    let steps = vec![make_step("scan"), make_step("web"), make_step("ssh")];
    let conns = vec![
        conn_if("scan", "web", "output_contains:80"),
        conn_if("scan", "ssh", "output_contains:22"),
    ];
    let layers = topo_layers(&steps, &conns);
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0][0].id, "scan");
    let l1_ids: Vec<&str> = layers[1].iter().map(|s| s.id.as_str()).collect();
    assert!(l1_ids.contains(&"web"));
    assert!(l1_ids.contains(&"ssh"));
}

#[test]
fn test_inline_pipeline_deser() {
    let json = r#"{
        "id": "nest",
        "step_type": "sub_pipeline",
        "tool_name": "inner",
        "inline_pipeline": {
            "id": "inner_p",
            "name": "Inner Pipeline",
            "steps": [{"id": "echo_step", "tool_name": "echo"}],
            "connections": []
        }
    }"#;
    let step: PipelineStep = serde_json::from_str(json).unwrap();
    assert_eq!(step.step_type, "sub_pipeline");
    let inner = step.inline_pipeline.unwrap();
    assert_eq!(inner.id, "inner_p");
    assert_eq!(inner.steps.len(), 1);
    assert_eq!(inner.steps[0].id, "echo_step");
}

#[test]
fn test_advanced_flow_json_roundtrip() {
    let json = r#"{
        "id": "advanced",
        "name": "Advanced Recon",
        "steps": [
            {"id": "subfinder", "tool_name": "subfinder", "command_template": "subfinder", "args": ["-d", "{target}", "-silent"]},
            {"id": "naabu", "tool_name": "naabu", "command_template": "naabu", "args": ["-host", "{target}"]},
            {"id": "web_scan", "step_type": "sub_pipeline", "tool_name": "web", "sub_pipeline": "web_vuln_v1"},
            {"id": "ssh_audit", "tool_name": "ssh-audit", "command_template": "ssh-audit", "args": ["{target}"]},
            {"id": "per_sub", "step_type": "foreach", "tool_name": "scan", "foreach_source": "subfinder", "sub_pipeline": "single_host_recon", "max_parallel": 3}
        ],
        "connections": [
            {"from_step": "naabu", "to_step": "web_scan", "condition": "output_contains:80"},
            {"from_step": "naabu", "to_step": "ssh_audit", "condition": "output_contains:22"},
            {"from_step": "subfinder", "to_step": "per_sub", "condition": "output_not_empty"}
        ]
    }"#;
    let pipeline: Pipeline = serde_json::from_str(json).unwrap();
    assert_eq!(pipeline.steps.len(), 5);
    assert_eq!(pipeline.connections.len(), 3);

    assert_eq!(pipeline.steps[2].step_type, "sub_pipeline");
    assert_eq!(pipeline.steps[2].sub_pipeline.as_deref(), Some("web_vuln_v1"));
    assert_eq!(pipeline.steps[4].step_type, "foreach");
    assert_eq!(pipeline.steps[4].foreach_source.as_deref(), Some("subfinder"));
    assert_eq!(pipeline.steps[4].max_parallel, Some(3));

    assert_eq!(pipeline.connections[0].condition.as_deref(), Some("output_contains:80"));
    assert_eq!(pipeline.connections[1].condition.as_deref(), Some("output_contains:22"));
    assert_eq!(pipeline.connections[2].condition.as_deref(), Some("output_not_empty"));

    let layers = topo_layers(&pipeline.steps, &pipeline.connections);
    assert_eq!(layers[0].len(), 2, "subfinder and naabu in parallel (layer 0)");
    let l0_ids: Vec<&str> = layers[0].iter().map(|s| s.id.as_str()).collect();
    assert!(l0_ids.contains(&"subfinder"));
    assert!(l0_ids.contains(&"naabu"));

    assert!(layers.len() >= 2);
}
