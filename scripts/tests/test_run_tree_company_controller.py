from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from scripts import run_tree


CHAIN = "00000000-0000-0000-0000-0000000000c1"
CHILD_CHAIN = "00000000-0000-0000-0000-0000000000c2"


def write_jsonl(path: Path, events: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(event) + "\n" for event in events))


class CompanyControllerTranscriptTests(unittest.TestCase):
    def make_session(self, root: Path) -> Path:
        session = root / "session-controller"
        write_jsonl(
            session / "transcript.json",
            [
                {
                    "type": "tool_request",
                    "request_id": "stage-call",
                    "tool_name": "stage_run",
                    "args": {"orgs": ["org-1"]},
                },
                {
                    "type": "tool_result",
                    "request_id": "stage-call",
                    "result": {"success": True},
                },
                {
                    "type": "harness_trace",
                    "kind": "stage_run_org_progress",
                    "stage": "target_intel",
                    "org_id": "org-1",
                    "org_name": "Acme",
                    "status": "running",
                    "evidence_count": 0,
                    "activity": "Company Controller is planning this company",
                },
                {
                    "type": "harness_trace",
                    "kind": "gate_decision",
                    "stage": "target_intel",
                    "gate": "BLOCK",
                    "first_blocking_reason": "DNS evidence missing",
                },
                {
                    "type": "harness_trace",
                    "kind": "gate_decision",
                    "stage": "target_intel",
                    "gate": "PASS",
                    "findings": 0,
                },
            ],
        )
        controller_parent = "stage-call::team::org-1::lead:worker-lead"
        controller_dir = session / "subagents" / f"recon-{controller_parent}::org::org-1"
        long_explanation = "Review company evidence " + ("carefully " * 20)
        long_step = "Delegate DNS " + ("carefully " * 20)
        write_jsonl(
            controller_dir / "transcript.json",
            [
                {
                    "_timestamp": "01",
                    "type": "sub_agent_tool_request",
                    "agent_id": "recon",
                    "tool_name": "update_plan",
                    "request_id": "plan-1",
                    "args": {
                        "explanation": long_explanation,
                        "plan": [
                            {"step": "Inspect evidence", "status": "completed"},
                            {"step": long_step, "status": "in_progress"},
                            {"step": "Submit Gate", "status": "pending"},
                        ],
                    },
                },
                {
                    "_timestamp": "02",
                    "type": "sub_agent_tool_result",
                    "agent_id": "recon",
                    "tool_name": "update_plan",
                    "request_id": "plan-1",
                    "success": True,
                    "result": {
                        "success": True,
                        "explanation": long_explanation,
                        "summary": {"total": 3, "completed": 1, "in_progress": 1, "pending": 1},
                        "plan": [
                            {"step": "Inspect evidence", "status": "completed"},
                            {"step": long_step, "status": "in_progress"},
                            {"step": "Submit Gate", "status": "pending"},
                        ],
                    },
                },
                {
                    "_timestamp": "03",
                    "type": "sub_agent_tool_request",
                    "agent_id": "recon",
                    "tool_name": "stage_team_dispatch_workers",
                    "request_id": "dispatch-1",
                    "args": {
                        "workers": [
                            {
                                "dedupe_key": "dns",
                                "role": "intel_provider",
                                "kind": "provider_followup",
                                "objective": "Collect DNS evidence",
                            }
                        ]
                    },
                },
                {
                    "_timestamp": "04",
                    "type": "sub_agent_tool_result",
                    "agent_id": "recon",
                    "tool_name": "stage_team_dispatch_workers",
                    "request_id": "dispatch-1",
                    "success": True,
                    "result": {
                        "status": "dispatch_accepted",
                        "request_count": 1,
                        "accepted_count": 1,
                        "rejected_count": 0,
                        "requests": [{"created_work_item_id": "item-dns", "decision": "accepted"}],
                    },
                },
                {
                    "_timestamp": "05",
                    "type": "sub_agent_completed",
                    "agent_id": "recon",
                    "response": f'{{"status":"dispatch_accepted"}}\n\n[sub_agent_session_id: {CHAIN}]',
                },
                {
                    "_timestamp": "06",
                    "type": "sub_agent_tool_request",
                    "agent_id": "recon",
                    "tool_name": "stage_team_prepare_final_submission",
                    "request_id": "prepare-1",
                    "args": {},
                },
                {
                    "_timestamp": "07",
                    "type": "sub_agent_tool_result",
                    "agent_id": "recon",
                    "tool_name": "stage_team_prepare_final_submission",
                    "request_id": "prepare-1",
                    "success": True,
                    "result": {"status": "prepare_final", "request_epoch_closed": True},
                },
                {
                    "_timestamp": "08",
                    "type": "sub_agent_tool_request",
                    "agent_id": "recon",
                    "tool_name": "submit_stage_deliverable",
                    "request_id": "submit-1",
                    "args": {"stage": "target_intel"},
                },
                {
                    "_timestamp": "09",
                    "type": "sub_agent_tool_result",
                    "agent_id": "recon",
                    "tool_name": "submit_stage_deliverable",
                    "request_id": "submit-1",
                    "success": True,
                    "result": {"status": "needs_fix", "reasons": ["DNS evidence missing"]},
                },
                {
                    "_timestamp": "10",
                    "type": "sub_agent_completed",
                    "agent_id": "recon",
                    "response": f'needs repair\n\n[sub_agent_session_id: {CHAIN}]',
                },
                {
                    "_timestamp": "11",
                    "type": "sub_agent_tool_request",
                    "agent_id": "recon",
                    "tool_name": "stage_team_prepare_final_submission",
                    "request_id": "prepare-2",
                    "args": {},
                },
                {
                    "_timestamp": "12",
                    "type": "sub_agent_tool_result",
                    "agent_id": "recon",
                    "tool_name": "stage_team_prepare_final_submission",
                    "request_id": "prepare-2",
                    "success": True,
                    "result": {"status": "prepare_final", "request_epoch_closed": True},
                },
                {
                    "_timestamp": "13",
                    "type": "sub_agent_tool_request",
                    "agent_id": "recon",
                    "tool_name": "submit_stage_deliverable",
                    "request_id": "submit-2",
                    "args": {"stage": "target_intel"},
                },
                {
                    "_timestamp": "14",
                    "type": "sub_agent_tool_result",
                    "agent_id": "recon",
                    "tool_name": "submit_stage_deliverable",
                    "request_id": "submit-2",
                    "success": True,
                    "result": {"status": "accepted"},
                },
                {
                    "_timestamp": "15",
                    "type": "sub_agent_completed",
                    "agent_id": "recon",
                    "response": f'done\n\n[sub_agent_session_id: {CHAIN}]',
                },
            ],
        )
        child_parent = "stage-call::team::org-1::worker:worker-dns"
        child_dir = session / "subagents" / f"recon-{child_parent}::org::org-1"
        write_jsonl(
            child_dir / "transcript.json",
            [
                {
                    "_timestamp": "01",
                    "type": "sub_agent_tool_request",
                    "agent_id": "recon",
                    "tool_name": "pentest_run",
                    "request_id": "dns-tool",
                    "args": {"tool_name": "dig", "args": "example.com"},
                },
                {
                    "_timestamp": "02",
                    "type": "sub_agent_tool_result",
                    "agent_id": "recon",
                    "tool_name": "pentest_run",
                    "request_id": "dns-tool",
                    "success": True,
                    "result": {"status": "accepted"},
                },
                {
                    "_timestamp": "03",
                    "type": "sub_agent_completed",
                    "agent_id": "recon",
                    "response": (
                        '{"business_disposition":"found","summary":"DNS found",'
                        '"fact_refs":[{"kind":"dns"}],"evidence_ids":[41],'
                        '"checked_empty_units":[],"blocker_code":null}'
                        f"\n\n[sub_agent_session_id: {CHILD_CHAIN}]"
                    ),
                },
            ],
        )
        return session

    def test_company_controller_timeline_and_same_chain_resume_are_explicit(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session = self.make_session(Path(temp_dir))
            rendered = "\n".join(run_tree.render_session_tree(session, trunc=10_000))

        for fragment in [
            "Company Controller: recon  [org org-1]",
            f"chain={CHAIN}",
            "turns=3 resume=same-chain x2",
            "PLAN completed=1/3 in_progress=1 pending=1",
            "PLAN STEP 1 [completed] Inspect evidence",
            "PLAN STEP 2 [in_progress] Delegate DNS",
            "PLAN STEP 3 [pending] Submit Gate",
            "DISPATCH dynamic requested=1 accepted=1 rejected=0 status=dispatch_accepted",
            "WAIT Lead parked; scheduler draining accepted children",
            "dynamic SubAgent: recon  [org org-1]",
            "WORKER OUTPUT disposition=found facts=1 evidence=[41]",
            "PREPARE FINAL closed=yes status=prepare_final",
            "FINAL SUBMIT needs_fix: DNS evidence missing",
            "RESUME same Company Controller chain",
            "FINAL SUBMIT accepted",
            "GATE BLOCK reason: DNS evidence missing",
            "GATE PASS findings=0",
            "summary: controllers=1 resumes=2 dynamic_dispatches=1 worker_outputs=1 submits=2 needs_fix=1 gate_pass=1 gate_block=1 anomalies=0",
        ]:
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, rendered)
        self.assertNotIn("legacy Stage Team worker", rendered)
        self.assertLess(rendered.index("DISPATCH dynamic"), rendered.index("dynamic SubAgent"))

    def test_default_truncates_plan_explanation_but_full_keeps_it(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session = self.make_session(Path(temp_dir))
            default = "\n".join(run_tree.render_session_tree(session, trunc=100))
            full = "\n".join(run_tree.render_session_tree(session, trunc=10_000))

        self.assertIn("PLAN completed=1/3", default)
        self.assertIn("…", default)
        self.assertIn("carefully carefully carefully carefully carefully", full)
        default_plan = next(
            line for line in default.splitlines() if "PLAN completed=1/3" in line
        )
        full_plan = next(
            line for line in full.splitlines() if "PLAN completed=1/3" in line
        )
        self.assertTrue(default_plan.endswith("…"))
        self.assertLess(len(default_plan), len(full_plan))
        default_step = next(
            line for line in default.splitlines() if "PLAN STEP 2 [in_progress]" in line
        )
        full_step = next(
            line for line in full.splitlines() if "PLAN STEP 2 [in_progress]" in line
        )
        self.assertTrue(default_step.endswith("…"))
        self.assertIn(("carefully " * 20).strip(), full_step)

    def test_main_plan_expands_steps_without_becoming_a_controller(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session = Path(temp_dir) / "main-plan"
            write_jsonl(
                session / "transcript.json",
                [
                    {
                        "type": "tool_request",
                        "request_id": "main-plan-1",
                        "tool_name": "update_plan",
                        "args": {
                            "plan": [
                                {"step": "Resolve scope", "status": "completed"},
                                {"step": "Run company stage", "status": "in_progress"},
                            ]
                        },
                    },
                    {
                        "type": "tool_result",
                        "request_id": "main-plan-1",
                        "result": {
                            "summary": {
                                "total": 2,
                                "completed": 1,
                                "in_progress": 1,
                                "pending": 0,
                            },
                            "plan": [
                                {"step": "Resolve scope", "status": "completed"},
                                {"step": "Run company stage", "status": "in_progress"},
                            ],
                        },
                    },
                ],
            )

            rendered = "\n".join(run_tree.render_session_tree(session, trunc=100))

        self.assertIn("PLAN STEP 1 [completed] Resolve scope", rendered)
        self.assertIn("PLAN STEP 2 [in_progress] Run company stage", rendered)
        self.assertIn("summary: controllers=0", rendered)
        self.assertNotIn("Company Controller:", rendered)

    def test_controller_runtime_anomalies_are_grouped_and_deduplicated(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session = Path(temp_dir)
            (session / "run.log").write_text(
                "ERROR Company Controller is waiting but no runnable child WorkItem remains\n"
                "ERROR Company Controller is waiting but no runnable child WorkItem remains\n"
                "WARN code=STAGE_TEAM_DISPATCH_NONE_ACCEPTED\n"
            )

            rendered = "\n".join(run_tree.runlog_anomalies(session, trunc=100))

        self.assertIn(
            "controller Company Controller is waiting but no runnable child WorkItem remains: x2",
            rendered,
        )
        self.assertIn("controller STAGE_TEAM_DISPATCH_NONE_ACCEPTED: x1", rendered)

    def test_ai_call_summary_separates_main_turns_from_subagent_starts(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session = Path(temp_dir)
            (session / "run.log").write_text(
                "[main-agent] Turn complete: provider=deepseek, model=v4, "
                "tokens={input=100, output=20, total=120}\n"
                "[main-agent] Turn complete: provider=deepseek, model=v4, "
                "tokens={input=80, output=10, total=90}\n"
                "[sub-agent:recon] Executing with main model (no override): "
                "provider=deepseek, model=v4\n"
                "[sub-agent:recon] Executing with main model (no override): "
                "provider=deepseek, model=v4\n"
                "[sub-agent:prober] Executing with main model (no override): "
                "provider=openai, model=gpt-x\n"
            )

            rendered = "\n".join(run_tree.runlog_ai_calls(session))

        self.assertIn(
            "main completed turns=2 tokens input=180 output=30 total=210", rendered
        )
        self.assertIn("provider=deepseek model=v4 turns=2", rendered)
        self.assertIn("sub-agent model starts=3", rendered)
        self.assertIn("starts only; not tool calls or completed turns", rendered)
        self.assertIn("agent=recon provider=deepseek model=v4 starts=2", rendered)
        self.assertIn("agent=prober provider=openai model=gpt-x starts=1", rendered)

    def test_db_tree_distinguishes_current_controller_from_legacy_fixed_team(self) -> None:
        base_plan = {
            "id": "plan-1",
            "stage_run_unit_id": "unit-1",
            "organization_id": "org-1",
            "stage_kind": "target_intel",
            "plan_version": 1,
            "plan_hash": "sha256:plan",
            "aggregator_kind": "worker",
            "max_workers_active": 2,
            "max_workers_total": 4,
            "dynamic_requests_allowed": True,
            "dispatch_epoch": 1,
            "requests_closed_at": "2026-07-15T00:00:00Z",
            "final_submitter_worker_run_id": None,
        }
        legacy_plan = {
            **base_plan,
            "leader_role": "intel_provider",
            "aggregator_role": "intel_aggregator",
            "allowed_worker_roles": ["intel_provider", "intel_aggregator"],
        }
        legacy_items = [
            {
                "id": "provider-1",
                "team_plan_id": "plan-1",
                "stable_key": "provider:dns",
                "role": "intel_provider",
                "status": "completed",
                "required_for_barrier": True,
            },
            {
                "id": "aggregate-1",
                "team_plan_id": "plan-1",
                "stable_key": "aggregate:unit",
                "role": "intel_aggregator",
                "status": "queued",
                "required_for_barrier": False,
            },
        ]
        legacy = "\n".join(
            run_tree._stage_team_tree_lines(
                [legacy_plan], legacy_items, [], [], [], [], trunc=100
            )
        )

        controller_plan = {
            **base_plan,
            "leader_role": "company_stage_controller",
            "aggregator_role": "company_stage_controller",
            "allowed_worker_roles": ["company_stage_controller", "intel_provider"],
        }
        controller_items = [
            {
                "id": "controller-1",
                "team_plan_id": "plan-1",
                "kind": "leader",
                "stable_key": "leader:primary",
                "role": "company_stage_controller",
                "status": "running",
                "required_for_barrier": False,
            },
            {
                "id": "child-1",
                "team_plan_id": "plan-1",
                "kind": "provider_followup",
                "stable_key": "dynamic:dns",
                "role": "intel_provider",
                "status": "completed",
                "required_for_barrier": True,
            },
        ]
        controller = "\n".join(
            run_tree._stage_team_tree_lines(
                [controller_plan], controller_items, [], [], [], [], trunc=100
            )
        )

        self.assertIn("mode=legacy_fixed_team (not Company Controller)", legacy)
        self.assertNotIn("mode=company_controller", legacy)
        self.assertIn(
            "mode=company_controller controller_item=controller-1 "
            "controller_role=company_stage_controller",
            controller,
        )
        self.assertIn("children terminal=1/1", controller)
        self.assertIn("controller=yes", controller)


if __name__ == "__main__":
    unittest.main()
