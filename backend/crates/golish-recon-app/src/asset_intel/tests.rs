use super::*;

fn fake_runtime() -> golish_pentest::models::AssetIntelRuntimeConfig {
    golish_pentest::models::AssetIntelRuntimeConfig::CliJson {
        skill_id: "company-default-json".into(),
        timeout_secs: 30,
        artifact_globs: vec![],
        arg_bindings: std::collections::HashMap::new(),
    }
}

fn fake_normalize_config() -> golish_pentest::models::AssetIntelNormalizeConfig {
    golish_pentest::models::AssetIntelNormalizeConfig {
        organization: vec![golish_pentest::models::AssetIntelNormalizeRule {
            path: "$..invest[*]".into(),
            label: golish_pentest::models::AssetIntelFieldRef::Field("name".into()),
            value: golish_pentest::models::AssetIntelFieldRef::Field("name".into()),
            confidence: 0.82,
            when: vec![],
        }],
        target: vec![golish_pentest::models::AssetIntelNormalizeRule {
            path: "$..icp[*]".into(),
            label: golish_pentest::models::AssetIntelFieldRef::Field("domain".into()),
            value: golish_pentest::models::AssetIntelFieldRef::FirstOf(vec![
                "domain".into(),
                "url".into(),
            ]),
            confidence: 0.78,
            when: vec![],
        }],
        profile_fields: vec![],
    }
}

fn org_candidate_with_raw(name: &str, scale: &str, status: &str) -> OrganizationCandidate {
    OrganizationCandidate {
        id: format!("org:enscan-go:{name}"),
        kind: OrganizationCandidateKind::Organization,
        label: name.into(),
        value: name.into(),
        source: "enscan-go".into(),
        confidence: 0.82,
        status: "needs_review".into(),
        evidence: serde_json::json!({
            "provider": "enscan-go",
            "runId": "run-test",
            "raw": {
                "name": name,
                "scale": scale,
                "status": status,
                "pid": format!("pid-{name}")
            }
        }),
        created_at: 1,
    }
}

fn auto_promote_policy() -> golish_pentest::models::AssetIntelDiscoveryConfig {
    use golish_pentest::models::{AssetIntelNormalizeFilter, AssetIntelNormalizeFilterOp};
    golish_pentest::models::AssetIntelDiscoveryConfig {
        auto_promote: true,
        promote_when: vec![
            AssetIntelNormalizeFilter {
                field: "scale".into(),
                op: AssetIntelNormalizeFilterOp::Gte,
                value: "51".into(),
            },
            AssetIntelNormalizeFilter {
                field: "status".into(),
                op: AssetIntelNormalizeFilterOp::Contains,
                value: "开业".into(),
            },
        ],
        ownership_field: "scale".into(),
        dedupe_by: vec!["pid".into(), "name".into()],
    }
}

#[test]
fn provider_output_is_trusted_only_for_successful_terminal_states() {
    assert!(provider_output_is_trusted(&AssetIntelProviderRunStatus {
        provider_id: "enscan-go".into(),
        status: AssetIntelProviderRunState::Completed,
        message: "ok".into(),
    }));
    assert!(provider_output_is_trusted(&AssetIntelProviderRunStatus {
        provider_id: "enscan-go".into(),
        status: AssetIntelProviderRunState::CheckedEmpty,
        message: "empty".into(),
    }));
    assert!(!provider_output_is_trusted(&AssetIntelProviderRunStatus {
        provider_id: "enscan-go-tyc-discovery".into(),
        status: AssetIntelProviderRunState::Failed,
        message: "command failed after emitting partial stdout".into(),
    }));
    assert!(!provider_output_is_trusted(&AssetIntelProviderRunStatus {
        provider_id: "enscan-go-kc-discovery".into(),
        status: AssetIntelProviderRunState::Unavailable,
        message: "missing credentials".into(),
    }));
}

#[test]
fn asset_intel_provider_descriptors_load_from_tool_configs() {
    let tool = golish_pentest::models::ToolConfig {
        id: "fake-intel".into(),
        name: "Fake Intel".into(),
        asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
            enabled: true,
            provider_id: "fake-provider".into(),
            display_name: "Fake Provider".into(),
            capabilities: vec!["domains".into(), "apps".into()],
            requires_integration: Some(golish_pentest::models::AssetIntelIntegrationRequirement {
                tool_id: "fake-intel".into(),
                group_ids: vec!["default".into()],
            }),
            auto: golish_pentest::models::AssetIntelAutoConfig {
                default: true,
                priority: 10,
            },
            runtime: fake_runtime(),
            normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
            discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
            lookup: None,
        }),
        ..Default::default()
    };

    let providers = provider_descriptors_from_tools(&[tool]);

    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "fake-provider");
    assert_eq!(providers[0].display_name, "Fake Provider");
    assert_eq!(
        providers[0].requires_integration,
        Some(AssetIntelIntegrationRequirement {
            tool_id: "fake-intel".into(),
            group_ids: vec!["default".into()],
        })
    );
    assert!(providers[0]
        .capabilities
        .contains(&AssetIntelCapability::Domains));
    assert!(providers[0]
        .capabilities
        .contains(&AssetIntelCapability::Apps));
}

#[test]
fn normalize_provider_records_splits_candidates_and_preserves_evidence() {
    let candidates = normalize_provider_records(
        "mock",
        "run-1",
        123,
        vec![
            AssetIntelProviderRecord {
                kind: OrganizationCandidateKind::Organization,
                label: "Acme Subsidiary".into(),
                value: "Acme Subsidiary".into(),
                confidence: 0.86,
                evidence: serde_json::json!({"raw": {"ownership": 51}}),
            },
            AssetIntelProviderRecord {
                kind: OrganizationCandidateKind::Target,
                label: "api.acme.test".into(),
                value: "api.acme.test".into(),
                confidence: 0.72,
                evidence: serde_json::json!({"raw": {"type": "domain"}}),
            },
        ],
    );

    assert_eq!(candidates.organizations.len(), 1);
    assert_eq!(candidates.targets.len(), 1);
    assert_eq!(candidates.organizations[0].source, "mock");
    assert_eq!(candidates.organizations[0].status, "needs_review");
    assert_eq!(candidates.organizations[0].created_at, 123);
    assert_eq!(candidates.organizations[0].evidence["provider"], "mock");
    assert_eq!(candidates.organizations[0].evidence["runId"], "run-1");
    assert_eq!(candidates.targets[0].id, "target:mock:api.acme.test");
}

#[test]
fn auto_promote_child_decisions_only_promote_active_controlled_investments() {
    let candidates = OrganizationCandidates {
        organizations: vec![
            org_candidate_with_raw("平安信托有限责任公司", "99.880923%", "开业"),
            org_candidate_with_raw("平安证券股份有限公司", "40.9596%", "开业"),
            org_candidate_with_raw("注销分支", "100%", "注销"),
            org_candidate_with_raw("已存在子公司", "100%", "开业"),
        ],
        targets: vec![],
    };
    let existing = HashSet::from(["已存在子公司".to_string()]);
    let policy = auto_promote_policy();

    let decisions = auto_promote_child_decisions(&candidates, &policy, &existing);

    assert_eq!(decisions.iter().filter(|item| item.promote).count(), 1);
    assert_eq!(decisions[0].candidate.value, "平安信托有限责任公司");
    assert_eq!(decisions[0].ownership_percent, Some(99.880923));
    assert_eq!(
        decisions
            .iter()
            .filter_map(|item| item.reason.as_ref())
            .collect::<Vec<_>>(),
        vec![
            &AutoPromoteSkipReason::OwnershipBelowThreshold,
            &AutoPromoteSkipReason::InactiveStatus,
            &AutoPromoteSkipReason::Duplicate,
        ]
    );
}

#[test]
fn clear_engagement_candidates_preserves_engagement_metadata() {
    let intel = serde_json::json!({
        "engagement": {
            "mode": "discover_assets",
            "lookup_match": { "name": "中国平安保险（集团）股份有限公司" },
            "candidates": {
                "organizations": [{ "id": "org:enscan-go:old", "value": "old" }],
                "targets": [{ "id": "target:enscan-go:old", "value": "old.example" }]
            }
        },
        "contacts": {
            "email": ["ir@example.test"]
        }
    });

    let cleared = clear_engagement_candidates_from_intel(intel).unwrap();

    assert_eq!(cleared["engagement"]["mode"], "discover_assets");
    assert_eq!(
        cleared["engagement"]["lookup_match"]["name"],
        "中国平安保险（集团）股份有限公司"
    );
    assert!(cleared["engagement"].get("candidates").is_none());
    assert_eq!(cleared["contacts"]["email"][0], "ir@example.test");
}

#[test]
fn json_descriptor_normalizer_maps_nested_candidate_buckets() {
    let normalize = fake_normalize_config();
    let raw = serde_json::json!({
        "payload": {
            "invest": [{ "name": "小米科技有限责任公司" }],
            "icp": [{ "domain": "mi.com" }]
        }
    });

    let (candidates, profile) =
        normalize_json_with_descriptor("fake", "run-1", 123, &normalize, &raw);

    assert_eq!(candidates.organizations.len(), 1);
    assert_eq!(candidates.organizations[0].label, "小米科技有限责任公司");
    assert_eq!(candidates.organizations[0].source, "fake");
    assert_eq!(candidates.targets.len(), 1);
    assert_eq!(candidates.targets[0].value, "mi.com");
    assert_eq!(candidates.targets[0].confidence, 0.78);
    assert_eq!(candidates.targets[0].evidence["provider"], "fake");
    assert!(profile.is_empty(), "no profile_fields rules in fake config");
}

#[test]
fn fake_provider_json_data_dedupes_across_sources() {
    let normalize = fake_normalize_config();
    let first_raw = serde_json::json!({
        "payload": {
            "invest": [{ "name": "小米科技有限责任公司" }],
            "icp": [{ "domain": "mi.com" }, { "domain": "api.mi.com" }]
        }
    });
    let second_raw = serde_json::json!({
        "data": {
            "invest": [{ "name": "小米科技有限责任公司" }],
            "icp": [{ "domain": "MI.COM" }, { "domain": "store.mi.com" }]
        }
    });

    let (mut merged, _) =
        normalize_json_with_descriptor("fake-cli", "run-1", 1, &normalize, &first_raw);
    let (http_candidates, _) =
        normalize_json_with_descriptor("fake-http", "run-1", 2, &normalize, &second_raw);
    merge_candidates(&mut merged, http_candidates);

    assert_eq!(merged.organizations.len(), 1);
    assert_eq!(merged.organizations[0].source, "fake-cli");
    assert_eq!(
        merged
            .targets
            .iter()
            .map(|item| item.value.as_str())
            .collect::<Vec<_>>(),
        vec!["mi.com", "api.mi.com", "store.mi.com"]
    );
}

#[tokio::test]
async fn http_json_runtime_posts_fake_data_and_normalizes_candidates() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let mut bytes = Vec::new();
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            let req = String::from_utf8_lossy(&bytes);
            if req.contains("query_type=domain") {
                break;
            }
        }
        let req = String::from_utf8_lossy(&bytes);
        assert!(req.starts_with("POST / HTTP/1.1"));
        assert!(req.contains("query=%E5%B0%8F%E7%B1%B3"));
        assert!(req.contains("query_type=domain"));

        let body = serde_json::json!({
            "code": 0,
            "data": [
                { "domain": "mi.com", "title": "Xiaomi" },
                { "domain": "api.mi.com", "title": "Xiaomi API" }
            ],
            "message": "ok"
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let mut form = std::collections::HashMap::new();
    form.insert("query".to_string(), "{{company_name}}".to_string());
    form.insert("query_type".to_string(), "domain".to_string());
    let tool = ToolConfig {
        id: "fake-http".into(),
        name: "Fake HTTP".into(),
        asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
            enabled: true,
            provider_id: "fake-http".into(),
            display_name: "Fake HTTP".into(),
            capabilities: vec!["domains".into()],
            requires_integration: None,
            auto: golish_pentest::models::AssetIntelAutoConfig {
                default: true,
                priority: 1,
            },
            runtime: golish_pentest::models::AssetIntelRuntimeConfig::HttpJson {
                requests: vec![golish_pentest::models::AssetIntelHttpRequest {
                    id: "domains".into(),
                    method: "POST".into(),
                    url,
                    headers: std::collections::HashMap::new(),
                    form,
                    json: Value::Null,
                    timeout_secs: 5,
                }],
            },
            normalize: golish_pentest::models::AssetIntelNormalizeConfig {
                organization: vec![],
                target: vec![golish_pentest::models::AssetIntelNormalizeRule {
                    path: "$..data[*]".into(),
                    label: golish_pentest::models::AssetIntelFieldRef::Field("title".into()),
                    value: golish_pentest::models::AssetIntelFieldRef::Field("domain".into()),
                    confidence: 0.72,
                    when: vec![],
                }],
                profile_fields: vec![],
            },
            discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
            lookup: None,
        }),
        ..Default::default()
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://golish:golish@127.0.0.1:1/golish")
        .unwrap();
    let project_root = tempfile::tempdir().unwrap();

    let (status, candidates, evidence, _profile) = run_http_json_provider(
        &pool,
        &tool,
        project_root.path(),
        "run-1",
        "小米",
        &AssetIntelHydrateConfig::default(),
        None,
    )
    .await
    .unwrap();
    server.await.unwrap();

    assert_eq!(status.status, AssetIntelProviderRunState::Completed);
    assert_eq!(candidates.targets.len(), 2);
    assert_eq!(candidates.targets[0].label, "Xiaomi");
    assert_eq!(candidates.targets[0].value, "mi.com");
    assert_eq!(evidence["candidateCount"], 2);
    let output_dir = project_root
        .path()
        .join(".golish/tool-output/asset-intel/run-1/fake-http");
    assert!(output_dir.join("raw/response-domains.json").exists());
    assert!(output_dir.join("manifest.json").exists());
}

#[tokio::test]
async fn http_json_runtime_treats_provider_code_error_as_failed() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();
        let body = serde_json::json!({
            "code": 1,
            "message": "该 API Key 不合法或不存在"
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let mut form = std::collections::HashMap::new();
    form.insert("query".to_string(), "{{company_name}}".to_string());
    let tool = ToolConfig {
        id: "fake-http".into(),
        name: "Fake HTTP".into(),
        asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
            enabled: true,
            provider_id: "fake-http".into(),
            display_name: "Fake HTTP".into(),
            capabilities: vec!["domains".into()],
            requires_integration: None,
            auto: golish_pentest::models::AssetIntelAutoConfig {
                default: true,
                priority: 1,
            },
            runtime: golish_pentest::models::AssetIntelRuntimeConfig::HttpJson {
                requests: vec![golish_pentest::models::AssetIntelHttpRequest {
                    id: "domain".into(),
                    method: "POST".into(),
                    url,
                    headers: std::collections::HashMap::new(),
                    form,
                    json: Value::Null,
                    timeout_secs: 5,
                }],
            },
            normalize: golish_pentest::models::AssetIntelNormalizeConfig {
                organization: vec![],
                target: vec![golish_pentest::models::AssetIntelNormalizeRule {
                    path: "$..data[*]".into(),
                    label: golish_pentest::models::AssetIntelFieldRef::Field("title".into()),
                    value: golish_pentest::models::AssetIntelFieldRef::Field("domain".into()),
                    confidence: 0.72,
                    when: vec![],
                }],
                profile_fields: vec![],
            },
            discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
            lookup: None,
        }),
        ..Default::default()
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://golish:golish@127.0.0.1:1/golish")
        .unwrap();
    let project_root = tempfile::tempdir().unwrap();

    let (status, candidates, evidence, _profile) = run_http_json_provider(
        &pool,
        &tool,
        project_root.path(),
        "run-provider-error",
        "中国平安",
        &AssetIntelHydrateConfig::default(),
        None,
    )
    .await
    .unwrap();
    server.await.unwrap();

    assert_eq!(status.status, AssetIntelProviderRunState::Failed);
    assert!(status.message.contains("API Key"));
    assert!(candidates.targets.is_empty());
    assert_eq!(evidence["state"], "failed");
    assert_eq!(evidence["reason"], "unauthorized");
    assert_eq!(evidence["providerCode"], "1");
    let manifest_path = evidence["manifestPath"]
        .as_str()
        .expect("manifestPath present");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["status"], "failed");
    assert_eq!(manifest["errors"][0]["code"], "unauthorized");
}

#[derive(Debug, Default, Clone)]
struct RecordingEmitter {
    events: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
}

impl golish_core::EventEmitter for RecordingEmitter {
    fn emit_json(&self, event: &str, payload: Value) {
        self.events
            .lock()
            .unwrap()
            .push((event.to_string(), payload));
    }
}

impl RecordingEmitter {
    fn snapshot(&self) -> Vec<(String, Value)> {
        self.events.lock().unwrap().clone()
    }

    fn handle(&self) -> EventEmitterHandle {
        EventEmitterHandle::new(self.clone())
    }
}

#[cfg(unix)]
#[tokio::test]
async fn cli_json_runtime_streams_progress_and_artifact_batches() {
    use std::os::unix::fs::PermissionsExt;

    let tools_dir = tempfile::tempdir().unwrap();
    let project_root = tempfile::tempdir().unwrap();
    let executable = tools_dir.path().join("fake-asset-intel.sh");
    // Fake CLI:
    //   1) emit a progress line on stdout (non-JSON → progress event)
    //   2) write icp.json → artifact watcher should observe it
    //   3) sleep > ARTIFACT_POLL_INTERVAL so the watcher polls
    //   4) write app.json → another artifact batch
    //   5) emit another progress line + exit 0
    fs::write(
        &executable,
        r#"#!/bin/sh
echo "[stage] collecting icp"
printf '%s' '{"payload":{"icp":[{"domain":"a.example"}]}}' > "$(pwd)/icp.json"
sleep 0.8
echo "[stage] collecting app"
printf '%s' '{"payload":{"icp":[{"domain":"b.example"}]}}' > "$(pwd)/app.json"
sleep 0.8
echo "[stage] done"
"#,
    )
    .unwrap();
    let mut perms = fs::metadata(&executable).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&executable, perms).unwrap();

    let tool = ToolConfig {
        id: "fake-stream".into(),
        name: "Fake Stream".into(),
        executable: "fake-asset-intel.sh".into(),
        runtime: "native".into(),
        skills: vec![golish_pentest::models::ToolSkill {
            id: "company-default-json".into(),
            name: "Company JSON".into(),
            description: String::new(),
            args: String::new(),
            tags: vec![],
        }],
        asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
            enabled: true,
            provider_id: "fake-stream".into(),
            display_name: "Fake Stream".into(),
            capabilities: vec!["domains".into()],
            requires_integration: None,
            auto: golish_pentest::models::AssetIntelAutoConfig {
                default: true,
                priority: 1,
            },
            runtime: fake_runtime(),
            normalize: fake_normalize_config(),
            discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
            lookup: None,
        }),
        ..Default::default()
    };

    let recorder = RecordingEmitter::default();
    let handle = recorder.handle();
    let (status, candidates, _evidence, _profile) = run_cli_json_provider(
        &tool,
        std::slice::from_ref(&tool),
        tools_dir.path(),
        project_root.path(),
        "run-stream",
        "Acme",
        &AssetIntelHydrateConfig::default(),
        Some(&handle),
    )
    .await
    .unwrap();

    assert_eq!(status.status, AssetIntelProviderRunState::Completed);
    // dedup of a.example + b.example
    assert_eq!(candidates.targets.len(), 2);

    let events = recorder.snapshot();
    let names: Vec<&str> = events
        .iter()
        .filter_map(|(name, payload)| {
            if name == ASSET_INTEL_EVENT {
                payload.get("kind").and_then(|v| v.as_str())
            } else {
                None
            }
        })
        .collect();

    assert!(
        names.contains(&"provider_started"),
        "expected provider_started in {:?}",
        names
    );
    assert!(
        names
            .iter()
            .filter(|name| **name == "provider_progress")
            .count()
            >= 2,
        "expected at least 2 progress events (saw {:?})",
        names
    );
    let batch_events: Vec<&(String, Value)> = events
        .iter()
        .filter(|(_, payload)| {
            payload.get("kind").and_then(|v| v.as_str()) == Some("provider_batch")
        })
        .collect();
    assert!(
        !batch_events.is_empty(),
        "expected at least one provider_batch event (got events: {:?})",
        names
    );
    // every batch should carry source = "artifact" with an artifact path
    for (_, payload) in &batch_events {
        assert_eq!(
            payload.get("source").and_then(|v| v.as_str()),
            Some("artifact"),
            "batch should originate from artifact (payload={:?})",
            payload
        );
        assert!(
            payload
                .get("artifact")
                .and_then(|v| v.as_str())
                .map(|p| p.ends_with(".json"))
                .unwrap_or(false),
            "artifact path should be set (payload={:?})",
            payload
        );
    }
    assert!(
        names.contains(&"provider_completed"),
        "expected provider_completed in {:?}",
        names
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cli_json_runtime_runs_in_project_tool_output_dir() {
    use std::os::unix::fs::PermissionsExt;

    let tools_dir = tempfile::tempdir().unwrap();
    let project_root = tempfile::tempdir().unwrap();
    let executable = tools_dir.path().join("fake-asset-intel.sh");
    fs::write(
        &executable,
        r#"#!/bin/sh
case "$(pwd)" in
  */.golish/tool-output/asset-intel/run-cwd/fake-cli)
    printf '{"payload":{"icp":[{"domain":"cwd.example","title":"CWD OK"}]}}'
    ;;
  *)
    echo "bad cwd: $(pwd)" >&2
    exit 2
    ;;
esac
"#,
    )
    .unwrap();
    let mut perms = fs::metadata(&executable).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&executable, perms).unwrap();

    let tool = ToolConfig {
        id: "fake-cli".into(),
        name: "Fake CLI".into(),
        executable: "fake-asset-intel.sh".into(),
        runtime: "native".into(),
        skills: vec![golish_pentest::models::ToolSkill {
            id: "company-default-json".into(),
            name: "Company JSON".into(),
            description: String::new(),
            args: String::new(),
            tags: vec![],
        }],
        asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
            enabled: true,
            provider_id: "fake-cli".into(),
            display_name: "Fake CLI".into(),
            capabilities: vec!["domains".into()],
            requires_integration: None,
            auto: golish_pentest::models::AssetIntelAutoConfig {
                default: true,
                priority: 1,
            },
            runtime: fake_runtime(),
            normalize: fake_normalize_config(),
            discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
            lookup: None,
        }),
        ..Default::default()
    };

    let (status, candidates, evidence, _profile) = run_cli_json_provider(
        &tool,
        std::slice::from_ref(&tool),
        tools_dir.path(),
        project_root.path(),
        "run-cwd",
        "Acme",
        &AssetIntelHydrateConfig::default(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(status.status, AssetIntelProviderRunState::Completed);
    assert_eq!(candidates.targets.len(), 1);
    assert_eq!(candidates.targets[0].value, "cwd.example");
    assert!(evidence["outDir"]
        .as_str()
        .is_some_and(|path| path.ends_with(".golish/tool-output/asset-intel/run-cwd/fake-cli")));
    let output_dir = project_root
        .path()
        .join(".golish/tool-output/asset-intel/run-cwd/fake-cli");
    assert!(output_dir.join("raw/stdout.log").exists());
    assert!(output_dir.join("raw/stderr.log").exists());
    assert!(output_dir.join("manifest.json").exists());
}

#[test]
fn asset_intel_skill_args_render_config_bindings() {
    let mut bindings = std::collections::HashMap::new();
    bindings.insert(
        "min_ownership_percent".to_string(),
        "-invest {{config.min_ownership_percent}}".to_string(),
    );
    bindings.insert("depth".to_string(), "-deep {{config.depth}}".to_string());
    bindings.insert("include_branches".to_string(), "-branch".to_string());

    let rendered = render_asset_intel_skill_args(
        "-n \"{{org}}\" -json -out-dir \"{{out_dir}}\"",
        "小米",
        &PathBuf::from("/tmp/golish-enscan"),
        &AssetIntelHydrateConfig {
            min_ownership_percent: Some("51".into()),
            depth: Some("2".into()),
            include_branches: Some(true),
            create_candidates: Some(true),
        },
        &bindings,
    );

    assert_eq!(
        split_command_args(&rendered),
        vec![
            "-n",
            "小米",
            "-json",
            "-out-dir",
            "/tmp/golish-enscan",
            "-invest",
            "51",
            "-deep",
            "2",
            "-branch",
        ]
    );
}

#[test]
fn select_asset_intel_providers_uses_json_auto_priority() {
    fn tool(id: &str, priority: i32, enabled: bool) -> ToolConfig {
        ToolConfig {
            id: id.into(),
            name: id.into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: id.into(),
                display_name: id.into(),
                capabilities: vec!["domains".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: enabled,
                    priority,
                },
                runtime: fake_runtime(),
                normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        }
    }

    let tools = vec![
        tool("low", 10, true),
        tool("high", 100, true),
        tool("off", 200, false),
    ];
    let selected = select_asset_intel_providers(&tools, &[]).unwrap();

    assert_eq!(
        selected
            .iter()
            .map(|tool| provider_id_for_tool(tool).unwrap())
            .collect::<Vec<_>>(),
        vec!["high".to_string(), "low".to_string()]
    );
}

/// Shared fixture for two-phase selector tests: 3 providers covering
/// the realistic mix we ship today.
/// - `enscan-go`: subsidiaries + domains (discovery-capable)
/// - `0.zone`:   domains + apps (enrichment-only)
/// - `legacy`:   domains, auto.default=false (excluded by auto filter)
fn two_phase_fixture_tools() -> Vec<ToolConfig> {
    fn tool(id: &str, caps: &[&str], priority: i32, auto_default: bool) -> ToolConfig {
        ToolConfig {
            id: id.into(),
            name: id.into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: id.into(),
                display_name: id.into(),
                capabilities: caps.iter().map(|s| (*s).to_string()).collect(),
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: auto_default,
                    priority,
                },
                runtime: fake_runtime(),
                normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        }
    }

    vec![
        tool("enscan-go", &["subsidiaries", "domains", "icp"], 100, true),
        tool("0.zone", &["domains", "apps", "contacts"], 90, true),
        tool("legacy", &["domains"], 50, false),
    ]
}

#[test]
fn select_subsidiary_providers_keeps_only_subsidiaries_capable_tools() {
    let tools = two_phase_fixture_tools();
    let selected = select_subsidiary_providers(&tools, &[]).unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|t| provider_id_for_tool(t).unwrap())
            .collect::<Vec<_>>(),
        vec!["enscan-go".to_string()],
        "only enscan-go declares the subsidiaries capability"
    );
}

fn multi_provider_tool(id: &str, providers: &[(&str, &[&str], bool, i32)]) -> ToolConfig {
    ToolConfig {
        id: id.into(),
        name: id.into(),
        executable: format!("{id}/bin"),
        asset_intel_providers: Some(
            providers
                .iter()
                .map(|(pid, caps, default, priority)| {
                    golish_pentest::models::AssetIntelToolConfig {
                        enabled: true,
                        provider_id: (*pid).into(),
                        display_name: (*pid).into(),
                        capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
                        requires_integration: None,
                        auto: golish_pentest::models::AssetIntelAutoConfig {
                            default: *default,
                            priority: *priority,
                        },
                        runtime: fake_runtime(),
                        normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                        discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                        lookup: None,
                    }
                })
                .collect(),
        ),
        ..Default::default()
    }
}

#[test]
fn select_subsidiary_providers_expands_multi_provider_tool() {
    let tool = multi_provider_tool(
        "multi",
        &[
            ("multi-hi", &["subsidiaries"], true, 100),
            ("multi-lo", &["subsidiaries"], true, 50),
        ],
    );
    let selected = select_subsidiary_providers(&[tool], &[]).unwrap();
    assert_eq!(selected.len(), 2);
    let ids: Vec<String> = selected
        .iter()
        .map(|t| provider_id_for_tool(t).unwrap())
        .collect();
    assert_eq!(ids, vec!["multi-hi".to_string(), "multi-lo".to_string()]);
}

#[test]
fn select_asset_intel_providers_treats_multi_provider_tool_as_single_pool() {
    // Tool A has two providers (priority 50 / 100); tool B has one (priority 75).
    // Expected sort across both tools: [100, 75, 50].
    let tool_a = multi_provider_tool(
        "multi",
        &[
            ("multi-low", &["subsidiaries"], true, 50),
            ("multi-high", &["subsidiaries"], true, 100),
        ],
    );
    let tool_b = ToolConfig {
        id: "single".into(),
        name: "single".into(),
        asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
            enabled: true,
            provider_id: "single-mid".into(),
            display_name: "single".into(),
            capabilities: vec!["subsidiaries".into()],
            requires_integration: None,
            auto: golish_pentest::models::AssetIntelAutoConfig {
                default: true,
                priority: 75,
            },
            runtime: fake_runtime(),
            normalize: Default::default(),
            discovery: Default::default(),
            lookup: None,
        }),
        ..Default::default()
    };
    let selected = select_asset_intel_providers(&[tool_a, tool_b], &[]).unwrap();
    let ids: Vec<String> = selected
        .iter()
        .map(|t| provider_id_for_tool(t).unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            "multi-high".to_string(),
            "single-mid".to_string(),
            "multi-low".to_string(),
        ]
    );
}

#[test]
fn provider_descriptors_from_tools_unpacks_multi_provider_tool() {
    let tool = multi_provider_tool(
        "multi",
        &[
            ("multi-a", &["subsidiaries"], true, 100),
            ("multi-b", &["domains"], false, 50),
        ],
    );
    let descriptors = provider_descriptors_from_tools(&[tool]);
    assert_eq!(descriptors.len(), 2);
    let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
    assert!(ids.contains(&"multi-a"));
    assert!(ids.contains(&"multi-b"));
}

#[test]
fn expand_provider_tools_clones_each_provider_into_virtual_tool() {
    let tool = multi_provider_tool(
        "shared",
        &[
            ("shared", &["subsidiaries"], true, 100),
            ("shared-alt", &["subsidiaries"], false, 50),
        ],
    );
    let expanded = expand_provider_tools(&[tool]);
    assert_eq!(expanded.len(), 2);
    assert_eq!(provider_id_for_tool(&expanded[0]).unwrap(), "shared");
    assert_eq!(provider_id_for_tool(&expanded[1]).unwrap(), "shared-alt");
    assert_eq!(expanded[0].executable, "shared/bin");
    assert_eq!(expanded[1].executable, "shared/bin");
    assert!(
        expanded[0].asset_intel_providers.is_none(),
        "virtual tool must not carry providers vec"
    );
    assert!(
        expanded[1].asset_intel_providers.is_none(),
        "virtual tool must not carry providers vec"
    );
}

#[test]
fn expand_provider_tools_passes_single_asset_intel_tool_through_unchanged() {
    let tools = two_phase_fixture_tools();
    let expanded = expand_provider_tools(&tools);
    assert_eq!(
        expanded
            .iter()
            .map(|t| provider_id_for_tool(t).unwrap())
            .collect::<Vec<_>>(),
        vec![
            "enscan-go".to_string(),
            "0.zone".to_string(),
            "legacy".to_string(),
        ],
        "single-provider tools must be cloned 1:1 in scan order"
    );
}

#[test]
fn expand_provider_tools_skips_disabled_providers() {
    let mut tool = multi_provider_tool(
        "shared",
        &[
            ("off", &["subsidiaries"], true, 1),
            ("on", &["subsidiaries"], true, 1),
        ],
    );
    // Mark the first provider disabled so the helper exercises the enabled filter.
    if let Some(providers) = tool.asset_intel_providers.as_mut() {
        providers[0].enabled = false;
    }
    let expanded = expand_provider_tools(&[tool]);
    assert_eq!(expanded.len(), 1);
    assert_eq!(provider_id_for_tool(&expanded[0]).unwrap(), "on");
}

#[test]
fn fixture_discovery_auto_defaults_skip_tyc_until_upstream_is_stable() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let toolsconfig_dir = std::path::PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("..")
        .join("resources")
        .join("toolsconfig");
    if !toolsconfig_dir.exists() {
        eprintln!(
            "fixture skipped: toolsconfig dir not found at {}",
            toolsconfig_dir.display()
        );
        return;
    }
    let scan = golish_pentest::scan_toolsconfig(&toolsconfig_dir);
    assert!(
        scan.success,
        "toolsconfig scan failed: {:?}",
        scan.error.as_deref()
    );

    let selected = select_subsidiary_providers(&scan.tools, &[]).unwrap();

    assert_eq!(
        selected
            .iter()
            .map(|tool| provider_id_for_tool(tool).unwrap())
            .collect::<Vec<_>>(),
        vec![
            "enscan-go".to_string(),
            "enscan-go-kc-discovery".to_string(),
            "enscan-go-rb-discovery".to_string(),
        ],
        "default discovery should skip TYC while ENScan_GO v2.0.5 TYC discovery is unstable"
    );
}

#[test]
fn fixture_enrichment_profile_fields_cover_observed_provider_keys() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let toolsconfig_dir = std::path::PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("..")
        .join("resources")
        .join("toolsconfig");
    if !toolsconfig_dir.exists() {
        eprintln!(
            "fixture skipped: toolsconfig dir not found at {}",
            toolsconfig_dir.display()
        );
        return;
    }
    let scan = golish_pentest::scan_toolsconfig(&toolsconfig_dir);
    assert!(
        scan.success,
        "toolsconfig scan failed: {:?}",
        scan.error.as_deref()
    );
    fn has_rule(
        asset: &golish_pentest::models::AssetIntelToolConfig,
        path: &str,
        source: &str,
        target: &str,
        kind: golish_pentest::models::AssetIntelProfileFieldTarget,
    ) -> bool {
        asset.normalize.profile_fields.iter().any(|rule| {
            rule.path == path
                && rule.target_field == target
                && rule.target_kind == kind
                && matches!(
                    &rule.source_field,
                    golish_pentest::models::AssetIntelFieldRef::Field(field) if field == source
                )
        })
    }

    let expanded = expand_provider_tools(&scan.tools);
    let zone = expanded
        .iter()
        .find(|tool| provider_id_for_tool(tool).as_deref() == Some("0.zone"))
        .and_then(|tool| tool.asset_intel.as_ref())
        .expect("0.zone provider fixture");
    let enscan = expanded
        .iter()
        .find(|tool| provider_id_for_tool(tool).as_deref() == Some("enscan-go-enrichment"))
        .and_then(|tool| tool.asset_intel.as_ref())
        .expect("ENScan enrichment provider fixture");

    assert!(
        has_rule(
            zone,
            "$..data[*]",
            "ip",
            "ip_ranges",
            golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
        ),
        "0.zone site.ip should hydrate organization ip_ranges"
    );
    assert!(
        has_rule(
            zone,
            "$..data[*]",
            "asn",
            "asns",
            golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
        ),
        "0.zone site.asn should hydrate organization asns"
    );
    assert!(
        has_rule(
            zone,
            "$..data[*]",
            "msg.code",
            "credit_code",
            golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
        ),
        "0.zone org.msg.code should hydrate credit_code"
    );
    assert!(
        has_rule(
            enscan,
            "$..enterprise_info[*]",
            "scope",
            "business_scope",
            golish_pentest::models::AssetIntelProfileFieldTarget::Intel
        ),
        "ENScan enterprise scope should be preserved in intel"
    );
    assert!(
        has_rule(
            enscan,
            "$..icp[*]",
            "icp",
            "icp_records",
            golish_pentest::models::AssetIntelProfileFieldTarget::Intel
        ),
        "ENScan ICP license number should be preserved in intel"
    );

    let credit_rule = zone
        .normalize
        .profile_fields
        .iter()
        .find(|rule| {
            rule.target_field == "credit_code"
                && matches!(
                    &rule.source_field,
                    golish_pentest::models::AssetIntelFieldRef::Field(field) if field == "msg.code"
                )
        })
        .expect("0.zone msg.code -> credit_code rule must exist");
    assert!(
        credit_rule.when.iter().any(|clause| {
            clause.field == "name_cn"
                && matches!(
                    clause.op,
                    golish_pentest::models::AssetIntelNormalizeFilterOp::Exists
                )
        }),
        "0.zone msg.code -> credit_code must require name_cn presence to avoid pulling \
             apk/site/domain msg.code values into the master organization profile"
    );

    for target_kind in [
        golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
        golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
        golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
    ] {
        for rule in zone.normalize.profile_fields.iter() {
            if rule.target_kind != target_kind {
                continue;
            }
            if !matches!(
                &rule.source_field,
                golish_pentest::models::AssetIntelFieldRef::Field(field)
                    if matches!(
                        field.as_str(),
                        "msg.industry"
                        | "msg.legal_person"
                        | "msg.reg_address"
                        | "msg.reg_time"
                        | "msg.capital"
                        | "msg.business"
                        | "msg.email[0]"
                        | "msg.contact_number"
                        | "msg.website[0]"
                    )
            ) {
                continue;
            }
            assert!(
                rule.when.iter().any(|clause| {
                    clause.field == "name_cn"
                        && matches!(
                            clause.op,
                            golish_pentest::models::AssetIntelNormalizeFilterOp::Exists
                        )
                }),
                "0.zone {:?} -> {} rule must require name_cn presence (org-only field), \
                     otherwise apk/site/domain records can pollute the master record",
                rule.source_field,
                rule.target_field
            );
        }
    }

    assert!(
        !zone
            .normalize
            .profile_fields
            .iter()
            .any(|rule| rule.target_field == "certificates"),
        "0.zone must not map ssl_certificate (a static-asset URL) into organization \
             certificates; revisit when we add a real cert subject extractor"
    );

    let apk_rule = zone
        .normalize
        .profile_fields
        .iter()
        .find(|rule| {
            rule.target_field == "mobile_apps"
                && matches!(
                    &rule.source_field,
                    golish_pentest::models::AssetIntelFieldRef::FirstOf(items)
                        if items.iter().any(|s| s == "msg.app_url")
                )
        })
        .expect("0.zone apk -> mobile_apps rule must exist");
    if let golish_pentest::models::AssetIntelFieldRef::FirstOf(items) = &apk_rule.source_field {
        assert!(
            !items.iter().any(|s| s == "title"),
            "0.zone apk -> mobile_apps must NOT fall back to `title` \
                 (网页 SEO 标题被误塞进 business systems 是上轮发现的 bug)"
        );
    }
}

#[test]
fn select_enrichment_providers_excludes_subsidiaries_capable_tools() {
    let tools = two_phase_fixture_tools();
    let selected = select_enrichment_providers(&tools, &[]).unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|t| provider_id_for_tool(t).unwrap())
            .collect::<Vec<_>>(),
        vec!["0.zone".to_string()],
        "0.zone is the only auto-default non-subsidiaries provider"
    );
}

#[test]
fn enrichment_config_disables_candidate_queue_writes() {
    let config = enrichment_hydrate_config(AssetIntelHydrateConfig {
        min_ownership_percent: Some("35".into()),
        depth: Some("2".into()),
        include_branches: Some(true),
        create_candidates: Some(true),
    });

    assert_eq!(config.min_ownership_percent.as_deref(), Some("35"));
    assert_eq!(config.depth.as_deref(), Some("2"));
    assert_eq!(config.include_branches, Some(true));
    assert_eq!(config.create_candidates, Some(false));
}

#[test]
fn enrich_organization_config_disables_candidate_queue_writes() {
    let args = AssetIntelEnrichOrganizationArgs {
        organization_id: Uuid::new_v4().to_string(),
        provider_ids: Vec::new(),
        config: AssetIntelHydrateConfig {
            min_ownership_percent: Some("35".into()),
            depth: Some("2".into()),
            include_branches: Some(true),
            create_candidates: Some(true),
        },
    };

    let config = enrichment_hydrate_config_for_organization(&args);

    assert_eq!(config.min_ownership_percent.as_deref(), Some("35"));
    assert_eq!(config.depth.as_deref(), Some("2"));
    assert_eq!(config.include_branches, Some(true));
    assert_eq!(config.create_candidates, Some(false));
}

#[test]
fn select_subsidiary_providers_rejects_explicit_request_for_enrichment_tool() {
    let tools = two_phase_fixture_tools();
    let err = select_subsidiary_providers(&tools, &["0.zone".to_string()])
        .expect_err("requesting 0.zone for discovery must reject");
    let msg = format!("{err}");
    assert!(
        msg.contains("subsidiaries") && msg.contains("0.zone"),
        "error should mention both the missing capability and the offending provider, got: {msg}"
    );
}

#[test]
fn select_enrichment_providers_rejects_explicit_request_for_subsidiaries_tool() {
    let tools = two_phase_fixture_tools();
    let err = select_enrichment_providers(&tools, &["enscan-go".to_string()])
        .expect_err("requesting enscan-go for enrichment must reject");
    let msg = format!("{err}");
    assert!(
        msg.contains("discovery") && msg.contains("enscan-go"),
        "error should direct caller to hydrate_subsidiaries, got: {msg}"
    );
}

#[test]
fn provider_has_subsidiaries_is_case_insensitive() {
    let tool = ToolConfig {
        id: "casing".into(),
        name: "casing".into(),
        asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
            enabled: true,
            provider_id: "casing".into(),
            display_name: "casing".into(),
            capabilities: vec!["Subsidiaries".into()],
            requires_integration: None,
            auto: golish_pentest::models::AssetIntelAutoConfig {
                default: true,
                priority: 1,
            },
            runtime: fake_runtime(),
            normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
            discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
            lookup: None,
        }),
        ..Default::default()
    };
    assert!(
        provider_has_subsidiaries(&tool),
        "capability matching must be case-insensitive so JSON authors don't get bit"
    );
}

#[test]
fn normalize_when_filter_drops_low_ownership_invest_rows() {
    let mut normalize = fake_normalize_config();
    // The org rule covers `$..invest[*]` already; layer a numeric filter
    // that only keeps rows with `scale >= 51`. Anything below should drop
    // out of the candidate pool entirely.
    normalize.organization[0].when = vec![golish_pentest::models::AssetIntelNormalizeFilter {
        field: "scale".into(),
        op: golish_pentest::models::AssetIntelNormalizeFilterOp::Gte,
        value: "51".into(),
    }];
    let raw = serde_json::json!({
        "payload": {
            "invest": [
                { "name": "全资子公司", "scale": "100" },
                { "name": "少数股权",   "scale": "5"   },
                { "name": "缺字段公司"                  },
            ]
        }
    });

    let (candidates, _profile) =
        normalize_json_with_descriptor("filter-provider", "run-filter", 99, &normalize, &raw);

    assert_eq!(
        candidates
            .organizations
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>(),
        vec!["全资子公司"],
        "only rows passing scale>=51 should remain"
    );
}

#[test]
fn normalize_when_filter_contains_op_keeps_matching_rows() {
    let mut normalize = fake_normalize_config();
    normalize.organization[0].when = vec![golish_pentest::models::AssetIntelNormalizeFilter {
        field: "entity_type".into(),
        op: golish_pentest::models::AssetIntelNormalizeFilterOp::Contains,
        value: "公司".into(),
    }];
    let raw = serde_json::json!({
        "data": {
            "invest": [
                { "name": "测试有限公司", "entity_type": "有限责任公司" },
                { "name": "个体张三",      "entity_type": "个体工商户"   },
            ]
        }
    });

    let (candidates, _profile) =
        normalize_json_with_descriptor("filter-provider", "run-contains", 1, &normalize, &raw);

    assert_eq!(candidates.organizations.len(), 1);
    assert_eq!(candidates.organizations[0].label, "测试有限公司");
}

#[test]
fn normalize_when_filter_exists_drops_empty_fields() {
    let mut normalize = fake_normalize_config();
    normalize.organization[0].when = vec![golish_pentest::models::AssetIntelNormalizeFilter {
        field: "pid".into(),
        op: golish_pentest::models::AssetIntelNormalizeFilterOp::Exists,
        value: String::new(),
    }];
    let raw = serde_json::json!({
        "data": {
            "invest": [
                { "name": "已知 pid", "pid": "abc" },
                { "name": "缺 pid"                 },
                { "name": "空 pid",   "pid": ""    },
            ]
        }
    });

    let (candidates, _profile) =
        normalize_json_with_descriptor("filter-provider", "run-exists", 1, &normalize, &raw);

    assert_eq!(candidates.organizations.len(), 1);
    assert_eq!(candidates.organizations[0].label, "已知 pid");
}

#[test]
fn extract_profile_field_entries_scalar_intel_contact_buckets() {
    let rules = vec![
        golish_pentest::models::AssetIntelProfileFieldRule {
            path: "$..enterprise_info[*]".into(),
            source_field: golish_pentest::models::AssetIntelFieldRef::Field("reg_code".into()),
            target_field: "credit_code".into(),
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            transform: golish_pentest::models::AssetIntelProfileFieldTransform::None,
            when: vec![],
        },
        golish_pentest::models::AssetIntelProfileFieldRule {
            path: "$..enterprise_info[*]".into(),
            source_field: golish_pentest::models::AssetIntelFieldRef::Field("legal".into()),
            target_field: "legal_representative".into(),
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            transform: golish_pentest::models::AssetIntelProfileFieldTransform::Trim,
            when: vec![],
        },
        golish_pentest::models::AssetIntelProfileFieldRule {
            path: "$..enterprise_info[*]".into(),
            source_field: golish_pentest::models::AssetIntelFieldRef::Field("email".into()),
            target_field: "email".into(),
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
            transform: golish_pentest::models::AssetIntelProfileFieldTransform::Lower,
            when: vec![],
        },
        golish_pentest::models::AssetIntelProfileFieldRule {
            path: "$..enterprise_info[*]".into(),
            source_field: golish_pentest::models::AssetIntelFieldRef::Field("phone".into()),
            target_field: "phone".into(),
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
            transform: golish_pentest::models::AssetIntelProfileFieldTransform::None,
            when: vec![],
        },
    ];
    let raw = serde_json::json!({
        "payload": {
            "enterprise_info": [
                {
                    "name": "小米科技",
                    "reg_code": "91110108551385082Q",
                    "legal": "  雷军  ",
                    "email": "Press@MI.com",
                    "phone": "010-12345678"
                }
            ]
        }
    });

    let entries = extract_profile_field_entries(&rules, &raw);

    assert_eq!(entries.len(), 4);
    let by_field: HashMap<_, _> = entries
        .iter()
        .map(|e| (e.target_field.as_str(), e.value.as_str()))
        .collect();
    assert_eq!(by_field["credit_code"], "91110108551385082Q");
    assert_eq!(by_field["legal_representative"], "雷军"); // trim
    assert_eq!(by_field["email"], "press@mi.com"); // lower
    assert_eq!(by_field["phone"], "010-12345678");
}

#[test]
fn extract_profile_field_entries_when_filter_drops_placeholder_values() {
    // ENScan AQC returns "-" (single dash) as a placeholder for missing
    // email / phone. Without a `when` filter that placeholder would land
    // in organizations.intel.contacts.email and pollute the master record.
    let rules = vec![
        golish_pentest::models::AssetIntelProfileFieldRule {
            path: "$..enterprise_info[*]".into(),
            source_field: golish_pentest::models::AssetIntelFieldRef::Field("email".into()),
            target_field: "email".into(),
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
            transform: golish_pentest::models::AssetIntelProfileFieldTransform::Lower,
            when: vec![golish_pentest::models::AssetIntelNormalizeFilter {
                field: "email".into(),
                op: golish_pentest::models::AssetIntelNormalizeFilterOp::Ne,
                value: "-".into(),
            }],
        },
        golish_pentest::models::AssetIntelProfileFieldRule {
            path: "$..enterprise_info[*]".into(),
            source_field: golish_pentest::models::AssetIntelFieldRef::Field("phone".into()),
            target_field: "phone".into(),
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
            transform: golish_pentest::models::AssetIntelProfileFieldTransform::Trim,
            when: vec![golish_pentest::models::AssetIntelNormalizeFilter {
                field: "phone".into(),
                op: golish_pentest::models::AssetIntelNormalizeFilterOp::Ne,
                value: "-".into(),
            }],
        },
    ];
    let raw = serde_json::json!({
        "enterprise_info": [
            {
                // dash placeholders — both must drop out
                "email": "-",
                "phone": "-"
            },
            {
                // real values — must pass through
                "email": "Press@MI.com",
                "phone": "010-12345678"
            }
        ]
    });

    let entries = extract_profile_field_entries(&rules, &raw);

    assert_eq!(entries.len(), 2, "only the real-value row survives");
    assert_eq!(entries[0].target_field, "email");
    assert_eq!(entries[0].value, "press@mi.com");
    assert_eq!(entries[1].target_field, "phone");
    assert_eq!(entries[1].value, "010-12345678");
}

#[test]
fn build_profile_patch_first_wins_for_scalar_intel_contact_dedupes() {
    let entries = vec![
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "credit_code".into(),
            value: "AAA".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "credit_code".into(),
            value: "BBB".into(), // duplicate from another provider — must NOT overwrite
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "industry".into(),
            value: "互联网".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            target_field: "legal_representative".into(),
            value: "雷军".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
            target_field: "email".into(),
            value: "a@example.com".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
            target_field: "email".into(),
            value: "A@example.com".into(), // case-only diff → must dedupe
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
            target_field: "email".into(),
            value: "b@example.com".into(),
        },
    ];
    let existing_intel = serde_json::json!({
        "contacts": { "email": ["preexisting@example.com"] },
        "engagement": { "mode": "discover_assets" }
    });

    let patch = build_profile_patch_from_entries(&existing_intel, &entries)
        .expect("patch build ok")
        .expect("patch is Some when entries present");

    assert_eq!(patch.credit_code.as_deref(), Some("AAA"));
    assert_eq!(patch.industry.as_deref(), Some("互联网"));
    let intel = patch.intel.expect("intel patched");
    assert_eq!(
        intel["legal_representative"],
        serde_json::Value::String("雷军".into())
    );
    let emails = intel["contacts"]["email"].as_array().expect("email array");
    let strs: Vec<&str> = emails.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        strs,
        vec!["preexisting@example.com", "a@example.com", "b@example.com"]
    );
    // engagement metadata must survive
    assert_eq!(
        intel["engagement"]["mode"],
        serde_json::Value::String("discover_assets".into())
    );
}

#[test]
fn build_profile_patch_dedupes_multi_value_intel_fields() {
    let entries = vec![
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            target_field: "icp_records".into(),
            value: "粤ICP备06118290号-2".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            target_field: "icp_records".into(),
            value: "粤ICP备06118290号-2".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            target_field: "icp_records".into(),
            value: "粤ICP备06118290号-16".into(),
        },
    ];
    let existing_intel = serde_json::json!({
        "icp_records": ["粤ICP备06118290号-1"]
    });

    let patch = build_profile_patch_from_entries(&existing_intel, &entries)
        .expect("patch build ok")
        .expect("patch is Some when entries present");

    assert_eq!(
        patch.intel.expect("intel patched")["icp_records"],
        serde_json::json!([
            "粤ICP备06118290号-1",
            "粤ICP备06118290号-2",
            "粤ICP备06118290号-16"
        ])
    );
}

#[test]
fn build_profile_patch_dedupes_app_intel_array_fields() {
    let entries = vec![
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            target_field: "mobile_apps".into(),
            value: "小米实况麻将".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            target_field: "mobile_apps".into(),
            value: "小米实况麻将".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            target_field: "mini_programs".into(),
            value: "小米商城".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            target_field: "app_domains".into(),
            value: "https://com.dfwe".into(),
        },
    ];

    let patch = build_profile_patch_from_entries(&serde_json::json!({}), &entries)
        .expect("patch build ok")
        .expect("app intel entries should produce a patch");
    let intel = patch.intel.expect("intel patched");

    assert_eq!(intel["mobile_apps"], serde_json::json!(["小米实况麻将"]));
    assert_eq!(intel["mini_programs"], serde_json::json!(["小米商城"]));
    assert_eq!(
        intel["app_domains"],
        serde_json::json!(["https://com.dfwe"])
    );
}

#[test]
fn build_profile_patch_writes_visible_profile_array_fields() {
    let entries = vec![
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "domains".into(),
            value: "example.com".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "domains".into(),
            value: "EXAMPLE.com".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "email_domains".into(),
            value: "example.com".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "business_systems".into(),
            value: "Example App".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "social_accounts".into(),
            value: "wechat:example".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "contacts".into(),
            value: "ir@example.com".into(),
        },
    ];

    let patch = build_profile_patch_from_entries(&serde_json::json!({}), &entries)
        .expect("patch build ok")
        .expect("patch is Some when profile fields are present");

    assert_eq!(patch.domains, Some(serde_json::json!(["example.com"])));
    assert_eq!(
        patch.email_domains,
        Some(serde_json::json!(["example.com"]))
    );
    assert_eq!(
        patch.business_systems,
        Some(serde_json::json!(["Example App"]))
    );
    assert_eq!(
        patch.social_accounts,
        Some(serde_json::json!(["wechat:example"]))
    );
    assert_eq!(patch.contacts, Some(serde_json::json!(["ir@example.com"])));
}

#[test]
fn extract_profile_fields_normalizes_asn_values() {
    let rules = vec![golish_pentest::models::AssetIntelProfileFieldRule {
        path: "$..data[*]".into(),
        source_field: golish_pentest::models::AssetIntelFieldRef::Field("asn".into()),
        target_field: "asns".into(),
        target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
        transform: golish_pentest::models::AssetIntelProfileFieldTransform::Asn,
        when: vec![],
    }];
    let raw = serde_json::json!({
        "data": [
            { "asn": 4134 },
            { "asn": " as37963 " },
            { "asn": "not-an-asn" }
        ]
    });

    let entries = extract_profile_field_entries(&rules, &raw);
    let patch = build_profile_patch_from_entries(&serde_json::json!({}), &entries)
        .expect("patch build ok")
        .expect("asn entries should produce a patch");

    assert_eq!(patch.asns, Some(serde_json::json!(["AS4134", "AS37963"])));
}

#[test]
fn team_cymru_asn_lookup_builds_profile_entries_from_public_ips() {
    let entries = vec![
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "ip_ranges".into(),
            value: "183.62.123.10".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "ip_ranges".into(),
            value: "182.92.121.121".into(),
        },
        ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "ip_ranges".into(),
            value: "10.0.0.1".into(),
        },
    ];
    let response = "\
AS      | IP               | BGP Prefix          | CC | Registry | Allocated  | AS Name
4134    | 183.62.123.10    | 183.56.0.0/13       | CN | apnic    | 2009-09-29 | CHINANET-BACKBONE
37963   | 182.92.121.121   | 182.92.0.0/16       | CN | apnic    | 2013-08-16 | ALIBABA-CN-NET
";

    let ips = collect_public_ips_for_asn_lookup(&entries);
    let mappings = parse_team_cymru_asn_response(response);
    let derived = profile_asn_entries_from_mappings(&mappings);
    let patch = build_profile_patch_from_entries(&serde_json::json!({}), &derived)
        .expect("patch build ok")
        .expect("derived ASN entries should produce a patch");

    assert_eq!(
        ips.iter().map(ToString::to_string).collect::<Vec<_>>(),
        vec!["183.62.123.10", "182.92.121.121"]
    );
    assert_eq!(patch.asns, Some(serde_json::json!(["AS4134", "AS37963"])));
}

#[test]
fn build_profile_patch_returns_none_for_empty_or_blank_entries() {
    let entries = vec![ProfileFieldEntry {
        target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
        target_field: "credit_code".into(),
        value: "   ".into(),
    }];
    let intel = serde_json::json!({});
    let patch = build_profile_patch_from_entries(&intel, &entries).unwrap();
    assert!(
        patch.is_none(),
        "all-blank entries should not produce a patch"
    );
}

#[test]
fn extract_lookup_matches_maps_enterprise_info_into_disambiguation_rows() {
    let config = golish_pentest::models::AssetIntelLookupConfig {
        enabled: true,
        skill_id: "company-lookup-json".into(),
        timeout_secs: 60,
        normalize: golish_pentest::models::AssetIntelLookupNormalize {
            path: "$..enterprise_info[*]".into(),
            name: golish_pentest::models::AssetIntelFieldRef::Field("name".into()),
            credit_code: Some(golish_pentest::models::AssetIntelFieldRef::Field(
                "reg_code".into(),
            )),
            industry: Some(golish_pentest::models::AssetIntelFieldRef::Field(
                "industry".into(),
            )),
            legal_representative: Some(golish_pentest::models::AssetIntelFieldRef::FirstOf(vec![
                "legal_person".into(),
                "legal".into(),
            ])),
            address: Some(golish_pentest::models::AssetIntelFieldRef::FirstOf(vec![
                "reg_address".into(),
                "addr".into(),
            ])),
            registered_at: Some(golish_pentest::models::AssetIntelFieldRef::Field(
                "reg_date".into(),
            )),
            score: None,
            default_confidence: 0.68,
        },
    };
    let raw = serde_json::json!({
        "payload": {
            "enterprise_info": [
                {
                    "name": "小米科技有限责任公司",
                    "reg_code": "91110108551385082Q",
                    "industry": "互联网",
                    "legal_person": "雷军",
                    "reg_address": "北京市海淀区清河中街68号",
                    "reg_date": "2010-03-03"
                },
                {
                    "name": "小米通讯技术有限公司",
                    "reg_code": "91440300325990618B",
                    "legal": "回退法人字段",
                    "addr": "回退地址字段"
                }
            ]
        }
    });

    let matches = extract_lookup_matches("enscan-go", &config, &raw);

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].provider_id, "enscan-go");
    assert_eq!(matches[0].name, "小米科技有限责任公司");
    assert_eq!(
        matches[0].credit_code.as_deref(),
        Some("91110108551385082Q")
    );
    assert_eq!(matches[0].industry.as_deref(), Some("互联网"));
    assert_eq!(matches[0].legal_representative.as_deref(), Some("雷军"));
    assert_eq!(
        matches[0].address.as_deref(),
        Some("北京市海淀区清河中街68号")
    );
    assert_eq!(matches[0].registered_at.as_deref(), Some("2010-03-03"));
    assert!((matches[0].confidence - 0.68).abs() < f64::EPSILON);

    assert_eq!(matches[1].name, "小米通讯技术有限公司");
    assert_eq!(
        matches[1].legal_representative.as_deref(),
        Some("回退法人字段")
    );
    assert_eq!(matches[1].address.as_deref(), Some("回退地址字段"));
    assert!(
        matches[1].industry.is_none(),
        "missing field should stay None"
    );
    assert!(matches[1].registered_at.is_none());
}

#[test]
fn dedupe_lookup_matches_prefers_credit_code_for_uniqueness() {
    let m1 = LookupCompanyMatch {
        provider_id: "enscan-go".into(),
        name: "小米科技有限责任公司".into(),
        credit_code: Some("91110108551385082Q".into()),
        industry: None,
        legal_representative: None,
        address: None,
        registered_at: None,
        confidence: 0.68,
        evidence: serde_json::json!({}),
    };
    let m2 = LookupCompanyMatch {
        provider_id: "another".into(),
        // Different display name but same credit code → must dedupe.
        name: "Xiaomi Inc".into(),
        credit_code: Some("91110108551385082q".into()), // case differs
        industry: None,
        legal_representative: None,
        address: None,
        registered_at: None,
        confidence: 0.5,
        evidence: serde_json::json!({}),
    };
    let m3 = LookupCompanyMatch {
        provider_id: "enscan-go".into(),
        name: "Acme Inc".into(),
        credit_code: None,
        industry: None,
        legal_representative: None,
        address: None,
        registered_at: None,
        confidence: 0.42,
        evidence: serde_json::json!({}),
    };
    let m4 = LookupCompanyMatch {
        provider_id: "another".into(),
        name: "  acme inc  ".into(), // case + whitespace only diff → must dedupe
        credit_code: None,
        industry: None,
        legal_representative: None,
        address: None,
        registered_at: None,
        confidence: 0.3,
        evidence: serde_json::json!({}),
    };

    let deduped = dedupe_lookup_matches(vec![m1.clone(), m2, m3.clone(), m4]);

    assert_eq!(deduped.len(), 2);
    assert_eq!(deduped[0].provider_id, "enscan-go");
    assert_eq!(deduped[0].name, m1.name);
    assert_eq!(deduped[1].name, "Acme Inc");
}

#[test]
fn merge_candidates_dedupes_same_value_across_providers() {
    let mut merged = normalize_provider_records(
        "first-provider",
        "run-1",
        1,
        vec![AssetIntelProviderRecord {
            kind: OrganizationCandidateKind::Target,
            label: "api.example.com".into(),
            value: "api.example.com".into(),
            confidence: 0.8,
            evidence: serde_json::json!({"provider": "enscan"}),
        }],
    );
    let zone = normalize_provider_records(
        "second-provider",
        "run-1",
        1,
        vec![AssetIntelProviderRecord {
            kind: OrganizationCandidateKind::Target,
            label: "duplicate".into(),
            value: "API.EXAMPLE.COM".into(),
            confidence: 0.7,
            evidence: serde_json::json!({"provider": "zone"}),
        }],
    );

    merge_candidates(&mut merged, zone);

    assert_eq!(merged.targets.len(), 1);
    assert_eq!(merged.targets[0].source, "first-provider");
    assert_eq!(
        merged.targets[0]
            .evidence
            .get("sources")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
}
