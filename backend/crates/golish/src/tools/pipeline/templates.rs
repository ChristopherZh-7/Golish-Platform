use super::*;

pub(crate) fn pipeline_from_json(json: &str) -> Option<Pipeline> {
    let mut p: Pipeline = serde_json::from_str(json).ok()?;
    for (i, step) in p.steps.iter_mut().enumerate() {
        if step.x == 0.0 && step.y == 0.0 {
            step.x = (i as f64) * 220.0 + 40.0;
            step.y = 80.0;
        }
    }
    Some(p)
}

/// Embedded built-in templates (compiled into the binary).
fn embedded_templates() -> Vec<Pipeline> {
    const RECON_BASIC: &str = include_str!("../templates/recon_basic.json");
    [RECON_BASIC]
        .iter()
        .filter_map(|json| pipeline_from_json(json))
        .collect()
}

pub(crate) fn templates_dir() -> Option<std::path::PathBuf> {
    golish_core::paths::flow_templates_dir()
}

fn user_templates() -> Vec<Pipeline> {
    let Some(dir) = templates_dir() else {
        return vec![];
    };
    if !dir.exists() {
        return vec![];
    }
    let mut templates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Ok(data) = std::fs::read_to_string(&path) {
                    if let Some(mut p) = pipeline_from_json(&data) {
                        p.is_template = true;
                        templates.push(p);
                    }
                }
            }
        }
    }
    templates
}

pub(crate) fn builtin_templates() -> Vec<Pipeline> {
    let mut all = embedded_templates();
    let user = user_templates();
    let user_ids: std::collections::HashSet<&str> =
        user.iter().map(|p| p.id.as_str()).collect();
    all.retain(|p| !user_ids.contains(p.id.as_str()));
    all.extend(user);
    all
}

pub fn get_builtin_recon_basic() -> Pipeline {
    embedded_templates()
        .into_iter()
        .find(|p| p.id == "recon_basic")
        .unwrap_or_else(recon_basic_template)
}

/// Detect target type: "domain", "ip", or "url"
pub fn detect_target_type(target: &str) -> &'static str {
    if target.starts_with("http://") || target.starts_with("https://") {
        return "url";
    }
    // Check if it looks like an IP (v4 only for simplicity)
    if target.split('.').count() == 4
        && target.split('.').all(|s| s.parse::<u8>().is_ok())
    {
        return "ip";
    }
    "domain"
}

/// Topological sort of pipeline steps into execution layers.
/// Steps in the same layer have no dependencies between each other and can run concurrently.
/// Falls back to sequential execution (one step per layer) when connections are empty.
pub(crate) fn topo_layers<'a>(
    steps: &'a [PipelineStep],
    connections: &[PipelineConnection],
) -> Vec<Vec<&'a PipelineStep>> {
    if connections.is_empty() {
        return steps.iter().map(|s| vec![s]).collect();
    }

    let step_ids: std::collections::HashSet<&str> =
        steps.iter().map(|s| s.id.as_str()).collect();

    let mut in_degree: std::collections::HashMap<&str, usize> =
        steps.iter().map(|s| (s.id.as_str(), 0)).collect();

    let mut adj: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();

    for conn in connections {
        if step_ids.contains(conn.from_step.as_str())
            && step_ids.contains(conn.to_step.as_str())
        {
            *in_degree.entry(conn.to_step.as_str()).or_insert(0) += 1;
            adj.entry(conn.from_step.as_str())
                .or_default()
                .push(conn.to_step.as_str());
        }
    }

    let step_map: std::collections::HashMap<&str, &PipelineStep> =
        steps.iter().map(|s| (s.id.as_str(), s)).collect();

    let mut layers: Vec<Vec<&PipelineStep>> = Vec::new();
    let mut visited = std::collections::HashSet::new();

    let mut current: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();
    current.sort();

    while !current.is_empty() {
        let layer: Vec<&PipelineStep> = current
            .iter()
            .filter_map(|id| step_map.get(id).copied())
            .collect();

        for &id in &current {
            visited.insert(id);
        }

        if !layer.is_empty() {
            layers.push(layer);
        }

        let mut next = Vec::new();
        for &id in &current {
            if let Some(neighbors) = adj.get(id) {
                for &nid in neighbors {
                    if let Some(deg) = in_degree.get_mut(nid) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 && !visited.contains(nid) {
                            next.push(nid);
                        }
                    }
                }
            }
        }
        next.sort();
        next.dedup();
        current = next;
    }

    let remaining: Vec<&PipelineStep> = steps
        .iter()
        .filter(|s| !visited.contains(s.id.as_str()))
        .collect();
    if !remaining.is_empty() {
        layers.push(remaining);
    }

    layers
}

/// Resolve the input file for a step, using explicit `input_from`, upstream connections,
/// or falling back to the target seed when the command uses `{prev_output}`.
pub(crate) fn resolve_step_input(
    step: &PipelineStep,
    step_outputs: &std::collections::HashMap<String, std::path::PathBuf>,
    connections: &[PipelineConnection],
    tmp_dir: &std::path::Path,
    target: &str,
) -> Option<std::path::PathBuf> {
    if let Some(ref from_id) = step.input_from {
        if let Some(path) = step_outputs.get(from_id) {
            return Some(path.clone());
        }
    }

    let upstream: Vec<&str> = connections
        .iter()
        .filter(|c| c.to_step == step.id)
        .map(|c| c.from_step.as_str())
        .collect();
    for uid in &upstream {
        if let Some(path) = step_outputs.get(*uid) {
            return Some(path.clone());
        }
    }

    let full_cmd_preview = format!("{} {}", step.command_template, step.args.join(" "));
    if full_cmd_preview.contains("{prev_output}") {
        let seed = tmp_dir.join(format!("seed-{}.txt", step.id));
        let _ = std::fs::write(&seed, target);
        return Some(seed);
    }

    None
}

/// Evaluate a condition expression against an upstream step's result and output file.
/// Returns `true` if the condition passes (step should run), `false` to skip.
pub(crate) fn evaluate_condition(
    condition: &str,
    result: &StepResult,
    output_path: &std::path::Path,
) -> bool {
    match condition {
        "exit_ok" => result.exit_code == Some(0),
        "exit_fail" => result.exit_code.is_some() && result.exit_code != Some(0),
        "output_not_empty" => result.stdout_lines > 0,
        _ if condition.starts_with("output_contains:") => {
            let pattern = &condition["output_contains:".len()..];
            std::fs::read_to_string(output_path)
                .map(|s| s.contains(pattern))
                .unwrap_or(false)
        }
        _ if condition.starts_with("output_lines_gt:") => {
            let n: usize = condition["output_lines_gt:".len()..].parse().unwrap_or(0);
            result.stdout_lines > n
        }
        _ if condition.starts_with("stored_gt:") => {
            let n: usize = condition["stored_gt:".len()..].parse().unwrap_or(0);
            result
                .store_stats
                .as_ref()
                .map(|s| s.stored_count > n)
                .unwrap_or(false)
        }
        other => {
            tracing::warn!("[pipeline] Unknown condition '{}', treating as pass", other);
            true
        }
    }
}

/// Resolve per-port target URLs from the `targets.ports` JSONB column.
/// Returns a vec of URLs like `http://8.138.179.62:8080`, `https://8.138.179.62:443`.
pub(crate) async fn resolve_port_targets(
    pool: &sqlx::PgPool,
    target: &str,
    project_path: Option<&str>,
) -> Vec<String> {
    let ports_json: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT ports FROM targets
           WHERE value = $1 AND project_path = $2
           LIMIT 1"#,
    )
    .bind(target)
    .bind(project_path)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let Some(serde_json::Value::Array(ports)) = ports_json else {
        tracing::info!(target = %target, "[resolve_port_targets] No ports column or not an array, falling back to default");
        return vec![format!("http://{}", target)];
    };

    if ports.is_empty() {
        tracing::info!(target = %target, "[resolve_port_targets] Empty ports array, falling back to default");
        return vec![format!("http://{}", target)];
    }

    tracing::info!(target = %target, port_count = ports.len(), "[resolve_port_targets] Found ports in DB");

    let urls: Vec<String> = ports
        .iter()
        .filter_map(|entry| {
            let port = entry.get("port")?.as_u64()? as u16;
            let service = entry
                .get("service")
                .and_then(|s| s.as_str())
                .unwrap_or("http");
            let scheme = if service == "https" || port == 443 {
                "https"
            } else {
                "http"
            };
            let url = if (scheme == "http" && port == 80) || (scheme == "https" && port == 443) {
                format!("{}://{}", scheme, target)
            } else {
                format!("{}://{}:{}", scheme, target, port)
            };
            Some(url)
        })
        .collect();

    tracing::info!(
        target = %target,
        resolved_count = urls.len(),
        urls = ?urls,
        "[resolve_port_targets] Resolved URLs for iteration"
    );
    urls
}

pub(crate) fn recon_basic_template() -> Pipeline {
    // step: (id, name, step_type, cmd, args, input_from, requires)
    struct StepDef {
        id: &'static str,
        name: &'static str,
        step_type: &'static str,
        cmd: &'static str,
        args: Vec<&'static str>,
        input_from: Option<&'static str>,
        requires: Option<&'static str>,
        iterate_over: Option<&'static str>,
        db_action: Option<&'static str>,
    }

    let steps = vec![
        StepDef {
            id: "dns_lookup", name: "dig", step_type: "dns_lookup",
            cmd: "dig", args: vec!["+short", "{target}"],
            input_from: None, requires: Some("domain"),
            iterate_over: None, db_action: None,
        },
        StepDef {
            id: "subdomain_enum", name: "subfinder", step_type: "subdomain_enum",
            cmd: "subfinder", args: vec!["-d", "{target}", "-silent"],
            input_from: None, requires: Some("domain"),
            iterate_over: None, db_action: None,
        },
        StepDef {
            id: "port_scan", name: "naabu", step_type: "port_scan",
            cmd: "naabu", args: vec!["-host", "{target}", "-top-ports", "1000", "-json", "-silent"],
            input_from: None, requires: None,
            iterate_over: None, db_action: None,
        },
        StepDef {
            id: "http_probe", name: "httpx", step_type: "http_probe",
            cmd: "httpx", args: vec!["-u", "{target}", "-sc", "-title", "-tech-detect", "-json", "-silent"],
            input_from: None, requires: None,
            iterate_over: Some("ports"), db_action: None,
        },
        StepDef {
            id: "tech_fingerprint", name: "whatweb", step_type: "tech_fingerprint",
            cmd: "whatweb", args: vec!["{target}", "--color=never"],
            input_from: None, requires: None,
            iterate_over: Some("ports"), db_action: None,
        },
        StepDef {
            id: "web_crawl", name: "katana", step_type: "web_crawl",
            cmd: "katana", args: vec!["-u", "{target}", "-d", "3", "-js-crawl", "-silent"],
            input_from: None, requires: None,
            iterate_over: Some("ports"), db_action: Some("target_add"),
        },
    ];

    let pipeline_steps: Vec<PipelineStep> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| PipelineStep {
            id: s.id.to_string(),
            step_type: s.step_type.to_string(),
            tool_name: s.name.to_string(),
            tool_id: String::new(),
            command_template: s.cmd.to_string(),
            args: s.args.iter().map(|a| a.to_string()).collect(),
            params: serde_json::json!({}),
            input_from: s.input_from.map(|v| v.to_string()),
            exec_mode: "sequential".to_string(),
            requires: s.requires.map(|v| v.to_string()),
            iterate_over: s.iterate_over.map(|v| v.to_string()),
            db_action: s.db_action.map(|v| v.to_string()),
            on_failure: "continue".to_string(),
            timeout_secs: None,
            sub_pipeline: None,
            inline_pipeline: None,
            foreach_source: None,
            max_parallel: None,
            x: (i as f64) * 220.0 + 40.0,
            y: 80.0,
        })
        .collect();

    // DAG connections: Layer 0 (dig, subfinder, naabu) → Layer 1 (httpx, whatweb) → Layer 2 (katana)
    let connections: Vec<PipelineConnection> = vec![
        // port_scan feeds into http_probe and tech_fingerprint
        PipelineConnection { from_step: "port_scan".into(), to_step: "http_probe".into(), condition: None },
        PipelineConnection { from_step: "port_scan".into(), to_step: "tech_fingerprint".into(), condition: None },
        PipelineConnection { from_step: "http_probe".into(), to_step: "web_crawl".into(), condition: None },
        PipelineConnection { from_step: "tech_fingerprint".into(), to_step: "web_crawl".into(), condition: None },
    ];

    Pipeline {
        id: "recon_basic".to_string(),
        name: "Basic Reconnaissance".to_string(),
        description: "DNS, subdomains, port scan, HTTP probe, tech fingerprint, web crawl (katana). Use {target} as placeholder.".to_string(),
        is_template: false,
        workflow_id: Some("recon_basic".to_string()),
        steps: pipeline_steps,
        connections,
        created_at: 0,
        updated_at: 0,
    }
}

