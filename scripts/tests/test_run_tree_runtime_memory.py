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


if __name__ == "__main__":
    unittest.main()
