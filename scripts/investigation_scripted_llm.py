#!/usr/bin/env python3
"""Deterministic OpenAI-compatible model for fresh Investigation path tests.

This server deliberately knows nothing about Golish persistence.  It only reads
the same prompts and JSON schemas a real model receives, then emits valid tool
calls.  That keeps migrations, repositories, runtime orchestration, tool-call
tracking, and every deterministic gate on the production path.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import threading
import time
import uuid
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any


SHA256_RE = re.compile(r"sha256:[0-9a-f]{64}")
UUID_RE = re.compile(
    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-5][0-9a-fA-F]{3}-"
    r"[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}"
)


def stable_uuid(label: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, f"golish-scripted-investigation:{label}"))


def stable_sha256(label: str) -> str:
    return "sha256:" + hashlib.sha256(label.encode("utf-8")).hexdigest()


def message_text(message: dict[str, Any]) -> str:
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts: list[str] = []
        for item in content:
            if isinstance(item, str):
                parts.append(item)
            elif isinstance(item, dict):
                text = item.get("text")
                if isinstance(text, str):
                    parts.append(text)
        return "\n".join(parts)
    return ""


def request_text(request: dict[str, Any]) -> str:
    return "\n".join(
        message_text(message)
        for message in request.get("messages", [])
        if isinstance(message, dict)
    )


def tool_map(request: dict[str, Any]) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for entry in request.get("tools", []):
        if not isinstance(entry, dict):
            continue
        function = entry.get("function")
        if isinstance(function, dict) and isinstance(function.get("name"), str):
            result[function["name"]] = function
    return result


def recent_tool_names(request: dict[str, Any]) -> list[str]:
    messages = request.get("messages", [])
    last_user = -1
    for index, message in enumerate(messages):
        if isinstance(message, dict) and message.get("role") == "user":
            last_user = index
    names: list[str] = []
    for message in messages[last_user + 1 :]:
        if not isinstance(message, dict) or message.get("role") != "assistant":
            continue
        for call in message.get("tool_calls") or []:
            function = call.get("function") if isinstance(call, dict) else None
            if isinstance(function, dict) and isinstance(function.get("name"), str):
                names.append(function["name"])
    return names


def all_tool_names(request: dict[str, Any]) -> list[str]:
    names: list[str] = []
    for message in request.get("messages", []):
        if not isinstance(message, dict) or message.get("role") != "assistant":
            continue
        for call in message.get("tool_calls") or []:
            function = call.get("function") if isinstance(call, dict) else None
            if isinstance(function, dict) and isinstance(function.get("name"), str):
                names.append(function["name"])
    return names


def named_tool_choice(request: dict[str, Any]) -> str | None:
    choice = request.get("tool_choice")
    if not isinstance(choice, dict):
        return None
    function = choice.get("function")
    if isinstance(function, dict) and isinstance(function.get("name"), str):
        return function["name"]
    return None


def stage_run_outcome(request: dict[str, Any]) -> bool | None:
    """Return the latest structured stage_run outcome visible to the model."""
    for message in reversed(request.get("messages", [])):
        if not isinstance(message, dict) or message.get("role") != "tool":
            continue
        content = message_text(message)
        try:
            result = json.loads(content)
        except (TypeError, json.JSONDecodeError):
            continue
        if isinstance(result, dict) and isinstance(result.get("passed"), bool):
            return result["passed"]
    return None


def json_after_marker(text: str, marker: str) -> Any | None:
    start = text.rfind(marker)
    if start < 0:
        return None
    remainder = text[start + len(marker) :].lstrip()
    if not remainder:
        return None
    decoder = json.JSONDecoder()
    try:
        value, _ = decoder.raw_decode(remainder)
        return value
    except json.JSONDecodeError:
        return None


def schema_witness(schema: Any, path: str = "root") -> Any:
    if not isinstance(schema, dict):
        return {}
    if "const" in schema:
        return schema["const"]
    enum = schema.get("enum")
    if isinstance(enum, list) and enum:
        return enum[0]
    one_of = schema.get("oneOf")
    if isinstance(one_of, list) and one_of:
        return schema_witness(one_of[0], path)
    any_of = schema.get("anyOf")
    if isinstance(any_of, list) and any_of:
        return schema_witness(any_of[0], path)
    kind = schema.get("type")
    if isinstance(kind, list):
        kind = next((entry for entry in kind if entry != "null"), "null")
    if kind == "object" or isinstance(schema.get("properties"), dict):
        properties = schema.get("properties") or {}
        required = schema.get("required") or []
        return {
            key: schema_witness(properties.get(key, {}), f"{path}.{key}")
            for key in required
        }
    if kind == "array":
        minimum = int(schema.get("minItems", 0))
        prefix = schema.get("prefixItems")
        if isinstance(prefix, list):
            return [schema_witness(item, f"{path}[{index}]") for index, item in enumerate(prefix)]
        return [
            schema_witness(schema.get("items", {}), f"{path}[{index}]")
            for index in range(minimum)
        ]
    if kind == "integer":
        return max(1, int(schema.get("minimum", 0)))
    if kind == "number":
        return float(schema.get("minimum", 0))
    if kind == "boolean":
        return True
    if kind == "null":
        return None
    if schema.get("format") == "uuid":
        return stable_uuid(path)
    pattern = schema.get("pattern", "")
    if "sha256" in pattern:
        return stable_sha256(path)
    return path.rsplit(".", 1)[-1].replace("_", "-") or "value"


def exact_uuid(text: str, field: str) -> str | None:
    match = re.search(rf'"{re.escape(field)}"\s*:\s*"({UUID_RE.pattern})"', text)
    return match.group(1) if match else None


def exact_integer(text: str, field: str) -> int | None:
    match = re.search(rf'"{re.escape(field)}"\s*:\s*([0-9]+)', text)
    return int(match.group(1)) if match else None


@dataclass
class ScenarioState:
    lock: threading.Lock = field(default_factory=threading.Lock)
    calls: int = 0
    targets: dict[str, int] = field(default_factory=dict)
    primary_turns: dict[str, int] = field(default_factory=dict)
    discovery_emitted: bool = False

    def next_call(self) -> int:
        with self.lock:
            self.calls += 1
            return self.calls

    def target_index(self, target_id: str) -> int:
        with self.lock:
            if target_id not in self.targets:
                self.targets[target_id] = len(self.targets)
            return self.targets[target_id]

    def next_primary_turn(self, session_id: str) -> int:
        with self.lock:
            turn = self.primary_turns.get(session_id, 0)
            self.primary_turns[session_id] = turn + 1
            return turn

    def take_first_discovery(self) -> bool:
        with self.lock:
            if self.discovery_emitted:
                return False
            self.discovery_emitted = True
            return True


STATE = ScenarioState()


def analysis_target_id(text: str) -> str | None:
    matches = re.findall(r"target_id=({})".format(UUID_RE.pattern), text)
    if matches:
        return matches[-1]
    return exact_uuid(text, "target_id")


def analysis_plan(text: str) -> dict[str, Any]:
    target_id = analysis_target_id(text) or stable_uuid("fallback-target")
    target_index = STATE.target_index(target_id)
    count = 0 if target_index == 0 else (2 if target_index == 1 else 1)
    roles = ["researcher", "researcher", "browser"]
    subtasks = [
        {
            "stable_key": f"asset_{target_index}_analysis_{ordinal}",
            "role": roles[ordinal],
            "objective": f"Reason over frozen evidence for asset {target_index}, slice {ordinal}",
            "rationale": "Exercise the dynamic cognition denominator without target I/O",
            "subject_refs": [{"kind": "target", "id": target_id}],
        }
        for ordinal in range(count)
    ]
    return {"schema_version": 1, "summary": "Deterministic bounded analysis plan", "subtasks": subtasks}


def refiner_patch(text: str) -> dict[str, Any]:
    completed = json_after_marker(text, "COMPLETED RESULT:") or {}
    remaining = json_after_marker(text, "CURRENT REMAINING PLAN:") or []
    completed_key = completed.get("stable_key", "completed")
    output_sha = completed.get("output_sha256", stable_sha256(completed_key))
    return {
        "schema_version": 1,
        "summary": "Accepted the completed cognition result",
        "completed_subtask_key": completed_key,
        "accepted_output_sha256": output_sha,
        "remaining_subtasks": remaining,
    }


def proposal_from_authority(authority: dict[str, Any], ordinal: int) -> dict[str, Any]:
    subjects = authority.get("proposal_subjects") or []
    proofs = authority.get("proof_inputs") or []
    subject = subjects[min(ordinal, len(subjects) - 1)] if subjects else {}
    proof = proofs[min(ordinal, len(proofs) - 1)] if proofs else {}
    chunks = proof.get("chunks") or []
    chunk = chunks[0] if chunks else {}
    proposal_id = stable_uuid(f"proposal:{subject.get('subject_id')}:{ordinal}")
    return {
        "proposal_id": proposal_id,
        "subject_kind": subject.get("subject_kind", "asset"),
        "subject_identity_hash": subject.get(
            "subject_identity_hash", stable_sha256(f"subject:{ordinal}")
        ),
        "predicate_schema": "scripted.asset.signal",
        "predicate_version": 1,
        "predicate_arguments": [["signal", f"deterministic-{ordinal}"]],
        "trust_boundary": "fresh-scripted-investigation",
        "polarity": "negative" if ordinal % 2 else "positive",
        "structured_claim": f"Deterministic hypothesis {ordinal} requires verification",
        "preconditions": ["Use only the exact frozen asset authority"],
        "impact": "Validates the dynamic verification and provenance path",
        "proof_refs": [
            {
                "input_id": proof.get("input_id", stable_uuid(f"input:{ordinal}")),
                "chunk_id": chunk.get("chunk_id", stable_uuid(f"chunk:{ordinal}")),
                "source_hash": proof.get("source_sha256", stable_sha256(f"source:{ordinal}")),
                "role": "support",
            }
        ],
        "knowledge_signals": [],
        "readiness": "ready_for_strategy",
    }


def primary_synthesis(text: str) -> dict[str, Any]:
    authority = json_after_marker(text, "SERVER-FROZEN PROOF AUTHORITY:") or {}
    manifest = json_after_marker(text, "IMMUTABLE CHILD OUTPUT MANIFEST:") or {}
    subjects = authority.get("proposal_subjects") or []
    subject_id = str(subjects[0].get("subject_id")) if subjects else "zero"
    target_index = STATE.target_index(subject_id)
    accepted = manifest.get("accepted_output_sha256") or []
    if target_index == 0:
        proposals: list[dict[str, Any]] = []
        residuals: list[dict[str, Any]] = [
            {
                "kind": "no_bounded_hypothesis",
                "reason_code": "sealed_input_did_not_support_a_proof_bound_hypothesis",
            }
        ]
    else:
        proposal_count = 2 if target_index == 1 else 1
        proposals = [proposal_from_authority(authority, ordinal) for ordinal in range(proposal_count)]
        residuals = []
    return {
        "schema_version": 1,
        "summary": "Deterministic synthesis from exact frozen authority",
        "accepted_output_sha256": accepted,
        "proposal_signals": proposals,
        "action_intents": [],
        "residuals": residuals,
    }


def dynamic_proposal(ordinal: int) -> dict[str, Any]:
    return {
        "predicate_schema": "scripted.followup.signal",
        "predicate_version": 1,
        "predicate_arguments": [["followup", str(ordinal)]],
        "trust_boundary": "fresh-scripted-investigation",
        "polarity": "positive",
        "structured_claim": f"Follow-up hypothesis {ordinal}",
        "preconditions": ["Exact current target only"],
        "impact": "Exercises discovery admission and a successor verification root",
        "rationale": "Actor observation exposed a bounded related signal",
    }


def dynamic_primary_turn(text: str) -> dict[str, Any]:
    session_id = exact_uuid(text, "session_id") or stable_uuid("session")
    revision_id = exact_uuid(text, "hypothesis_revision_id") or stable_uuid("revision")
    target_id = exact_uuid(text, "target_id") or stable_uuid("target")
    turn = STATE.next_primary_turn(session_id)
    if turn == 0:
        refs = [
            {"kind": "target", "id": target_id},
            {"kind": "hypothesis_revision", "id": revision_id},
        ]
        return {
            "schema_version": 1,
            "session_id": session_id,
            "hypothesis_revision_id": revision_id,
            "decision": "delegate",
            "subtasks": [
                {
                    "stable_key": "repeat_researcher_a",
                    "role": "researcher",
                    "objective": "Review the exact hypothesis against frozen context",
                    "rationale": "Exercise repeated dynamic roles",
                    "subject_refs": refs,
                },
                {
                    "stable_key": "repeat_researcher_b",
                    "role": "researcher",
                    "objective": "Independently challenge the exact hypothesis",
                    "rationale": "Exercise repeated dynamic roles and actor census",
                    "subject_refs": refs,
                },
            ],
        }
    disposition = ["verified", "refuted", "invalid"][
        int(uuid.UUID(revision_id)) % 3
    ]
    return {
        "schema_version": 1,
        "session_id": session_id,
        "hypothesis_revision_id": revision_id,
        "decision": "resolve",
        "subtasks": [],
        "disposition": disposition,
        "conclusion": "Deterministic actor observations are sufficient for a terminal decision",
        "cited_evidence_ids": [],
        "new_hypothesis_proposals": (
            [dynamic_proposal(0)] if STATE.take_first_discovery() else []
        ),
    }


def actor_observation(text: str) -> dict[str, Any]:
    session_id = exact_uuid(text, "session_id") or stable_uuid("actor-session")
    revision_id = exact_uuid(text, "hypothesis_revision_id") or stable_uuid("actor-revision")
    actor_call_id = exact_uuid(text, "actor_call_id") or stable_uuid("actor-call")
    subtask_id = exact_uuid(text, "subtask_id") or stable_uuid("actor-subtask")
    ordinal = exact_integer(text, "actor_ordinal") or 1
    role_match = re.search(r'"specialist_role"\s*:\s*"([a-z_]+)"', text)
    role = role_match.group(1) if role_match else "researcher"
    return {
        "schema_version": 1,
        "session_id": session_id,
        "hypothesis_revision_id": revision_id,
        "actor_call_id": actor_call_id,
        "actor_ordinal": ordinal,
        "subtask_id": subtask_id,
        "specialist_role": role,
        "summary": "Deterministic evidence-bounded actor observation",
        "cited_evidence_ids": [],
        "new_hypothesis_proposals": [],
    }


def submit_result(request: dict[str, Any], function: dict[str, Any]) -> dict[str, Any]:
    text = request_text(request)
    parameters = function.get("parameters") or {}
    result_schema = (
        parameters.get("properties", {}).get("result", {})
        if isinstance(parameters, dict)
        else {}
    )
    description = str(result_schema.get("description", ""))
    if "InvestigationGeneratedTaskPlanV1" in description:
        result = analysis_plan(text)
    elif "InvestigationRefinerPatchV1" in description:
        result = refiner_patch(text)
    elif "InvestigationPrimarySynthesisV1" in description:
        result = primary_synthesis(text)
    elif "Fresh evidence-grounded observation" in description:
        result = actor_observation(text)
    elif "closed semantic turn" in description:
        result = dynamic_primary_turn(text)
    elif "investigation_cognitive_output.v1" in text:
        result = {
            "business_disposition": "found",
            "summary": "Deterministic cognition over the frozen asset authority",
            "fact_refs": [],
            "evidence_ids": [],
            "checked_empty_units": [],
            "blocker_code": None,
            "proposal_signals": [],
            "action_intents": [],
            "residuals": [],
        }
    else:
        result = schema_witness(result_schema, "result")
    return {"result": result, "success": True, "summary": "scripted Investigation result"}


def choose_tool_call(request: dict[str, Any]) -> tuple[str, dict[str, Any]] | None:
    tools = tool_map(request)
    recent = recent_tool_names(request)
    all_calls = all_tool_names(request)
    forced = named_tool_choice(request)
    if forced in tools:
        name = forced
    elif "stage_run" in tools and "stage_run" not in all_calls:
        name = "stage_run"
    elif stage_run_outcome(request) is True and "submit_stage_deliverable" in tools:
        name = "submit_stage_deliverable"
    elif "pentest_list_tools" in tools and "pentest_list_tools" not in recent:
        name = "pentest_list_tools"
    elif "submit_result" in tools:
        name = "submit_result"
    elif "update_plan" in tools and "update_plan" not in recent:
        name = "update_plan"
    else:
        return None
    function = tools[name]
    if name == "stage_run":
        arguments = {"orgs": []}
    elif name == "submit_stage_deliverable":
        arguments = {"stage_id": "investigation", "claims": []}
    elif name == "update_plan":
        arguments = {
            "explanation": "Exercise the scripted Investigation path",
            "plan": [{"step": "Complete the exact bounded task", "status": "in_progress"}],
        }
    elif name == "submit_result":
        arguments = submit_result(request, function)
    else:
        arguments = schema_witness(function.get("parameters") or {}, name)
    return name, arguments


def completion_payload(
    request: dict[str, Any],
    call_number: int,
    selected: tuple[str, dict[str, Any]] | None,
) -> dict[str, Any]:
    model = str(request.get("model", "golish-investigation-scripted-v1"))
    if selected is None:
        message: dict[str, Any] = {
            "role": "assistant",
            "content": "Scripted fresh Investigation path completed.",
        }
        finish_reason = "stop"
    else:
        name, arguments = selected
        message = {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": f"call_{call_number:08d}",
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": json.dumps(arguments, separators=(",", ":")),
                    },
                }
            ],
        }
        finish_reason = "tool_calls"
    return {
        "id": f"chatcmpl-scripted-{call_number}",
        "object": "chat.completion",
        "created": int(time.time()),
        "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    }


class Handler(BaseHTTPRequestHandler):
    server_version = "GolishInvestigationScriptedLLM/1"

    def log_message(self, fmt: str, *args: Any) -> None:
        sys.stderr.write("scripted-llm http: " + fmt % args + "\n")

    def do_GET(self) -> None:  # noqa: N802
        body = json.dumps({"ok": True, "calls": STATE.calls}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:  # noqa: N802
        if not self.path.rstrip("/").endswith("chat/completions"):
            self.send_error(404)
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            request = json.loads(self.rfile.read(length))
            if stage_run_outcome(request) is False:
                body = json.dumps(
                    {
                        "error": {
                            "message": "scripted path stopped after the first failed stage_run",
                            "type": "scripted_path_failure",
                        }
                    }
                ).encode()
                self.send_response(409)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            call_number = STATE.next_call()
            selected = choose_tool_call(request)
            sys.stderr.write(
                json.dumps(
                    {
                        "call": call_number,
                        "model": request.get("model"),
                        "stream": bool(request.get("stream")),
                        "selected_tool": selected[0] if selected else None,
                        "tools": sorted(tool_map(request)),
                    },
                    sort_keys=True,
                )
                + "\n"
            )
            payload = completion_payload(request, call_number, selected)
            if request.get("stream"):
                message = payload["choices"][0]["message"]
                delta = {"role": "assistant"}
                if message.get("tool_calls"):
                    delta["tool_calls"] = [
                        {
                            "index": 0,
                            "id": message["tool_calls"][0]["id"],
                            "type": "function",
                            "function": message["tool_calls"][0]["function"],
                        }
                    ]
                else:
                    delta["content"] = message.get("content", "")
                chunks = [
                    {
                        "id": payload["id"],
                        "object": "chat.completion.chunk",
                        "created": payload["created"],
                        "model": payload["model"],
                        "choices": [{"index": 0, "delta": delta, "finish_reason": None}],
                    },
                    {
                        "id": payload["id"],
                        "object": "chat.completion.chunk",
                        "created": payload["created"],
                        "model": payload["model"],
                        "choices": [
                            {
                                "index": 0,
                                "delta": {},
                                "finish_reason": payload["choices"][0]["finish_reason"],
                            }
                        ],
                    },
                ]
                body = "".join(
                    f"data: {json.dumps(chunk, separators=(',', ':'))}\n\n" for chunk in chunks
                ) + "data: [DONE]\n\n"
                encoded = body.encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)
            else:
                encoded = json.dumps(payload, separators=(",", ":")).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)
        except Exception as exc:  # pragma: no cover - diagnostic boundary
            sys.stderr.write(f"scripted-llm failure: {exc!r}\n")
            self.send_error(500, explain=str(exc))


def self_test() -> None:
    assert schema_witness({"type": "string", "format": "uuid"}, "x") == stable_uuid("x")
    assert SHA256_RE.fullmatch(
        schema_witness({"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"}, "x")
    )
    request = {
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "stage_run",
                    "parameters": {"type": "object", "required": ["orgs"]},
                },
            }
        ],
        "messages": [{"role": "user", "content": "run"}],
    }
    assert choose_tool_call(request) == ("stage_run", {"orgs": []})
    failed_request = {
        "tools": request["tools"],
        "messages": [
            {
                "role": "assistant",
                "tool_calls": [
                    {
                        "type": "function",
                        "function": {"name": "stage_run", "arguments": "{}"},
                    }
                ],
            },
            {"role": "tool", "content": '{"passed":false,"code":"BLOCKED"}'},
        ],
    }
    assert stage_run_outcome(failed_request) is False
    assert choose_tool_call(failed_request) is None
    session_id = stable_uuid("self-test-session")
    revision_id = stable_uuid("self-test-revision")
    target_id = stable_uuid("self-test-target")
    primary_request = {
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "submit_result",
                    "description": "closed semantic turn",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "result": {
                                "type": "object",
                                "description": "closed semantic turn",
                            }
                        },
                    },
                },
            }
        ],
        "messages": [
            {
                "role": "user",
                "content": json.dumps(
                    {
                        "session_id": session_id,
                        "hypothesis_revision_id": revision_id,
                        "target_id": target_id,
                    }
                ),
            }
        ],
    }
    selected = choose_tool_call(primary_request)
    payload = completion_payload(primary_request, 99, selected)
    arguments = json.loads(
        payload["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
    )
    assert arguments["result"]["decision"] == "delegate"
    assert len(arguments["result"]["subtasks"]) == 2
    selected = choose_tool_call(primary_request)
    payload = completion_payload(primary_request, 100, selected)
    arguments = json.loads(
        payload["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
    )
    assert arguments["result"]["decision"] == "resolve"
    assert len(arguments["result"]["new_hypothesis_proposals"]) == 1
    print("investigation_scripted_llm self-test: ok")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    host, port = server.server_address[:2]
    print(json.dumps({"base_url": f"http://{host}:{port}/v1"}), flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
