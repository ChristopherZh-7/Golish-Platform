use super::*;

#[tokio::test]
async fn test_plan_manager_new_is_empty() {
    let manager = PlanManager::new();
    assert!(manager.is_empty().await);
}

#[tokio::test]
async fn test_plan_manager_default_is_empty() {
    let manager = PlanManager::default();
    assert!(manager.is_empty().await);
}

#[tokio::test]
async fn test_plan_manager_update() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("Test plan".to_string()),
        plan: vec![
            PlanStepInput {
                step: "Step 1".to_string(),
                status: StepStatus::Completed,
            },
            PlanStepInput {
                step: "Step 2".to_string(),
                status: StepStatus::InProgress,
            },
            PlanStepInput {
                step: "Step 3".to_string(),
                status: StepStatus::Pending,
            },
        ],
    };

    let plan = manager.update_plan(args).await.unwrap();

    assert_eq!(plan.version, 1);
    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.summary.completed, 1);
    assert_eq!(plan.summary.in_progress, 1);
    assert_eq!(plan.summary.pending, 1);
    assert_eq!(plan.explanation, Some("Test plan".to_string()));
}

#[tokio::test]
async fn test_plan_manager_version_increments() {
    let manager = PlanManager::new();

    for i in 1..=5 {
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![PlanStepInput {
                step: format!("Step version {}", i),
                status: StepStatus::Pending,
            }],
        };

        let plan = manager.update_plan(args).await.unwrap();
        assert_eq!(plan.version, i);
    }
}

#[tokio::test]
async fn test_plan_manager_snapshot() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("Snapshot test".to_string()),
        plan: vec![PlanStepInput {
            step: "Test step".to_string(),
            status: StepStatus::Pending,
        }],
    };

    manager.update_plan(args).await.unwrap();

    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot.explanation, Some("Snapshot test".to_string()));
    assert_eq!(snapshot.steps.len(), 1);
    assert_eq!(snapshot.version, 1);
}

#[tokio::test]
async fn test_plan_manager_clear() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("Will be cleared".to_string()),
        plan: vec![PlanStepInput {
            step: "Step".to_string(),
            status: StepStatus::InProgress,
        }],
    };

    manager.update_plan(args).await.unwrap();
    assert!(!manager.is_empty().await);

    manager.clear().await;
    assert!(manager.is_empty().await);

    let snapshot = manager.snapshot().await;
    assert!(snapshot.explanation.is_none());
    assert!(snapshot.steps.is_empty());
    // Version is reset on clear
    assert_eq!(snapshot.version, 0);
}

#[tokio::test]
async fn test_plan_manager_trims_whitespace() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("  Trimmed explanation  ".to_string()),
        plan: vec![PlanStepInput {
            step: "  Trimmed step  ".to_string(),
            status: StepStatus::Pending,
        }],
    };

    let plan = manager.update_plan(args).await.unwrap();
    assert_eq!(plan.explanation, Some("Trimmed explanation".to_string()));
    assert_eq!(plan.steps[0].step, "Trimmed step");
}

#[tokio::test]
async fn test_plan_manager_rejects_empty_steps() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![PlanStepInput {
            step: "  ".to_string(), // Empty after trim
            status: StepStatus::Pending,
        }],
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::EmptyStepDescription(1))));
}

#[tokio::test]
async fn test_plan_manager_rejects_multiple_in_progress() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "Step 1".to_string(),
                status: StepStatus::InProgress,
            },
            PlanStepInput {
                step: "Step 2".to_string(),
                status: StepStatus::InProgress,
            },
        ],
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::MultipleInProgress(2))));
}

#[tokio::test]
async fn test_plan_manager_allows_zero_in_progress() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "Step 1".to_string(),
                status: StepStatus::Completed,
            },
            PlanStepInput {
                step: "Step 2".to_string(),
                status: StepStatus::Pending,
            },
        ],
    };

    let result = manager.update_plan(args).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_plan_manager_allows_one_in_progress() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "Step 1".to_string(),
                status: StepStatus::InProgress,
            },
            PlanStepInput {
                step: "Step 2".to_string(),
                status: StepStatus::Pending,
            },
        ],
    };

    let result = manager.update_plan(args).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().summary.in_progress, 1);
}

#[tokio::test]
async fn test_plan_manager_rejects_too_many_steps() {
    let manager = PlanManager::new();

    let steps: Vec<PlanStepInput> = (0..15)
        .map(|i| PlanStepInput {
            step: format!("Step {}", i),
            status: StepStatus::Pending,
        })
        .collect();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: steps,
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::InvalidStepCount(15))));
}

#[tokio::test]
async fn test_plan_manager_rejects_zero_steps() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("Empty plan".to_string()),
        plan: vec![],
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::InvalidStepCount(0))));
}

#[tokio::test]
async fn test_plan_manager_accepts_boundary_step_counts() {
    let manager = PlanManager::new();

    // Test minimum (1 step)
    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![PlanStepInput {
            step: "Single step".to_string(),
            status: StepStatus::Pending,
        }],
    };
    assert!(manager.update_plan(args).await.is_ok());

    // Test maximum (12 steps)
    let steps: Vec<PlanStepInput> = (0..12)
        .map(|i| PlanStepInput {
            step: format!("Step {}", i + 1),
            status: StepStatus::Pending,
        })
        .collect();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: steps,
    };
    let result = manager.update_plan(args).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().steps.len(), 12);
}

#[tokio::test]
async fn test_plan_manager_rejects_just_over_max() {
    let manager = PlanManager::new();

    // Test 13 steps (just over max)
    let steps: Vec<PlanStepInput> = (0..13)
        .map(|i| PlanStepInput {
            step: format!("Step {}", i + 1),
            status: StepStatus::Pending,
        })
        .collect();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: steps,
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::InvalidStepCount(13))));
}

#[tokio::test]
async fn test_plan_manager_empty_description_at_various_positions() {
    let manager = PlanManager::new();

    // Empty at position 1
    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "".to_string(),
                status: StepStatus::Pending,
            },
            PlanStepInput {
                step: "Valid".to_string(),
                status: StepStatus::Pending,
            },
        ],
    };
    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::EmptyStepDescription(1))));

    // Empty at position 2
    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "Valid".to_string(),
                status: StepStatus::Pending,
            },
            PlanStepInput {
                step: "\t\n".to_string(), // Whitespace only
                status: StepStatus::Pending,
            },
        ],
    };
    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::EmptyStepDescription(2))));
}

// ========================================================================
// load_from_db emit tests (P0-1 plan restore on restart)
// ========================================================================
//
// Covers:
//   docs/design/2026-05-17-plan-restore-on-restart.md · Task 3
//
// We need a tiny in-memory implementation of `DbRepoProvider` so we can
// inject a single fake execution plan and observe that `load_from_db`:
//   * fires `PlanEventEmitter::emit_plan_updated` once with the restored
//     plan snapshot
//   * does not fire when there is no DB row to restore
//
// All non-plan trait methods `unimplemented!()` because `load_from_db`
// only touches `plan_list_active`.

mod load_from_db_tests {
    use super::*;
    use crate::db_traits::{
        AgentType, DbRepoProvider, ExecutionPlanView, MessageChainView, NewExecutionPlan, NewTask,
        NewWikiChangelog, NewWikiPage, PlanStatus, SubtaskStatus, SubtaskView, TaskStatus,
        TaskView,
    };
    use crate::planner::{PlanEventEmitter, SharedPlanEventEmitter};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    /// Captures every `emit_plan_updated` call for assertion.
    #[derive(Default)]
    struct CapturingEmitter {
        events: Mutex<Vec<(u32, PlanSummary, Vec<PlanStep>, Option<String>)>>,
    }

    impl PlanEventEmitter for CapturingEmitter {
        fn emit_plan_updated(
            &self,
            version: u32,
            summary: PlanSummary,
            steps: Vec<PlanStep>,
            explanation: Option<String>,
        ) {
            self.events
                .lock()
                .unwrap()
                .push((version, summary, steps, explanation));
        }
    }

    impl CapturingEmitter {
        fn shared() -> (Arc<Self>, SharedPlanEventEmitter) {
            let arc = Arc::new(Self::default());
            (arc.clone(), arc as SharedPlanEventEmitter)
        }

        fn len(&self) -> usize {
            self.events.lock().unwrap().len()
        }
    }

    /// Tiny stub repo: returns a configurable plan list for `plan_list_active`
    /// and panics on every other method (those paths are not exercised by
    /// `PlanManager::load_from_db`).
    struct StubRepo {
        plans: Vec<ExecutionPlanView>,
    }

    impl StubRepo {
        fn with_plan(plan: ExecutionPlanView) -> Self {
            Self { plans: vec![plan] }
        }

        fn empty() -> Self {
            Self { plans: vec![] }
        }
    }

    #[async_trait]
    impl DbRepoProvider for StubRepo {
        async fn plan_list_active(
            &self,
            _project_path: &str,
        ) -> anyhow::Result<Vec<ExecutionPlanView>> {
            Ok(self.plans.clone())
        }

        async fn plan_update_steps(
            &self,
            _id: Uuid,
            _steps: &serde_json::Value,
            _current_step: i32,
            _status: PlanStatus,
        ) -> anyhow::Result<()> {
            unimplemented!("plan_update_steps not used by load_from_db tests")
        }

        async fn plan_create(&self, _plan: NewExecutionPlan) -> anyhow::Result<ExecutionPlanView> {
            unimplemented!("plan_create not used by load_from_db tests")
        }

        // ── Sub-agent dispatch stubs (P0-4) ────────────────────────────
        async fn dispatch_record_start(
            &self,
            _session_id: Uuid,
            _parent_dispatch_id: Option<Uuid>,
            _agent_id: &str,
            _tool_call_id: Option<&str>,
            _depth: i32,
            _args: &serde_json::Value,
        ) -> anyhow::Result<Uuid> {
            unimplemented!()
        }
        async fn dispatch_record_finish(
            &self,
            _id: Uuid,
            _status: crate::db_traits::DispatchStatus,
            _result: Option<&serde_json::Value>,
            _error_message: Option<&str>,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn dispatch_list_running(
            &self,
            _session_id: Uuid,
        ) -> anyhow::Result<Vec<crate::db_traits::SubAgentDispatchView>> {
            unimplemented!()
        }

        // ── Wiki KB stubs ───────────────────────────────────────────────
        async fn wiki_upsert_page(&self, _page: &NewWikiPage) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn wiki_link_cve(&self, _cve: &str, _path: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn wiki_delete_refs_from(&self, _path: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn wiki_upsert_page_ref(
            &self,
            _from_path: &str,
            _to_path: &str,
            _context: &str,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn wiki_add_changelog(&self, _entry: &NewWikiChangelog) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn wiki_search_fts(
            &self,
            _query: &str,
            _limit: i64,
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        async fn wiki_search_by_category(
            &self,
            _category: &str,
            _limit: i64,
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        async fn wiki_search_by_tag(
            &self,
            _tag: &str,
            _limit: i64,
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        async fn wiki_list_cves_with_pocs(&self) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        async fn wiki_list_unresearched_cves(
            &self,
            _limit: i64,
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        async fn wiki_poc_stats(&self) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        #[allow(clippy::too_many_arguments)]
        async fn wiki_upsert_poc_full(
            &self,
            _cve_id: &str,
            _name: &str,
            _poc_type: &str,
            _language: &str,
            _content: &str,
            _source: &str,
            _source_url: &str,
            _severity: &str,
            _description: &str,
            _tags: &[String],
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }

        // ── Vuln intel ─────────────────────────────────────────────────
        async fn vuln_intel_search(
            &self,
            _cve_id: &str,
            _limit: i64,
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }

        // ── Security analysis stubs ────────────────────────────────────
        #[allow(clippy::too_many_arguments)]
        async fn audit_log_operation(
            &self,
            _summary: &str,
            _op_type: &str,
            _description: &str,
            _project_path: Option<&str>,
            _source: &str,
            _target_id: Option<Uuid>,
            _session_id: Option<&str>,
            _tool_name: Option<&str>,
            _status: &str,
            _detail: &serde_json::Value,
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        #[allow(clippy::too_many_arguments)]
        async fn api_endpoints_insert(
            &self,
            _target_id: Uuid,
            _project_path: Option<&str>,
            _url: &str,
            _method: &str,
            _path: &str,
            _params: &serde_json::Value,
            _raw_data: &serde_json::Value,
            _auth_type: Option<&str>,
            _source: &str,
            _risk_level: &str,
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        async fn js_analysis_insert(
            &self,
            _target_id: Uuid,
            _project_path: &str,
            _url: &str,
            _filename: &str,
            _analysis: &serde_json::Value,
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        async fn js_analysis_update_file_path(
            &self,
            _id: Uuid,
            _file_path: &str,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn fingerprints_upsert(
            &self,
            _target_id: Uuid,
            _project_path: &str,
            _category: &str,
            _name: &str,
            _version: Option<&str>,
            _confidence: f64,
            _raw_data: Option<&serde_json::Value>,
        ) -> anyhow::Result<bool> {
            unimplemented!()
        }
        async fn passive_scans_insert(
            &self,
            _target_id: Uuid,
            _project_path: &str,
            _scan_type: &str,
            _tool_name: &str,
            _findings: &serde_json::Value,
            _raw_output: Option<&str>,
            _severity: &str,
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        async fn query_target_data(
            &self,
            _target_id: Uuid,
            _sections: &[String],
        ) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }

        // ── Tasks & subtasks ───────────────────────────────────────────
        async fn task_create(&self, _task: NewTask) -> anyhow::Result<TaskView> {
            unimplemented!()
        }
        async fn task_get(&self, _id: Uuid) -> anyhow::Result<Option<TaskView>> {
            unimplemented!()
        }
        async fn task_update_status(&self, _id: Uuid, _status: TaskStatus) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn task_set_result(&self, _id: Uuid, _result: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn subtask_create(
            &self,
            _task_id: Uuid,
            _session_id: Uuid,
            _title: &str,
            _description: &str,
            _agent: Option<AgentType>,
        ) -> anyhow::Result<SubtaskView> {
            unimplemented!()
        }
        async fn subtask_update_status(
            &self,
            _id: Uuid,
            _status: SubtaskStatus,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn subtask_set_result(&self, _id: Uuid, _result: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn subtask_next_pending(
            &self,
            _task_id: Uuid,
        ) -> anyhow::Result<Option<SubtaskView>> {
            unimplemented!()
        }
        async fn subtask_list_by_task(&self, _task_id: Uuid) -> anyhow::Result<Vec<SubtaskView>> {
            unimplemented!()
        }
        async fn subtask_delete_pending(&self, _task_id: Uuid) -> anyhow::Result<()> {
            unimplemented!()
        }

        // ── Message chains ─────────────────────────────────────────────
        async fn message_chain_create(
            &self,
            _session_id: Uuid,
            _task_id: Option<Uuid>,
            _subtask_id: Option<Uuid>,
            _agent_type: AgentType,
            _parent_chain_id: Option<Uuid>,
            _model: Option<&str>,
        ) -> anyhow::Result<MessageChainView> {
            unimplemented!()
        }
        async fn message_chain_update_chain(
            &self,
            _id: Uuid,
            _chain_json: &serde_json::Value,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }
        #[allow(clippy::too_many_arguments)]
        async fn message_chain_update_usage(
            &self,
            _id: Uuid,
            _input_tokens: i32,
            _output_tokens: i32,
            _cache_read_tokens: i32,
            _input_cost: f64,
            _output_cost: f64,
            _duration_ms: i32,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }
    }

    fn make_demo_plan() -> ExecutionPlanView {
        ExecutionPlanView {
            id: Uuid::new_v4(),
            title: "Restore me".to_string(),
            description: "Demo plan for restart restore test".to_string(),
            steps: serde_json::json!([
                {"id":"s1","title":"step one","description":"","status":"completed"},
                {"id":"s2","title":"step two","description":"","status":"in_progress"},
                {"id":"s3","title":"step three","description":"","status":"pending"}
            ]),
            status: PlanStatus::InProgress,
            current_step: 1,
        }
    }

    #[tokio::test]
    async fn load_from_db_emits_plan_updated_with_restored_snapshot() {
        let mut manager =
            PlanManager::new().with_db_repo(Some(Uuid::new_v4()), Some("/tmp/proj".to_string()));
        manager.set_repo(Arc::new(StubRepo::with_plan(make_demo_plan())));

        let (emitter, shared) = CapturingEmitter::shared();
        manager.set_event_emitter(shared);

        let loaded = manager.load_from_db().await;
        assert!(
            loaded,
            "load_from_db should report success when DB has a plan"
        );
        assert_eq!(
            emitter.len(),
            1,
            "emit_plan_updated should fire exactly once"
        );

        let events = emitter.events.lock().unwrap();
        let (version, summary, steps, explanation) = events.last().unwrap();
        assert_eq!(*version, 1, "restored plan version starts at 1");
        assert_eq!(steps.len(), 3);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.in_progress, 1);
        assert_eq!(summary.pending, 1);
        assert_eq!(
            explanation.as_deref(),
            Some("Demo plan for restart restore test"),
            "description should round-trip into explanation",
        );
    }

    #[tokio::test]
    async fn load_from_db_does_not_emit_when_empty() {
        let mut manager =
            PlanManager::new().with_db_repo(Some(Uuid::new_v4()), Some("/tmp/proj".to_string()));
        manager.set_repo(Arc::new(StubRepo::empty()));

        let (emitter, shared) = CapturingEmitter::shared();
        manager.set_event_emitter(shared);

        let loaded = manager.load_from_db().await;
        assert!(!loaded, "load_from_db returns false when DB has no plan");
        assert_eq!(emitter.len(), 0, "no plan ⇒ no emission");
    }

    #[tokio::test]
    async fn load_from_db_does_not_emit_when_no_project_path() {
        let mut manager = PlanManager::new().with_db_repo(Some(Uuid::new_v4()), None);
        manager.set_repo(Arc::new(StubRepo::with_plan(make_demo_plan())));

        let (emitter, shared) = CapturingEmitter::shared();
        manager.set_event_emitter(shared);

        let loaded = manager.load_from_db().await;
        assert!(!loaded, "no project_path ⇒ load_from_db is a no-op");
        assert_eq!(emitter.len(), 0);
    }
}
