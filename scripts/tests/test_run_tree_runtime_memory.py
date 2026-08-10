from __future__ import annotations

import json
import re
import tempfile
import unittest
from pathlib import Path

from scripts import run_tree


class FixtureQuery:
    """Offline DB fixture keyed by the diagnostic query's stable SQL marker."""

    def __init__(self, rows: dict[str, list[tuple]]) -> None:
        self.rows = rows
        self.calls: list[tuple[str, tuple]] = []
        self.sql_by_id: dict[str, str] = {}

    def __call__(self, sql: str, params: tuple = ()) -> list[tuple]:
        match = re.search(r"/\* run_tree:([a-z_]+) \*/", sql)
        query_id = match.group(1) if match else "unmarked"
        self.calls.append((query_id, params))
        self.sql_by_id[query_id] = sql
        return self.rows.get(query_id, [])


def row(**values: object) -> tuple[dict[str, object]]:
    return (values,)


class RuntimeMemoryDiagnosisTests(unittest.TestCase):
    def render_with_query(
        self, rows: dict[str, list[tuple]]
    ) -> tuple[str, FixtureQuery]:
        self.assertTrue(
            hasattr(run_tree, "_runtime_memory_lines"),
            "run_tree must expose the offline-testable runtime-memory renderer",
        )
        query = FixtureQuery(rows)
        lines = run_tree._runtime_memory_lines(
            query,
            session_id="session-fixture",
            candidate_operation_ids=["00000000-0000-0000-0000-000000000001"],
            trunc=240,
        )
        self.assertTrue(query.calls, "renderer must execute diagnostic queries")
        return "\n".join(lines), query

    def render(self, rows: dict[str, list[tuple]]) -> str:
        rendered, _query = self.render_with_query(rows)
        return rendered

    def test_session_evidence_facts_never_falls_back_to_global_rows(self) -> None:
        calls: list[tuple[str, tuple]] = []

        def query(sql: str, params: tuple = ()) -> list[tuple]:
            calls.append((sql, params))
            return []

        rows = run_tree.session_evidence_facts(query, "session-exact")

        self.assertEqual(rows, [])
        self.assertEqual(len(calls), 1)
        self.assertIn("session_id=%s", calls[0][0])
        self.assertEqual(calls[0][1], ("session-exact",))

    def test_session_operation_ids_accepts_runtime_trace_detail_but_not_model_args(self) -> None:
        runtime_operation_id = "00000000-0000-0000-0000-0000000000a1"
        model_authored_id = "00000000-0000-0000-0000-0000000000ff"
        with tempfile.TemporaryDirectory() as temp_dir:
            session_dir = Path(temp_dir)
            events = [
                {
                    "type": "harness_trace",
                    "detail": {"operation_id": runtime_operation_id},
                },
                {
                    "type": "tool_request",
                    "args": {"operation_id": model_authored_id},
                },
            ]
            (session_dir / "transcript.json").write_text(
                "".join(json.dumps(event) + "\n" for event in events)
            )

            operation_ids = run_tree.session_operation_ids(session_dir)

        self.assertEqual(operation_ids, [runtime_operation_id])

    def test_legacy_fallback_requires_a_real_checkpoint_shape(self) -> None:
        self.assertTrue(
            hasattr(run_tree, "_has_legacy_checkpoint"),
            "legacy fallback diagnosis needs a shape-aware checkpoint predicate",
        )
        self.assertFalse(run_tree._has_legacy_checkpoint({"unrelated_namespace": {"x": 1}}))
        self.assertTrue(
            run_tree._has_legacy_checkpoint(
                {
                    "graph_flow": {
                        "state": {"seeded": {}, "visited": [], "applied": {}},
                        "next_node": "external_attack_surface",
                    }
                }
            )
        )
        self.assertTrue(
            run_tree._has_legacy_checkpoint(
                {
                    "profile": "red_team",
                    "current_stage": "external_attack_surface",
                    "current_stage_run_id": "00000000-0000-0000-0000-0000000000a2",
                    "queue_titles": [],
                    "completed_count": 0,
                }
            )
        )

    def test_runtime_operation_fallback_resolves_the_transcript_chat_key(self) -> None:
        _rendered, query = self.render_with_query({})

        sql = query.sql_by_id["runtime_operations"]
        self.assertIn("JOIN sessions AS session ON session.id = task.session_id", sql)
        self.assertIn("session.chat_session_key = %s", sql)
        self.assertNotIn("task.session_id::text = %s", sql)
        self.assertEqual(
            next(params for query_id, params in query.calls if query_id == "runtime_operations"),
            (["00000000-0000-0000-0000-000000000001"], "session-fixture", "session-fixture"),
        )

    def test_v2_runtime_memory_fixture_renders_every_exact_identity_and_recovery_field(self) -> None:
        rendered = self.render(
            {
                "runtime_rollout": [
                    row(
                        contract="v2_only",
                        contract_rank=3,
                        row_version=7,
                        updated_at="2026-07-13T01:00:00Z",
                    )
                ],
                "runtime_operations": [
                    row(
                        operation_id="op-1",
                        runtime_memory_contract="v2_only",
                        profile="red_team",
                        current_stage="external_attack_surface",
                        project_scope_id="project-1",
                        engagement_org_id="org-root",
                        superseded_by=None,
                        stage_started_at="2026-07-13T01:01:00Z",
                        state_blob={
                            "graph_flow": {
                                "state": {"seeded": {}, "visited": [], "applied": {}},
                                "next_node": "external_attack_surface",
                            }
                        },
                    )
                ],
                "stage_executions": [
                    row(
                        id="stage-exec-1",
                        stage_kind="external_attack_surface",
                        status="started",
                        started_at="2026-07-13T01:01:00Z",
                        completed_at=None,
                    )
                ],
                "scope_decisions": [
                    row(
                        id="decision-1",
                        stage_execution_id="scoping-exec-1",
                        root_organization_id="org-root",
                        mode="included",
                        decision_hash="decision-sha",
                        choice_tool_call_id="tool-choice",
                        proposal_tool_call_id="tool-proposal",
                        review_tool_call_id="tool-review",
                    )
                ],
                "scope_snapshots": [
                    row(
                        id="snapshot-1",
                        scope_decision_id="decision-1",
                        project_scope_id="project-1",
                        project_path_at_freeze="/fixture/project",
                        root_organization_id="org-root",
                        mode="included",
                        scope_hash="scope-sha",
                        schema_version=1,
                        frozen_at="2026-07-13T01:00:30Z",
                        sealed_at="2026-07-13T01:00:31Z",
                    )
                ],
                "scope_units": [
                    row(
                        snapshot_id="snapshot-1",
                        organization_id="org-root",
                        parent_organization_id=None,
                        organization_name_at_freeze="Root Corp",
                        role="root",
                        depth=0,
                        ordinal=0,
                        ownership_percent=None,
                        decision_row_id="row-root",
                        approval_source={"tool_call_id": "tool-review"},
                    ),
                    row(
                        snapshot_id="snapshot-1",
                        organization_id="org-child",
                        parent_organization_id="org-root",
                        organization_name_at_freeze="Child Corp",
                        role="subsidiary",
                        depth=1,
                        ordinal=1,
                        ownership_percent="75.00",
                        decision_row_id="row-child",
                        approval_source={"review_row_id": "review-child"},
                    ),
                ],
                "stage_units": [
                    row(
                        id="stage-unit-1",
                        stage_execution_id="stage-exec-1",
                        scope_snapshot_id="snapshot-1",
                        organization_id="org-child",
                        stage_kind="external_attack_surface",
                        generation=2,
                        specialist="prober",
                        status="running",
                        gate_attempt=3,
                        row_version=5,
                        started_at="2026-07-13T01:02:00Z",
                        terminal_at=None,
                        scope_member=True,
                    )
                ],
                "stage_workers": [
                    row(
                        id="worker-1",
                        stage_execution_id="stage-exec-1",
                        stage_run_unit_id="stage-unit-1",
                        organization_id="org-child",
                        worker_generation=4,
                        specialist="prober",
                        work_item_kind="organization",
                        work_item_key="org-child",
                        agent_path="stage/external_attack_surface/org-child",
                        parent_request_id="request-parent",
                        message_chain_id="chain-1",
                        status="recovery_required",
                        gate_attempt=3,
                        checkpoint={"turn": 8, "pending": ["PORT"]},
                        checkpoint_version=11,
                        lease_token="lease-1",
                        lease_owner="runtime-a",
                        lease_acquired_at="2026-07-13T01:02:00Z",
                        lease_expires_at="2026-07-13T01:02:30Z",
                        heartbeat_at="2026-07-13T01:02:20Z",
                        attempt_epoch=6,
                        active_tool_call_id="tool-active-1",
                        active_tool_started_at="2026-07-13T01:02:25Z",
                        active_tool_name="eas_discover_ports",
                        active_tool_status="running",
                        active_tool_request_id="request-tool-1",
                        lease_expired=True,
                        unit_identity_matches=True,
                        scope_member=True,
                    )
                ],
                "stage_submissions": [
                    row(
                        id="submission-1",
                        stage_execution_id="stage-exec-1",
                        stage_run_unit_id="stage-unit-1",
                        worker_run_id="worker-1",
                        organization_id="org-child",
                        tool_call_record_id="tool-submit-1",
                        tool_request_id="request-submit-1",
                        stage_kind="external_attack_surface",
                        attempt_epoch=6,
                        lease_token="lease-1",
                        payload_sha256="payload-sha",
                        submitted_at="2026-07-13T01:03:00Z",
                        scope_member=True,
                    )
                ],
                "stage_handoffs": [
                    row(
                        id="handoff-1",
                        organization_id="org-child",
                        scope_snapshot_id="snapshot-1",
                        from_stage_kind="external_attack_surface",
                        stage_execution_id="stage-exec-old",
                        source_stage_run_unit_id="stage-unit-old",
                        deliverable_submission_id="submission-old",
                        scope_hash="scope-sha",
                        payload_sha256="handoff-sha",
                        unit_gate_decision_hash="gate-sha",
                        aggregate_pass_token_hash="pass-sha",
                        gate_passed_at="2026-07-13T01:00:50Z",
                        invalidated_at=None,
                        evidence_ids=[101, 102],
                        scope_member=True,
                    )
                ],
            }
        )

        expected_fragments = [
            "rollout: contract=v2_only rank=3 row_version=7",
            "operation op-1: contract=v2_only profile=red_team current_stage=external_attack_surface",
            "project_scope=project-1 engagement_org=org-root",
            "stage_executions: exact_active=1",
            "id=stage-exec-1 stage=external_attack_surface status=started",
            "scope_decision id=decision-1 stage_execution=scoping-exec-1 root_org=org-root mode=included hash=decision-sha",
            "choice=tool-choice proposal=tool-proposal review=tool-review",
            "scope_snapshot id=snapshot-1 decision=decision-1 project_scope=project-1 root_org=org-root",
            "mode=included hash=scope-sha schema=1 sealed_at=2026-07-13T01:00:31Z",
            "scope_unit org=org-child parent=org-root role=subsidiary depth=1 ordinal=1 ownership=75.00",
            "stage_unit id=stage-unit-1 execution=stage-exec-1 snapshot=snapshot-1 org=org-child",
            "stage=external_attack_surface generation=2 specialist=prober status=running gate_attempt=3 row_version=5",
            "worker id=worker-1 unit=stage-unit-1 execution=stage-exec-1 org=org-child generation=4",
            "specialist=prober work_item=organization:org-child status=recovery_required gate_attempt=3",
            "lease present=yes owner=runtime-a epoch=6 expires=2026-07-13T01:02:30Z expired=yes",
            "active_tool id=tool-active-1 request=request-tool-1 name=eas_discover_ports status=running",
            "chain=chain-1 checkpoint_version=11 checkpoint_present=yes",
            "recovery=manual_required",
            "submission id=submission-1 execution=stage-exec-1 unit=stage-unit-1 worker=worker-1 org=org-child",
            "tool=tool-submit-1/request-submit-1 stage=external_attack_surface epoch=6 payload_sha256=payload-sha",
            "handoff id=handoff-1 org=org-child from_stage=external_attack_surface execution=stage-exec-old",
            "scope_hash=scope-sha payload_sha256=handoff-sha evidence_ids=[101, 102] invalidated_at=None",
            "selected_read_source=v2 legacy_fallback=forbidden",
        ]
        for fragment in expected_fragments:
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, rendered)
        self.assertNotIn("lease-1", rendered)
        self.assertNotIn('"pending": ["PORT"]', rendered)

    def test_finished_v2_operation_selects_completed_current_stage_without_false_anomaly(
        self,
    ) -> None:
        rendered = self.render(
            {
                "runtime_rollout": [
                    row(
                        contract="v2_only",
                        contract_rank=3,
                        row_version=8,
                        updated_at="2026-07-15T01:00:00Z",
                    )
                ],
                "runtime_operations": [
                    row(
                        operation_id="op-finished",
                        runtime_memory_contract="v2_only",
                        task_status="finished",
                        profile="pentest",
                        current_stage="target_intel",
                        project_scope_id="project-finished",
                        engagement_org_id="org-root",
                        superseded_by=None,
                        stage_started_at="2026-07-15T01:01:00Z",
                        state_blob={},
                    )
                ],
                "stage_executions": [
                    row(
                        id="stage-terminal",
                        stage_kind="target_intel",
                        status="completed",
                        started_at="2026-07-15T01:01:00Z",
                        completed_at="2026-07-15T01:02:00Z",
                    )
                ],
                "scope_snapshots": [
                    row(
                        id="snapshot-terminal",
                        scope_decision_id="decision-terminal",
                        project_scope_id="project-finished",
                        project_path_at_freeze="/fixture/finished",
                        root_organization_id="org-root",
                        mode="root_only",
                        scope_hash="scope-terminal",
                        schema_version=1,
                        frozen_at="2026-07-15T01:00:00Z",
                        sealed_at="2026-07-15T01:00:01Z",
                    )
                ],
                "stage_units": [
                    row(
                        id="unit-terminal",
                        stage_execution_id="stage-terminal",
                        scope_snapshot_id="snapshot-terminal",
                        organization_id="org-root",
                        stage_kind="target_intel",
                        generation=1,
                        specialist="recon",
                        status="passed",
                        gate_attempt=0,
                        row_version=2,
                        started_at="2026-07-15T01:01:01Z",
                        terminal_at="2026-07-15T01:01:59Z",
                        scope_member=True,
                    )
                ],
            }
        )

        self.assertIn(
            "stage_executions: exact_active=0 terminal_selected=stage-terminal",
            rendered,
        )
        self.assertNotIn("anomaly: missing active stage execution", rendered)
        self.assertIn("selected_read_source=v2 legacy_fallback=forbidden", rendered)
        self.assertNotIn("v2_only operation has incomplete runtime state", rendered)

    def test_finished_summary_only_reporting_is_complete_without_a_stage_unit(self) -> None:
        rendered = self.render(
            {
                "runtime_rollout": [
                    row(
                        contract="v2_only",
                        contract_rank=3,
                        row_version=9,
                        updated_at="2026-08-10T02:00:00Z",
                    )
                ],
                "runtime_operations": [
                    row(
                        operation_id="op-reporting",
                        runtime_memory_contract="v2_only",
                        task_status="finished",
                        profile="red_team",
                        current_stage="reporting",
                        project_scope_id="project-reporting",
                        engagement_org_id="org-root",
                        superseded_by=None,
                        stage_started_at="2026-08-10T02:00:00Z",
                        state_blob={},
                    )
                ],
                "stage_executions": [
                    row(
                        id="stage-reporting",
                        stage_kind="reporting",
                        status="completed",
                        started_at="2026-08-10T02:00:00Z",
                        completed_at="2026-08-10T02:00:01Z",
                    )
                ],
                "scope_snapshots": [
                    row(
                        id="snapshot-reporting",
                        scope_decision_id="decision-reporting",
                        project_scope_id="project-reporting",
                        project_path_at_freeze="/fixture/reporting",
                        root_organization_id="org-root",
                        mode="root_only",
                        scope_hash="scope-reporting",
                        schema_version=1,
                        frozen_at="2026-08-10T01:00:00Z",
                        sealed_at="2026-08-10T01:00:01Z",
                    )
                ],
                "runtime_reporting_completion": [
                    row(
                        report_id="report-1",
                        revision_id="revision-1",
                        validation_status="validated",
                        publication_status="unpublished",
                        source_manifest_count=203,
                    )
                ],
            }
        )

        self.assertIn(
            "stage_executions: exact_active=0 terminal_selected=stage-reporting",
            rendered,
        )
        self.assertIn("selected_read_source=v2 legacy_fallback=forbidden", rendered)
        self.assertNotIn("v2_only operation has incomplete runtime state", rendered)

    def test_stage_team_tree_renders_plan_items_workers_outputs_requests_and_barrier(self) -> None:
        rendered, query = self.render_with_query(
            {
                "runtime_operations": [
                    row(
                        operation_id="op-team",
                        runtime_memory_contract="v2_only",
                        attack_execution_contract="v2_only",
                        profile="assessment",
                        current_stage="target_intel",
                        project_scope_id="project-1",
                        engagement_org_id="org-root",
                        superseded_by=None,
                        stage_started_at="2026-07-14T00:00:00Z",
                        state_blob={},
                    )
                ],
                "stage_team_plans": [
                    row(
                        id="plan-1",
                        stage_execution_id="execution-1",
                        stage_run_unit_id="unit-1",
                        organization_id="org-root",
                        stage_kind="target_intel",
                        schema_version=1,
                        plan_version=1,
                        plan_hash="sha256:plan",
                        leader_role="intel_provider",
                        aggregator_kind="worker",
                        aggregator_role="intel_aggregator",
                        allowed_worker_roles=["intel_provider", "intel_aggregator"],
                        max_workers_total=4,
                        max_workers_active=2,
                        dynamic_requests_allowed=True,
                        dispatch_epoch=0,
                        requests_closed_at="2026-07-14T00:00:05Z",
                        final_submitter_kind="worker",
                        final_submitter_worker_run_id=None,
                    )
                ],
                "stage_team_work_items": [
                    row(
                        id="item-1",
                        team_plan_id="plan-1",
                        kind="provider",
                        stable_key="provider:fofa",
                        role="intel_provider",
                        input_manifest_hash="sha256:input",
                        subject_ref_count=1,
                        required_for_barrier=True,
                        conflict_key=None,
                        priority=0,
                        status="completed",
                        output_schema="stage_worker_output.v1",
                        created_by="server_seed",
                    ),
                    row(
                        id="item-aggregate",
                        team_plan_id="plan-1",
                        kind="aggregate",
                        stable_key="aggregate:unit",
                        role="intel_aggregator",
                        input_manifest_hash="sha256:aggregate",
                        subject_ref_count=1,
                        required_for_barrier=False,
                        conflict_key=None,
                        priority=100,
                        status="queued",
                        output_schema="stage_deliverable.v1",
                        created_by="server_seed",
                    ),
                ],
                "stage_team_dependencies": [
                    row(
                        team_plan_id="plan-1",
                        work_item_id="item-aggregate",
                        depends_on_work_item_id="item-1",
                    )
                ],
                "stage_team_outputs": [
                    row(
                        id="output-1",
                        team_plan_id="plan-1",
                        work_item_id="item-1",
                        worker_run_id="worker-1",
                        business_disposition="found",
                        canonical_fact_ref_count=2,
                        evidence_ids=[31, 32],
                        checked_empty_cell_count=0,
                        blocker_codes=[],
                        output_hash="sha256:output",
                    )
                ],
                "stage_team_requests": [
                    row(
                        id="request-1",
                        team_plan_id="plan-1",
                        parent_work_item_id="item-1",
                        parent_worker_run_id="worker-1",
                        requested_role="intel_provider",
                        request_kind="provider_followup",
                        subject_ref_count=1,
                        reason_code="coverage_gap",
                        request_payload_hash="sha256:request",
                        status="accepted",
                        decision_reason_code=None,
                        accepted_work_item_id="item-2",
                    )
                ],
                "stage_workers": [
                    row(
                        id="worker-1",
                        work_item_id="item-1",
                        stage_execution_id="execution-1",
                        stage_run_unit_id="unit-1",
                        organization_id="org-root",
                        worker_generation=1,
                        specialist="intel_provider",
                        work_item_kind="provider",
                        work_item_key="provider:fofa",
                        agent_path="main>stage_run:target_intel>intel_provider",
                        parent_request_id="parent-1",
                        message_chain_id="chain-1",
                        status="passed",
                        gate_attempt=0,
                        checkpoint_version=1,
                        checkpoint_present=True,
                        checkpoint_bytes=100,
                        lease_present=False,
                        lease_owner=None,
                        lease_acquired_at=None,
                        lease_expires_at=None,
                        heartbeat_at=None,
                        attempt_epoch=1,
                        active_tool_call_id=None,
                        active_tool_started_at=None,
                        active_tool_name=None,
                        active_tool_status=None,
                        active_tool_request_id=None,
                        lease_expired=False,
                        evidence_watermark=32,
                        unit_identity_matches=True,
                        scope_member=True,
                    )
                ],
            }
        )

        for fragment in [
            "stage_teams:",
            "unit=unit-1 org=org-root plan=plan-1 stage=target_intel v=1",
            "barrier ready=yes terminal=1/1 live=0 retry=0 recovery=0 missing_outputs=0",
            "request id=request-1 parent=item-1/worker-1 role=intel_provider kind=provider_followup",
            "work_item id=item-1 kind=provider key=provider:fofa role=intel_provider status=completed",
            "worker id=worker-1 generation=1 status=passed chain=chain-1 epoch=1",
            "output id=output-1 worker=worker-1 disposition=found facts=2 evidence=[31, 32]",
            "dependencies=['item-1']",
        ]:
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, rendered)
        self.assertNotIn("canonical_output", query.sql_by_id["stage_team_outputs"])
        self.assertNotIn("budget_hint", query.sql_by_id["stage_team_requests"])

    def test_attack_pipeline_diagnostics_are_aggregate_safe_and_redacted(self) -> None:
        rendered, query = self.render_with_query(
            {
                "attack_rollout": [
                    row(
                        contract="v2_only",
                        rank=3,
                        row_version=9,
                        updated_at="2026-07-13T03:00:00Z",
                    )
                ],
                "runtime_operations": [
                    row(
                        operation_id="op-attack",
                        runtime_memory_contract="v2_only",
                        attack_execution_contract="v2_only",
                        profile="red_team",
                        current_stage="verification",
                        project_scope_id="project-1",
                        engagement_org_id="org-root",
                        superseded_by=None,
                        stage_started_at="2026-07-13T03:01:00Z",
                        state_blob={},
                    )
                ],
                "stage_executions": [
                    row(
                        id="stage-verification",
                        stage_kind="verification",
                        status="started",
                        started_at="2026-07-13T03:01:00Z",
                        completed_at=None,
                    )
                ],
                "scope_snapshots": [
                    row(
                        id="snapshot-1",
                        scope_decision_id="decision-1",
                        project_scope_id="project-1",
                        project_path_at_freeze="/fixture/project",
                        root_organization_id="org-root",
                        mode="included",
                        scope_hash="scope-sha",
                        schema_version=1,
                        frozen_at="2026-07-13T03:00:00Z",
                        sealed_at="2026-07-13T03:00:01Z",
                    )
                ],
                "attack_waves": [
                    row(
                        id="wave-0",
                        scope_snapshot_id="snapshot-1",
                        generation=0,
                        status="terminal",
                        policy_hash="policy-sha",
                        max_waves=3,
                        max_candidates_total=100,
                        max_chain_depth=3,
                        max_attempts_total=200,
                        row_version=4,
                        created_at="2026-07-13T03:02:00Z",
                        updated_at="2026-07-13T03:05:00Z",
                        terminal_at="2026-07-13T03:05:00Z",
                        policy_snapshot={"raw_exploit_recipe": "must-not-render"},
                    ),
                    row(
                        id="wave-1",
                        scope_snapshot_id="snapshot-1",
                        generation=1,
                        status="verification",
                        policy_hash="policy-sha",
                        max_waves=3,
                        max_candidates_total=100,
                        max_chain_depth=3,
                        max_attempts_total=200,
                        row_version=1,
                        created_at="2026-07-13T03:05:00Z",
                        updated_at="2026-07-13T03:06:00Z",
                        terminal_at=None,
                    ),
                ],
                "attack_wave_unit_counts": [
                    row(
                        wave_run_id="wave-0",
                        total=2,
                        open_count=0,
                        reasoning_count=0,
                        review_count=0,
                        verification_count=0,
                        terminal_count=2,
                        review_closed_count=2,
                        verification_closed_count=2,
                        consolidation_pending_count=0,
                        consolidation_ready_count=0,
                        consolidation_consumed_count=2,
                        consolidation_terminal_count=0,
                    ),
                    row(
                        wave_run_id="wave-1",
                        total=2,
                        open_count=1,
                        reasoning_count=0,
                        review_count=0,
                        verification_count=1,
                        terminal_count=0,
                        review_closed_count=1,
                        verification_closed_count=0,
                        consolidation_pending_count=2,
                        consolidation_ready_count=0,
                        consolidation_consumed_count=0,
                        consolidation_terminal_count=0,
                    ),
                ],
                "candidate_attempt_ownership": [
                    row(
                        id="attempt-1",
                        candidate_id="candidate-1",
                        wave_run_id="wave-1",
                        wave_unit_id="wave-unit-1",
                        organization_id="org-child",
                        ordinal=0,
                        status="running",
                        row_version=5,
                        terminal_at=None,
                        stage_worker_run_id="worker-1",
                        worker_status="running",
                        worker_generation=2,
                        specialist="candidate_verifier",
                        attempt_epoch=6,
                        checkpoint_version=11,
                        checkpoint_present=True,
                        checkpoint_bytes=87,
                        lease_present=True,
                        lease_owner="runtime-a",
                        lease_expires_at="2026-07-13T03:07:00Z",
                        lease_expired=False,
                        active_tool_call_id="tool-1",
                        active_tool_name="verify_execute_candidate_action",
                        active_tool_status="running",
                        ownership_matches=True,
                        checkpoint={"raw_exploit_payload": "must-not-render"},
                        lease_token="full-lease-token-must-not-render",
                        result_json={"raw_exploit_material": "must-not-render"},
                    )
                ],
                "attack_lane": [
                    row(
                        lane_key="global:exploit",
                        stage_worker_run_id="worker-1",
                        lease_present=True,
                        lease_owner="runtime-a",
                        lease_expires_at="2026-07-13T03:07:00Z",
                        lease_expired=False,
                        updated_at="2026-07-13T03:06:30Z",
                        lease_token="full-lane-token-must-not-render",
                    )
                ],
                "attack_fact_deltas": [
                    row(
                        id="delta-1",
                        source_attempt_id="attempt-0",
                        candidate_id="candidate-0",
                        wave_run_id="wave-0",
                        wave_unit_id="wave-unit-0",
                        organization_id="org-child",
                        canonical_ref_kind="api_endpoint",
                        canonical_ref_id="endpoint-1",
                        canonical_ref_version=7,
                        canonical_ref_hash="canonical-sha",
                        delta_kind="new_surface",
                        dedupe_hash="delta-sha",
                        status="consumed",
                        consumed_by_wave_run_id="wave-1",
                        evidence_count=2,
                        created_at="2026-07-13T03:04:00Z",
                        consumed_at="2026-07-13T03:05:00Z",
                        summary="raw exploit details must-not-render",
                        target_value_at_time="https://secret.invalid/payload",
                    )
                ],
                "attack_residual_risks": [
                    row(
                        id="residual-1",
                        wave_run_id="wave-1",
                        wave_unit_id="wave-unit-1",
                        organization_id="org-child",
                        reason_code="max_waves_exhausted",
                        policy_hash="policy-sha",
                        wave_count=3,
                        candidate_count=8,
                        chain_depth=3,
                        attempt_count=9,
                        disclosure_status="pending",
                        evidence_count=2,
                        created_at="2026-07-13T03:07:00Z",
                        disclosed_at=None,
                        reason_detail="raw exploit details must-not-render",
                    )
                ],
            }
        )

        expected_fragments = [
            "attack_rollout: contract=v2_only rank=3 row_version=9",
            "attack_contract=v2_only",
            "attack_waves: 2",
            "wave id=wave-0 snapshot=snapshot-1 generation=0 status=terminal policy_hash=policy-sha",
            "caps waves=3 candidates=100 depth=3 attempts=200 row_version=4",
            "attack_wave_unit_counts:",
            "wave=wave-0 total=2 status open=0 reasoning=0 review=0 verification=0 terminal=2",
            "closed review=2 verification=2 consolidation pending=0 ready=0 consumed=2 terminal=0",
            "candidate_attempt_ownership: 1",
            "attempt=attempt-1 candidate=candidate-1 wave=wave-1 unit=wave-unit-1 org=org-child ordinal=0 status=running",
            "worker=worker-1 status=running generation=2 specialist=candidate_verifier epoch=6",
            "checkpoint_version=11 checkpoint_present=yes checkpoint_bytes=87 lease_present=yes",
            "attack_lane: 1",
            "lane=global:exploit worker=worker-1 lease_present=yes owner=runtime-a",
            "attack_fact_deltas: 1",
            "delta=delta-1 attempt=attempt-0 candidate=candidate-0 wave=wave-0 unit=wave-unit-0 org=org-child kind=new_surface",
            "canonical=api_endpoint:endpoint-1@7 hash=canonical-sha dedupe=delta-sha",
            "status=consumed consumer_wave=wave-1 evidence_count=2",
            "attack_residual_risks: 1",
            "residual=residual-1 wave=wave-1 unit=wave-unit-1 org=org-child reason=max_waves_exhausted",
            "counters waves=3 candidates=8 depth=3 attempts=9 disclosure=pending evidence_count=2",
        ]
        for fragment in expected_fragments:
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, rendered)

        for secret in [
            "full-lease-token-must-not-render",
            "full-lane-token-must-not-render",
            "raw_exploit_recipe",
            "raw_exploit_payload",
            "raw_exploit_material",
            "raw exploit details",
            "https://secret.invalid/payload",
        ]:
            with self.subTest(secret=secret):
                self.assertNotIn(secret, rendered)

        self.assertNotIn(
            "JOIN attack_fact_delta_evidence",
            query.sql_by_id["attack_fact_deltas"],
        )
        self.assertNotIn(
            "JOIN attack_residual_risk_evidence",
            query.sql_by_id["attack_residual_risks"],
        )
        self.assertNotIn(" JOIN ", query.sql_by_id["attack_wave_unit_counts"])

    def test_duplicate_active_and_cross_org_rows_are_rejected_and_force_legacy_fallback(self) -> None:
        rendered = self.render(
            {
                "runtime_rollout": [
                    row(
                        contract="dual_write_v2_preferred",
                        contract_rank=2,
                        row_version=4,
                        updated_at="2026-07-13T02:00:00Z",
                    )
                ],
                "runtime_operations": [
                    row(
                        operation_id="op-bad",
                        runtime_memory_contract="dual_write_v2_preferred",
                        profile="red_team",
                        current_stage="external_attack_surface",
                        project_scope_id="project-1",
                        engagement_org_id="org-root",
                        superseded_by=None,
                        stage_started_at="2026-07-13T02:01:00Z",
                        state_blob={
                            "graph_flow": {
                                "state": {"seeded": {}, "visited": [], "applied": {}},
                                "next_node": "external_attack_surface",
                            }
                        },
                    )
                ],
                "stage_executions": [
                    row(
                        id="stage-a",
                        stage_kind="external_attack_surface",
                        status="started",
                        started_at="2026-07-13T02:01:00Z",
                        completed_at=None,
                    ),
                    row(
                        id="stage-b",
                        stage_kind="enumeration",
                        status="started",
                        started_at="2026-07-13T02:02:00Z",
                        completed_at=None,
                    ),
                ],
                "scope_snapshots": [
                    row(
                        id="snapshot-1",
                        scope_decision_id="decision-1",
                        project_scope_id="project-1",
                        project_path_at_freeze="/fixture/project",
                        root_organization_id="org-root",
                        mode="included",
                        scope_hash="scope-sha",
                        schema_version=1,
                        frozen_at="2026-07-13T02:00:00Z",
                        sealed_at="2026-07-13T02:00:01Z",
                    )
                ],
                "stage_units": [
                    row(
                        id="unit-foreign",
                        stage_execution_id="stage-a",
                        scope_snapshot_id="snapshot-1",
                        organization_id="org-foreign",
                        stage_kind="external_attack_surface",
                        generation=0,
                        specialist="prober",
                        status="running",
                        gate_attempt=0,
                        row_version=0,
                        started_at="2026-07-13T02:01:10Z",
                        terminal_at=None,
                        scope_member=False,
                    )
                ],
                "stage_workers": [
                    row(
                        id="worker-foreign",
                        stage_execution_id="stage-a",
                        stage_run_unit_id="unit-foreign",
                        organization_id="org-sibling",
                        worker_generation=0,
                        specialist="prober",
                        work_item_kind="organization",
                        work_item_key="org-sibling",
                        agent_path="stage/eas/org-sibling",
                        parent_request_id=None,
                        message_chain_id="chain-foreign",
                        status="running",
                        gate_attempt=0,
                        checkpoint={"turn": 1},
                        checkpoint_version=1,
                        lease_token="lease-foreign",
                        lease_owner="runtime-b",
                        lease_acquired_at="2026-07-13T02:01:10Z",
                        lease_expires_at="2026-07-13T02:01:40Z",
                        heartbeat_at="2026-07-13T02:01:30Z",
                        attempt_epoch=1,
                        active_tool_call_id="tool-foreign",
                        active_tool_started_at="2026-07-13T02:01:35Z",
                        active_tool_name=None,
                        active_tool_status=None,
                        active_tool_request_id=None,
                        lease_expired=True,
                        unit_identity_matches=False,
                        scope_member=False,
                    )
                ],
            }
        )

        self.assertIn("stage_executions: exact_active=2", rendered)
        self.assertIn("anomaly: multiple active stage executions", rendered)
        self.assertIn(
            "cross-org rejection: stage_unit=unit-foreign org=org-foreign is not in snapshot=snapshot-1",
            rendered,
        )
        self.assertIn(
            "cross-org rejection: worker=worker-foreign org=org-sibling does not match its stage unit/scope",
            rendered,
        )
        self.assertIn("anomaly: active tool row tool-foreign is missing", rendered)
        self.assertIn("recovery=manual_required", rendered)
        self.assertIn("anomaly: expired lease with active tool is not recovery_required", rendered)
        self.assertIn(
            "selected_read_source=legacy_fallback legacy_fallback=used",
            rendered,
        )

    def test_unified_investigation_renders_exact_authority_chain(self) -> None:
        query = FixtureQuery(
            {
                "investigation_authorities": [
                    row(
                        authority_id="authority-1",
                        stage_execution_id="investigation-exec-1",
                        owning_stage_run_request_id="stage-run-request-1",
                        scope_snapshot_id="scope-1",
                        contract_version="unified_investigation_authority.v1",
                        created_at="2026-08-02T01:00:00Z",
                    )
                ],
                "investigation_read_sessions": [
                    row(
                        authority_id="authority-1",
                        session_set_id="session-set-1",
                        session_set_status="sealed",
                        session_set_member_count=1,
                        session_set_member_sha256="sha256:session-set",
                        main_read_session_id="main-read-1",
                        stage_execution_id="investigation-exec-1",
                        owning_stage_run_request_id="stage-run-request-1",
                        stage_run_unit_id="unit-org-1",
                        scope_snapshot_id="scope-1",
                        organization_id="org-1",
                        snapshot_id="analysis-authority-1",
                        snapshot_sha256="sha256:analysis-authority",
                        context_chain_id="context-1",
                        transcript_partition_id="transcript-1",
                        receipt_id="read-receipt-1",
                        receipt_sha256="sha256:read-receipt",
                        context_item_count=12,
                        methodology_hit_count=4,
                        omission_count=0,
                        created_at="2026-08-02T01:01:00Z",
                    )
                ],
                "investigation_analysis_authorities": [
                    row(
                        snapshot_id="analysis-authority-1",
                        authority_id="authority-1",
                        stage_execution_id="investigation-exec-1",
                        owning_stage_run_request_id="stage-run-request-1",
                        stage_run_unit_id="unit-org-1",
                        scope_snapshot_id="scope-1",
                        organization_id="org-1",
                        snapshot_sha256="sha256:analysis-authority",
                        context_item_count=12,
                        methodology_hit_count=4,
                        omission_count=0,
                        sealed_at="2026-08-02T01:01:00Z",
                    )
                ],
                "investigation_candidate_snapshots": [
                    row(
                        snapshot_id="candidate-snapshot-1",
                        organization_id="org-1",
                        wave_ordinal=0,
                        genesis=True,
                        previous_generation_seal_id=None,
                        fact_delta_watermark=0,
                        snapshot_status="sealed_ready",
                        authority_hash="sha256:candidate-authority",
                        created_at="2026-08-02T01:02:00Z",
                    )
                ],
                "investigation_generations": [
                    row(
                        generation_id="generation-1",
                        organization_id="org-1",
                        generation_ordinal=0,
                        candidate_snapshot_id="candidate-snapshot-1",
                        previous_generation_id=None,
                        generation_seal_id="generation-seal-1",
                        member_count=1,
                        member_set_hash="sha256:generation-members",
                        open_obligation_set_hash="sha256:obligations",
                        generation_hash="sha256:generation",
                        sealed_at="2026-08-02T01:03:00Z",
                        created_at="2026-08-02T01:02:30Z",
                    )
                ],
                "investigation_hypotheses": [
                    row(
                        generation_id="generation-1",
                        generation_member_id="generation-member-1",
                        generation_ordinal=0,
                        member_ordinal=0,
                        organization_id="org-1",
                        root_id="hypothesis-root-1",
                        revision_id="hypothesis-revision-1",
                        revision_ordinal=0,
                        revision_hash="sha256:hypothesis",
                        subject_kind="endpoint",
                        epistemic_state="supported",
                        lifecycle_state="current",
                        planning_readiness="ready_for_strategy",
                        priority=7,
                        is_current_head=True,
                        head_version=0,
                    )
                ],
                "investigation_admissions": [
                    row(
                        admission_set_id="admission-set-1",
                        generation_id="generation-1",
                        organization_id="org-1",
                        stage_execution_id="investigation-exec-1",
                        stage_run_unit_id="unit-org-1",
                        status="sealed",
                        member_count=1,
                        member_set_sha256="sha256:admission-members",
                        admission_member_id="admission-member-1",
                        generation_member_id="generation-member-1",
                        hypothesis_revision_id="hypothesis-revision-1",
                        disposition="scheduled",
                        reason_code="READY_FOR_VERIFICATION",
                        task_id="verification-task-1",
                        member_sha256="sha256:admission-member",
                    )
                ],
                "investigation_tasks": [
                    row(
                        task_id="verification-task-1",
                        organization_id="org-1",
                        stage_execution_id="investigation-exec-1",
                        stage_run_unit_id="unit-org-1",
                        hypothesis_revision_id="hypothesis-revision-1",
                        hypothesis_revision_sha256="sha256:hypothesis",
                        verification_plan_id="verification-plan-1",
                        verification_plan_sha256="sha256:verification-plan",
                        first_admission_generation_id="generation-1",
                        task_contract_version="hypothesis_verification_task.v1",
                        current_state="terminal",
                        head_version=5,
                        latest_event_id="task-event-5",
                    )
                ],
                "investigation_delegation_census": [
                    row(
                        task_plan_id="task-plan-1",
                        organization_id="org-1",
                        task_plan_status="sealed",
                        subtask_count=2,
                        subtask_set_sha256="sha256:subtasks",
                        census_seal_id="census-1",
                        primary_dispatch_receipt_id="dispatch-primary-1",
                        primary_worker_run_id="worker-primary-1",
                        runnable_subtask_count=2,
                        runnable_subtask_set_sha256="sha256:runnable",
                        dispatch_count=3,
                        dispatch_set_sha256="sha256:dispatches",
                        pipeline_event_count=6,
                        pipeline_event_set_sha256="sha256:pipeline",
                        seal_sha256="sha256:census",
                    )
                ],
                "investigation_assignments": [
                    row(
                        assignment_set_id="assignment-set-1",
                        task_id="verification-task-1",
                        hypothesis_revision_id="hypothesis-revision-1",
                        verification_plan_id="verification-plan-1",
                        status="sealed",
                        member_count=1,
                        member_set_sha256="sha256:assignment-members",
                        assignment_member_id="assignment-member-1",
                        plan_objective_id="plan-objective-1",
                        verification_objective_id="verification-objective-1",
                        assignment_kind="campaign",
                        campaign_id="campaign-1",
                        campaign_state="terminal",
                        campaign_version=1,
                        campaign_terminal_decision="proof",
                        campaign_terminal_hash="sha256:campaign-terminal",
                    )
                ],
                "investigation_prepared_actions": [
                    row(
                        prepared_action_id="prepared-action-1",
                        campaign_id="campaign-1",
                        organization_id="org-1",
                        action_ordinal=0,
                        action_contract_kind="single_action_v1",
                        action_kind="verify.http_get",
                        canonical_request_hash="sha256:request",
                        renderer_version="renderer.v1",
                        risk_tier="T0",
                        state="succeeded",
                        reason_code="completed",
                        residual_id=None,
                        row_version=2,
                    )
                ],
                "investigation_action_authorizations": [
                    row(
                        authorization_receipt_id="authorization-1",
                        prepared_action_id="prepared-action-1",
                        campaign_id="campaign-1",
                        organization_id="org-1",
                        decision="authorized",
                        decision_reason_code="trusted_get",
                        reviewed_action_hash="sha256:reviewed",
                        authorization_hash="sha256:authorization",
                        actor_kind="local_operator",
                        operator_channel="local_cli",
                        residual_id=None,
                    )
                ],
                "investigation_action_executions": [
                    row(
                        action_execution_id="action-execution-1",
                        prepared_action_id="prepared-action-1",
                        authorization_receipt_id="authorization-1",
                        organization_id="org-1",
                        execution_ordinal=1,
                        execution_kind="single_action_v1",
                        state="succeeded",
                        durable_begin_hash="sha256:begin",
                        capability_execution_receipt_id="capability-receipt-1",
                        closeout_hash="sha256:closeout",
                        row_version=2,
                    )
                ],
                "investigation_outcomes": [
                    row(
                        outcome_set_id="outcome-set-1",
                        assignment_set_id="assignment-set-1",
                        task_id="verification-task-1",
                        status="sealed",
                        member_count=1,
                        member_set_sha256="sha256:outcome-members",
                        outcome_member_id="outcome-member-1",
                        campaign_id="campaign-1",
                        outcome_kind="completed",
                        terminal_receipt_id="campaign-terminal-1",
                        terminal_receipt_sha256="sha256:campaign-terminal",
                    )
                ],
                "investigation_fact_deltas": [
                    row(
                        fact_delta_bundle_id="fact-delta-1",
                        campaign_id="campaign-1",
                        campaign_terminal_decision_id="campaign-terminal-1",
                        organization_id="org-1",
                        hypothesis_revision_id="hypothesis-revision-1",
                        verification_objective_id="verification-objective-1",
                        delta_kind="support",
                        fact_delta_hash="sha256:fact-delta",
                        consumption_id="consumption-1",
                        generation_id="generation-2",
                        disposition="applied",
                        consumption_hash="sha256:consumption",
                    )
                ],
                "investigation_fuel": [
                    row(
                        budget_id="budget-1",
                        scope_kind="task",
                        owner_id="verification-task-1",
                        organization_id="org-1",
                        task_id="verification-task-1",
                        axis="campaign",
                        limit_amount=4,
                        reserved_amount=0,
                        consumed_amount=1,
                        unknown_held_amount=1,
                        refunded_before_begin_amount=0,
                        head_version=2,
                    )
                ],
                "investigation_residuals": [
                    row(
                        residual_id="residual-1",
                        organization_id="org-1",
                        revision_id="hypothesis-revision-1",
                        snapshot_id="candidate-snapshot-1",
                        reason_code="operator_followup",
                        owner_kind="operator",
                        residual_hash="sha256:residual",
                        closed_at=None,
                        created_at="2026-08-02T01:10:00Z",
                    )
                ],
                "investigation_closures": [
                    row(
                        closure_id="closure-1",
                        authority_id="authority-1",
                        stage_execution_id="investigation-exec-1",
                        stop_intent_id="stop-1",
                        stop_epoch=1,
                        disposition="pass_with_gaps",
                        work_count=6,
                        work_set_sha256="sha256:work",
                        task_plan_count=1,
                        task_plan_set_sha256="sha256:plans",
                        dispatch_count=3,
                        dispatch_set_sha256="sha256:dispatches",
                        residual_set_sha256="sha256:residuals",
                        closure_sha256="sha256:closure",
                        frozen_work_count=6,
                        frozen_work_set_sha256="sha256:work",
                        stop_receipt_sha256="sha256:stop",
                        closed_at="2026-08-02T01:11:00Z",
                    )
                ],
                "investigation_reporting": [
                    row(
                        report_id="report-1",
                        scope_snapshot_id="scope-1",
                        current_revision_id="report-revision-1",
                        revision_id="report-revision-1",
                        revision_number=1,
                        row_version=2,
                        source_set_hash="report-source-set",
                        validation_status="validated",
                        publication_status="final",
                        input_seal_id="report-input-seal-1",
                        input_source_member_count=8,
                        input_source_set_hash="report-input-source-set",
                        report_input_hash="report-input-hash",
                        source_manifest_count=8,
                    )
                ],
            }
        )

        rendered = "\n".join(
            run_tree._unified_investigation_lines(query, "operation-1", 240)
        )

        expected_in_order = [
            "Main authority=authority-1 execution=investigation-exec-1 request=stage-run-request-1",
            "org=org-1 read_session=main-read-1",
            "Analysis authority_snapshot=analysis-authority-1",
            "Analysis snapshot=candidate-snapshot-1 wave=0 status=sealed_ready",
            "generation=generation-1 ordinal=0",
            "Hypothesis revision=hypothesis-revision-1",
            "Verification admission_set=admission-set-1",
            "Verification task=verification-task-1 state=terminal",
            "PentAGI task_plan=task-plan-1 org=org-1 status=sealed",
            "objective=verification-objective-1 assignment=campaign campaign=campaign-1",
            "Prepared Action=prepared-action-1 campaign=campaign-1",
            "Action authorization=authorization-1 prepared=prepared-action-1 decision=authorized",
            "Action execution=action-execution-1 prepared=prepared-action-1",
            "FactDelta=fact-delta-1 kind=support consumption=consumption-1",
            "campaign_outcome_set=outcome-set-1 status=sealed",
            "Residual=residual-1 org=org-1 revision=hypothesis-revision-1",
            "Investigation run closure=closure-1 authority=authority-1 stop=stop-1",
            "Reporting report=report-1 revision=report-revision-1#1",
        ]
        positions = []
        for fragment in expected_in_order:
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, rendered)
                positions.append(rendered.index(fragment))
        self.assertEqual(positions, sorted(positions))
        self.assertIn("unknown_held=1", rendered)

        expected_query_ids = {
            "investigation_authorities",
            "investigation_read_sessions",
            "investigation_analysis_authorities",
            "investigation_candidate_snapshots",
            "investigation_generations",
            "investigation_hypotheses",
            "investigation_admissions",
            "investigation_tasks",
            "investigation_delegation_census",
            "investigation_assignments",
            "investigation_prepared_actions",
            "investigation_action_authorizations",
            "investigation_action_executions",
            "investigation_outcomes",
            "investigation_fact_deltas",
            "investigation_fuel",
            "investigation_residuals",
            "investigation_closures",
            "investigation_reporting",
        }
        self.assertEqual({query_id for query_id, _params in query.calls}, expected_query_ids)
        self.assertTrue(all(params == ("operation-1",) for _query_id, params in query.calls))
        for query_id in expected_query_ids:
            sql = query.sql_by_id[query_id].lower()
            self.assertIn("operation_id = %s", sql)
            self.assertNotIn("limit 1", sql)
            self.assertNotRegex(sql, r"order\s+by[^;]+\bdesc\b")

    def test_unified_schema_error_is_unavailable_not_checked_empty(self) -> None:
        query = FixtureQuery(
            {
                "investigation_authorities": [("ERR", "relation does not exist")],
                "investigation_read_sessions": [("ERR", "relation does not exist")],
            }
        )

        rendered = "\n".join(
            run_tree._unified_investigation_lines(query, "operation-1", 240)
        )

        self.assertIn("Main authority: unavailable (relation does not exist)", rendered)
        self.assertIn("Main: unavailable (authority schema/query unavailable)", rendered)
        self.assertNotIn("checked_empty", rendered)

    def test_legacy_topology_does_not_query_unified_authority_tables(self) -> None:
        rendered, query = self.render_with_query(
            {
                "runtime_operations": [
                    row(
                        operation_id="legacy-operation",
                        runtime_memory_contract="legacy_v1",
                        attack_execution_contract="legacy_attack_v1",
                        investigation_contract_version="legacy_candidate_v1",
                        investigation_rollout_mode="legacy_only",
                        stage_topology_contract="legacy_candidate_verification_v1",
                        stage_topology_sha256="sha256:legacy-topology",
                        stage_topology_freeze_source="legacy_backfill_v1",
                        profile="red_team",
                        current_stage="attack_candidate",
                        project_scope_id="project-1",
                        engagement_org_id="org-1",
                        superseded_by=None,
                        stage_started_at="2026-08-02T01:00:00Z",
                        state_blob={},
                    )
                ]
            }
        )

        self.assertIn("topology=legacy_candidate_verification_v1", rendered)
        self.assertNotIn("unified Investigation authority chain", rendered)
        self.assertFalse(
            any(query_id.startswith("investigation_") for query_id, _params in query.calls)
        )


if __name__ == "__main__":
    unittest.main()
