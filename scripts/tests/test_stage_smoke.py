import os
import sys
import tempfile
import unittest
import urllib.parse
import urllib.request
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path

from scripts import stage_smoke


class StageSmokeRouteBudgetTests(unittest.TestCase):
    def parse(self, *args: str):
        return stage_smoke.build_parser().parse_args(list(args))

    def test_candidate_chain_from_scoping_includes_enumeration(self):
        args = self.parse(
            "--profile", "red_team", "--from", "scoping", "--to", "attack_candidate"
        )

        self.assertTrue(stage_smoke.includes_enumeration(args))
        self.assertEqual(
            stage_smoke.route_probe_budget(args),
            (
                stage_smoke.DEFAULT_SMOKE_ROUTE_PROBE_MAX_RUNTIME_MS,
                stage_smoke.DEFAULT_SMOKE_ROUTE_PROBE_MAX_REQUESTS,
            ),
        )

    def test_default_budget_can_finish_the_builtin_closed_route_plan_once(self):
        root = Path(stage_smoke.__file__).resolve().parent.parent
        wordlist = root / "resources" / "wordlists" / "route_probe_1.txt"
        entries = [
            line
            for raw in wordlist.read_text(encoding="utf-8").splitlines()
            if (line := raw.strip()) and not line.startswith("#")
        ]

        self.assertGreater(
            stage_smoke.DEFAULT_SMOKE_ROUTE_PROBE_MAX_REQUESTS,
            len(entries) + 100,
        )
        self.assertGreaterEqual(
            stage_smoke.DEFAULT_SMOKE_ROUTE_PROBE_MAX_RUNTIME_MS,
            60_000,
        )

    def test_positional_candidate_defaults_to_scoping_and_includes_enumeration(self):
        args = self.parse("--profile", "red_team", "attack_candidate")

        self.assertTrue(stage_smoke.includes_enumeration(args))

    def test_slice_starting_after_enumeration_does_not_inject_budget(self):
        args = self.parse("--from", "vuln_triage", "--to", "attack_candidate")

        self.assertFalse(stage_smoke.includes_enumeration(args))
        self.assertIsNone(stage_smoke.route_probe_budget(args))

    def test_only_candidate_does_not_claim_enumeration(self):
        args = self.parse("--only", "attack_candidate")

        self.assertFalse(stage_smoke.includes_enumeration(args))

    def test_only_enumeration_keeps_the_existing_budget(self):
        args = self.parse("--only", "enumeration")

        self.assertTrue(stage_smoke.includes_enumeration(args))

    def test_unified_reporting_chain_includes_enumeration(self):
        args = self.parse(
            "--unified-topology", "--from", "scoping", "--to", "reporting"
        )

        self.assertTrue(stage_smoke.includes_enumeration(args))

    def test_red_team_company_controller_slice_requires_v2_before_operation_creation(self):
        parser = stage_smoke.build_parser()
        args = parser.parse_args(
            ["--profile", "red_team", "--from", "scoping", "--to", "enumeration"]
        )
        stage_smoke.resolve_stage_args(args, parser)

        with self.assertRaises(SystemExit):
            stage_smoke.validate_stage_contract_args(args, parser)

    def test_unified_red_team_company_controller_slice_passes_preflight(self):
        parser = stage_smoke.build_parser()
        args = parser.parse_args(
            [
                "--profile",
                "red_team",
                "--unified-topology",
                "--from",
                "scoping",
                "--to",
                "enumeration",
            ]
        )
        stage_smoke.resolve_stage_args(args, parser)
        stage_smoke.validate_stage_contract_args(args, parser)
        self.assertTrue(args.unified_topology)

    def test_exact_organization_identity_is_preserved_as_wrapper_input(self):
        args = self.parse(
            "--org",
            "杭州默安科技有限公司",
            "--org-id",
            "19d56caa-d894-4bd3-954a-9a6709a6f560",
            "--to",
            "reporting",
        )
        self.assertEqual(args.org_id, "19d56caa-d894-4bd3-954a-9a6709a6f560")

    def test_unified_slice_starting_at_application_understanding_skips_budget(self):
        args = self.parse(
            "--unified-topology",
            "--from",
            "application_understanding",
            "--to",
            "reporting",
        )

        self.assertFalse(stage_smoke.includes_enumeration(args))

    def test_controlled_fixture_alias_selects_the_same_local_fixture(self):
        canonical = self.parse("--fixture-web")
        acceptance_alias = self.parse("--controlled-fixture")
        stage_smoke.apply_acceptance_preset(acceptance_alias)

        self.assertTrue(canonical.fixture_web)
        self.assertTrue(acceptance_alias.fixture_web)
        self.assertTrue(acceptance_alias.unified_topology)
        self.assertTrue(acceptance_alias.json)

    def test_oversized_json_events_are_bounded_without_losing_identity(self):
        event = {
            "type": "sub_agent_tool_result",
            "stage": "enumeration",
            "status": "failed",
            "code": "ENUMERATION_TEST_FAILURE",
            "result": "x" * (stage_smoke.MAX_STREAMED_JSON_EVENT_CHARS + 1),
        }
        line = __import__("json").dumps(event) + "\n"
        output = StringIO()

        with redirect_stdout(output):
            stage_smoke.emit_bounded_json_event(event, line)

        rendered = output.getvalue()
        self.assertLess(len(rendered), 1_024)
        self.assertIn("sub_agent_tool_result", rendered)
        self.assertIn("ENUMERATION_TEST_FAILURE", rendered)
        self.assertNotIn("x" * 1_024, rendered)

    def _valid_unified_reporting_summary(self):
        summary = {
            "operation_identity": {
                "stageTopologyContract": "unified_investigation_v1",
                "investigationContractVersion": "hypothesis_registry_v1",
                "investigationRolloutMode": "new_only",
            },
            "operation_exact_sets": {
                "stage_runs": {
                    "members": [
                        {
                            "stageExecutionId": f"stage-{index}",
                            "stage": stage,
                            "status": "completed",
                        }
                        for index, stage in enumerate(stage_smoke.UNIFIED_STAGE_ORDER)
                    ]
                },
                "enumeration_lane_receipts": {
                    "members": [
                        {
                            "lane": lane,
                            "missing": [],
                            "receiptId": lane,
                            "targetId": "target-1",
                            "resolutionOccurrenceId": (
                                "occurrence-1" if lane == "resolution" else None
                            ),
                        }
                        for lane in (
                            "browser",
                            "js_api",
                            "parameter",
                            "resolution",
                            "coverage",
                        )
                    ]
                },
            },
        }
        exact_sets = summary["operation_exact_sets"]
        for label in stage_smoke.REQUIRED_OPERATION_EXACT_SET_LABELS:
            exact_set = exact_sets.setdefault(label, {"memberCount": 0})
            exact_set.setdefault("members", [])
            exact_set.setdefault("memberSetHash", "sha256:" + "a" * 64)
            exact_set.setdefault("memberCount", len(exact_set["members"]))
        for label in (
            "deliverable_submissions",
            "capability_receipts",
            "evidence_ledger",
            "target_intel_goal_epochs",
            "target_intel_goal_reviews",
            "target_intel_goal_frontier",
            "target_intel_goal_work_journal",
            "target_intel_semantic_artifacts",
            "target_intel_asset_observations",
            "stage_handoffs",
            "enumeration_endpoint_occurrences",
            "enumeration_parameter_assessments",
            "enumeration_occurrence_parameters",
            "enumeration_parameter_provenance",
            "enumeration_resolution_closeouts",
            "application_model_revisions",
            "investigation_analysis_bindings",
            "hypothesis_revisions",
            "verification_prepared_actions",
            "verification_action_authorizations",
            "verification_fact_deltas",
            "hypothesis_residual_risks",
            "investigation_run_closures",
            "investigation_stop_intents",
            "investigation_projection_outbox",
            "investigation_closure_publications",
            "report_source_manifest",
        ):
            exact_sets[label]["members"] = [{"id": label}]
            exact_sets[label]["memberCount"] = 1
        exact_sets["investigation_run_heads"]["members"] = [{"authorityId": "run-1"}]
        exact_sets["investigation_main_read_sessions"]["members"] = [
            {
                "organizationId": "org-1",
                "contextChainId": "chain-1",
                "transcriptPartitionId": "partition-1",
            }
        ]
        exact_sets["investigation_task_plans"]["members"] = [
            {"taskPlanId": "task-plan-1"}
        ]
        exact_sets["investigation_delegation_census"]["members"] = [
            {
                "taskPlanId": "task-plan-1",
                "primaryDispatchReceiptId": "dispatch-1",
                "primaryWorkerRunId": "worker-1",
            }
        ]
        exact_sets["hypothesis_verification_tasks"]["members"] = [
            {"taskId": "verification-task-1", "currentState": "terminal"}
        ]
        exact_sets["verification_campaigns"]["members"] = [
            {"campaignId": "campaign-1", "state": "terminal"}
        ]
        exact_sets["report_revisions"]["members"] = [
            {
                "revisionId": "report-revision-1",
                "validationStatus": "validated",
                "publicationStatus": "unpublished",
            }
        ]
        for exact_set in exact_sets.values():
            exact_set["memberCount"] = len(exact_set["members"])
        return summary

    def test_unified_reporting_summary_requires_exact_closed_sets(self):
        summary = self._valid_unified_reporting_summary()
        stage_smoke.validate_unified_reporting_summary(summary)

        summary["operation_exact_sets"]["verification_campaigns"]["members"] = []
        summary["operation_exact_sets"]["verification_campaigns"]["memberCount"] = 0
        with self.assertRaisesRegex(RuntimeError, "verification_campaigns"):
            stage_smoke.validate_unified_reporting_summary(summary)

        summary["operation_exact_sets"]["verification_campaigns"]["members"] = [
            {"campaignId": "campaign-1", "state": "terminal"}
        ]
        summary["operation_exact_sets"]["verification_campaigns"]["memberCount"] = 2
        with self.assertRaisesRegex(RuntimeError, "malformed_exact_sets"):
            stage_smoke.validate_unified_reporting_summary(summary)

        summary["operation_exact_sets"]["verification_campaigns"]["memberCount"] = 1
        stage_members = summary["operation_exact_sets"]["stage_runs"]["members"]
        stage_members.append(
            {
                "stageExecutionId": "duplicate-investigation",
                "stage": "investigation",
                "status": "completed",
            }
        )
        summary["operation_exact_sets"]["stage_runs"]["memberCount"] += 1
        with self.assertRaisesRegex(RuntimeError, "investigation_run_count=2"):
            stage_smoke.validate_unified_reporting_summary(summary)

    def test_unified_reporting_summary_requires_summary_only_persistence(self):
        summary = self._valid_unified_reporting_summary()
        stage_smoke.validate_unified_reporting_summary(summary)

        report = summary["operation_exact_sets"]["report_revisions"]["members"][0]
        report["publicationStatus"] = "final"
        with self.assertRaisesRegex(RuntimeError, "invalid_summary_reports"):
            stage_smoke.validate_unified_reporting_summary(summary)

        summary = self._valid_unified_reporting_summary()
        input_seals = summary["operation_exact_sets"]["report_input_seals"]
        input_seals["members"] = [{"sealId": "legacy-template-seal"}]
        input_seals["memberCount"] = 1
        with self.assertRaisesRegex(RuntimeError, "unexpected_report_input_seals=True"):
            stage_smoke.validate_unified_reporting_summary(summary)

        summary = self._valid_unified_reporting_summary()
        artifacts = summary["operation_exact_sets"]["report_revision_artifacts"]
        artifacts["members"] = [{"artifactKind": "html"}]
        artifacts["memberCount"] = 1
        with self.assertRaisesRegex(RuntimeError, "unexpected_report_artifacts=True"):
            stage_smoke.validate_unified_reporting_summary(summary)

    def test_unified_reporting_summary_allows_distinct_resolution_occurrences(self):
        summary = self._valid_unified_reporting_summary()
        lane_set = summary["operation_exact_sets"]["enumeration_lane_receipts"]
        lane_set["members"].append(
            {
                "lane": "resolution",
                "missing": [],
                "receiptId": "resolution-2",
                "targetId": "target-1",
                "resolutionOccurrenceId": "occurrence-2",
            }
        )
        lane_set["memberCount"] = len(lane_set["members"])

        stage_smoke.validate_unified_reporting_summary(summary)

        lane_set["members"][-1]["resolutionOccurrenceId"] = "occurrence-1"
        with self.assertRaisesRegex(RuntimeError, "duplicate_lane_subjects=True"):
            stage_smoke.validate_unified_reporting_summary(summary)

    def test_controlled_fixture_exposes_spa_chunks_and_safe_api_routes(self):
        fixture = stage_smoke.start_fixture_web()
        try:
            with urllib.request.urlopen(f"{fixture.url}/dashboard") as response:
                dashboard = response.read().decode("utf-8")
            self.assertIn("controlled red-team fixture", dashboard)

            with urllib.request.urlopen(f"{fixture.url}/api/debug/config") as response:
                config = response.read().decode("utf-8")
                self.assertEqual(response.headers["X-Golish-Fixture"], "controlled-red-team-v2")
            self.assertIn('"debug":true', config)

            self.assertTrue((fixture.root / "assets" / "profile.chunk.js").is_file())
            self.assertTrue((fixture.root / "openapi.json").is_file())
            self.assertTrue((fixture.root / "intel" / "company.json").is_file())
            self.assertIn("http://localhost:", dashboard)
            self.assertNotEqual(
                urllib.parse.urlparse(fixture.external_sdk_url).netloc,
                urllib.parse.urlparse(fixture.url).netloc,
            )
            fixture.assert_read_only_safety()
        finally:
            fixture.stop()

    def test_controlled_provider_overlay_removes_real_intel_and_targets_fixture(self):
        fixture = stage_smoke.start_fixture_web()
        try:
            toolsconfig_dir, intel_providers_dir = (
                stage_smoke.build_controlled_provider_overlay(fixture)
            )
            self.assertFalse((toolsconfig_dir / "enscan-go.json").exists())
            self.assertTrue((toolsconfig_dir / "httpx.json").exists())
            descriptors = list(intel_providers_dir.glob("*.json"))
            self.assertEqual(len(descriptors), 1)
            descriptor = __import__("json").loads(descriptors[0].read_text())
            request_url = descriptor["tool"]["asset_intel"]["runtime"]["requests"][0][
                "url"
            ]
            self.assertTrue(request_url.startswith(fixture.url + "/intel/company.json?"))
            self.assertNotIn("api.github.com", request_url)
        finally:
            fixture.stop()

    def test_controlled_nuclei_overlay_is_closed_and_get_only(self):
        fixture = stage_smoke.start_fixture_web()
        try:
            template_root = stage_smoke.build_controlled_nuclei_overlay(fixture)
            templates = sorted(template_root.rglob("*.yaml"))
            self.assertEqual(len(templates), 8)
            self.assertEqual(len(list((template_root / "http").glob("*.yaml"))), 5)
            self.assertEqual(len(list((template_root / "dast").glob("*.yaml"))), 3)
            rendered = "\n".join(path.read_text() for path in templates)
            self.assertNotIn("method: POST", rendered)
            self.assertNotIn("interactsh", rendered)
            self.assertNotIn("http://", rendered)
            self.assertNotIn("https://", rendered)
            for tag in (
                "default-login",
                "csrf",
                "exposure",
                "ssl",
                "disclosure",
                "sqli",
                "xss",
                "rce",
            ):
                self.assertIn(f"tags: {tag}", rendered)
        finally:
            fixture.stop()

    def test_controlled_intel_transport_requires_a_fixture_hit(self):
        fixture = stage_smoke.start_fixture_web()
        try:
            with self.assertRaisesRegex(RuntimeError, "local provider transport"):
                fixture.assert_controlled_intel_transport()
            with urllib.request.urlopen(f"{fixture.url}/intel/company.json") as response:
                self.assertIn("controlled-fixture-intel", response.read().decode("utf-8"))
            fixture.assert_controlled_intel_transport()
        finally:
            fixture.stop()

    def test_controlled_fixture_fails_closed_if_dangerous_get_is_sent(self):
        fixture = stage_smoke.start_fixture_web()
        try:
            fixture.server.record_request("GET", "/logout")
            with self.assertRaisesRegex(RuntimeError, "GET /logout"):
                fixture.assert_read_only_safety()
        finally:
            fixture.stop()

    def test_controlled_fixture_fails_closed_on_any_state_changing_request(self):
        fixture = stage_smoke.start_fixture_web()
        try:
            request = urllib.request.Request(
                f"{fixture.url}/api/orders",
                data=b'{"sku":"fixture","quantity":1}',
                method="POST",
                headers={"Content-Type": "application/json"},
            )
            with urllib.request.urlopen(request) as response:
                self.assertIn('"accepted":true', response.read().decode("utf-8"))
            with self.assertRaisesRegex(RuntimeError, "POST /api/orders"):
                fixture.assert_read_only_safety()
        finally:
            fixture.stop()

    def test_controlled_fixture_request_audit_is_bounded(self):
        fixture = stage_smoke.start_fixture_web()
        try:
            for index in range(stage_smoke.FIXTURE_AUDIT_SAMPLE_LIMIT + 100):
                fixture.server.record_request("GET", f"/wordlist-{index}")
            output = StringIO()
            with redirect_stdout(output):
                fixture.assert_read_only_safety()
            rendered = output.getvalue()
            self.assertIn("100 route(s) omitted", rendered)
            self.assertLess(len(rendered), 4096)
        finally:
            fixture.stop()

    def test_streaming_suppresses_token_deltas_but_keeps_tool_results(self):
        output = StringIO()
        with redirect_stdout(output):
            stage_smoke.emit_bounded_json_event(
                {"type": "sub_agent_text_delta", "delta": "secretly huge"},
                '{"type":"sub_agent_text_delta"}\n',
            )
            stage_smoke.emit_bounded_json_event(
                {"type": "tool_result", "status": "ok"},
                '{"type":"tool_result","status":"ok"}\n',
            )
        self.assertNotIn("sub_agent_text_delta", output.getvalue())
        self.assertIn("tool_result", output.getvalue())

    def test_controlled_fixture_requires_expected_enumeration_requests(self):
        fixture = stage_smoke.start_fixture_web()
        try:
            with self.assertRaisesRegex(RuntimeError, "missing_main_gets"):
                fixture.assert_enumeration_capture()

            for route in (
                "/assets/bootstrap.js",
                "/assets/app.chunk.js",
                "/assets/profile.chunk.js",
                "/api/users",
                "/api/debug/config",
                "/api/orders/fixture-order",
            ):
                fixture.server.record_request("GET", route)
            fixture.assert_enumeration_capture()

            fixture.auxiliary_servers[0][0].record_request(
                "GET", "/vendor/analytics-sdk.js"
            )
            with self.assertRaisesRegex(RuntimeError, "external_sdk_hits=1"):
                fixture.assert_enumeration_capture()
        finally:
            fixture.stop()

    def test_run_tree_queries_live_ephemeral_db_before_stage_exit(self):
        with tempfile.TemporaryDirectory() as temporary_dir:
            ack_path = Path(temporary_dir) / "diagnostic.complete"
            stage_code = (
                "import json,pathlib,time\n"
                "print(json.dumps({'type':'db_smoke_summary','summary':{'ok':True}}),flush=True)\n"
                "print(json.dumps({'type':'db_smoke_diagnostic_ready',"
                "'dbUrl':'postgres://golish:fixture@127.0.0.1:15433/golish'}),flush=True)\n"
                f"p=pathlib.Path({str(ack_path)!r})\n"
                "deadline=time.time()+5\n"
                "while not p.is_file() and time.time()<deadline: time.sleep(.01)"
                "\nraise SystemExit(0 if p.is_file() else 9)"
            )
            run_tree_code = (
                "import sys;"
                "assert sys.argv[-2]=='--db-url';"
                "assert sys.argv[-1].startswith('postgres://golish:')"
            )

            captured_stdout = StringIO()
            with redirect_stdout(captured_stdout):
                returncode, summary = stage_smoke.run_stage_process(
                    [sys.executable, "-c", stage_code],
                    stage_smoke.repo_root(),
                    os.environ.copy(),
                    run_tree_cmd=[sys.executable, "-c", run_tree_code],
                    run_tree_ack_path=ack_path,
                )

            self.assertEqual(returncode, 0)
            self.assertEqual(summary, {"ok": True})
            self.assertTrue(ack_path.is_file())
            self.assertNotIn("db_smoke_summary", captured_stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
