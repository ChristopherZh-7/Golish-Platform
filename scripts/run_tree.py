#!/usr/bin/env python3
"""Reconstruct one AI run as a readable call tree (+ optional DB self-diagnosis).

Why: a run's data is spread across `transcript.json` (main agent),
`subagents/<agent>-<parent_req>::org::<org>/transcript.json` (each sub-agent),
the per-run `run.log`, and the DB. Reading the sub->sub call chain by hand means
correlating `parent_request_id` across flat directories. This script stitches it
into ONE indented tree so a human (or a later AI) can see, at a glance:

  main agent -> stage_run -> one Company Controller per company -> Controller
  plan / same-chain resumes -> dynamically dispatched SubAgents -> worker output
  -> final preparation / submit -> Gate PASS/BLOCK. Older Producer/Aggregator
  transcripts remain readable but are explicitly labeled legacy fixed teams.

What it surfaces from `transcript.json` (all already on disk):
  - gate decisions      (`harness_trace` kind=gate_decision: gate + first_blocking_reason)
  - per-org coverage    (kind=stage_run_org_progress: per-technique found/empty/blocked)
  - evidence booked     (kind=evidence_booked), deliverable submits, background notes
  - main agent's FINAL  (`completed`: response + reasoning summary)
  - sub-agent prose      (`sub_agent_reasoning` / `sub_agent_text_delta` /
                          `sub_agent_completed` — the "why", persisted since
                          the 2026-06-16 sub-agent-prose change)
  - model call signals  (`run.log`: completed main turns/tokens and sub-agent
                          model starts; starts are not treated as completions)

`--db` adds a deterministic self-diagnosis against the embedded Postgres for the
root causes transcripts can't show (targets with organization_id=NULL, empty
target_assets/dns_records despite "found" evidence, audit_log technique facts).

Usage:
    scripts/run_tree.py                 # latest session under the transcripts dir
    scripts/run_tree.py <session_id>    # a specific session
    scripts/run_tree.py <path/to/session_dir>
    scripts/run_tree.py --workspace /path/to/ws [<session>]
    scripts/run_tree.py --full          # don't truncate args/reasons/prose
    scripts/run_tree.py --db            # also run DB self-diagnosis
    scripts/run_tree.py --db-url postgres://user:pw@host:port/db   # implies --db

Transcript dir resolution (first hit wins):
    $VT_TRANSCRIPT_DIR  >  <workspace>/.golish/transcripts  >  $QBIT_WORKSPACE/.golish/transcripts
    >  ./.golish/transcripts  >  ~/.golish/transcripts

DB connection (only with --db): --db-url  >  $GOLISH_DB_URL  >
    postgres://golish:golish_local@localhost:15432/golish (the embedded default).
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from uuid import UUID

TRUNC = 100  # default arg/reason/prose truncation; --full disables
DEFAULT_DB_URL = "postgres://golish:golish_local@localhost:15432/golish"


def _load_jsonl(path: Path) -> list[dict]:
    out: list[dict] = []
    try:
        with path.open() as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    out.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
    except FileNotFoundError:
        pass
    return out


def _candidate_roots(workspace: str | None) -> list[Path]:
    roots: list[Path] = []
    if os.environ.get("VT_TRANSCRIPT_DIR"):
        roots.append(Path(os.environ["VT_TRANSCRIPT_DIR"]))
    if workspace:
        roots.append(Path(workspace) / ".golish" / "transcripts")
    if os.environ.get("QBIT_WORKSPACE"):
        roots.append(Path(os.environ["QBIT_WORKSPACE"]).expanduser() / ".golish" / "transcripts")
    roots.append(Path.cwd() / ".golish" / "transcripts")
    roots.append(Path.home() / ".golish" / "transcripts")
    seen, uniq = set(), []
    for r in roots:
        if r not in seen:
            seen.add(r)
            uniq.append(r)
    return uniq


def _is_default_session_candidate(path: Path) -> bool:
    name = path.name
    if name.startswith("_") or name.startswith("."):
        return False
    if name.startswith("title-gen-"):
        return False
    return (path / "transcript.json").exists()


def resolve_session_dir(arg: str | None, workspace: str | None) -> Path:
    # Explicit path to a session dir.
    if arg:
        p = Path(arg)
        if p.is_dir() and (p / "transcript.json").exists():
            return p
    for root in _candidate_roots(workspace):
        if not root.is_dir():
            continue
        if arg:
            cand = root / arg
            if cand.is_dir():
                return cand
        else:
            sessions = [d for d in root.iterdir() if d.is_dir() and _is_default_session_candidate(d)]
            if sessions:
                return max(sessions, key=lambda d: d.stat().st_mtime)
    raise SystemExit(
        f"no session found (arg={arg!r}); looked under: "
        + ", ".join(str(r) for r in _candidate_roots(workspace))
    )


def _short(value, n: int) -> str:
    if value is None:
        return ""
    s = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False)
    s = " ".join(s.split())
    return s if len(s) <= n else s[: n - 1] + "\u2026"


def _tool_summary(tool: str, args, result, trunc: int) -> str:
    """One-line summary for a tool call, spotlighting recon + gate signals."""
    a = args or {}
    r = result if isinstance(result, dict) else {}
    detail = ""
    if tool == "update_plan":
        summary = r.get("summary") if isinstance(r.get("summary"), dict) else {}
        plan = r.get("plan") if isinstance(r.get("plan"), list) else a.get("plan", [])
        total = summary.get("total", len(plan) if isinstance(plan, list) else "?")
        completed = summary.get("completed", "?")
        in_progress = summary.get("in_progress", "?")
        pending = summary.get("pending", "?")
        explanation = r.get("explanation", a.get("explanation"))
        detail = (
            f"completed={completed}/{total} in_progress={in_progress} pending={pending}"
        )
        if explanation:
            explanation_trunc = trunc if trunc >= 10_000 else min(trunc, 72)
            detail += f" explanation={_short(explanation, explanation_trunc)}"
        return f"PLAN {detail}"
    if tool == "stage_team_dispatch_workers":
        workers = a.get("workers") if isinstance(a, dict) else []
        requested = r.get(
            "request_count", len(workers) if isinstance(workers, list) else "?"
        )
        accepted = r.get("accepted_count", "?")
        rejected = r.get("rejected_count", "?")
        status = r.get("status", "?")
        detail = (
            f"dynamic requested={requested} accepted={accepted} rejected={rejected} "
            f"status={status}"
        )
        if r.get("partial_persist_error"):
            detail += f" partial_error={_short(r['partial_persist_error'], trunc)}"
        if r.get("error"):
            detail += f" error={_short(r['error'], trunc)}"
        return f"DISPATCH {detail}"
    if tool == "stage_team_prepare_final_submission":
        return (
            "PREPARE FINAL "
            f"closed={_yes_no(r.get('request_epoch_closed'))} status={r.get('status', '?')}"
        )
    if tool == "pentest_run":
        cmd = (a.get("tool_name", "") + " " + a.get("args", "")).strip()
        detail = cmd or _short(a, trunc)
    elif tool in ("run_pty_cmd", "run_command"):
        detail = _short(a.get("command", a), trunc)
    elif tool == "submit_stage_deliverable":
        st = r.get("status")
        reasons = r.get("reasons")
        if isinstance(reasons, list) and reasons:
            detail = f"{st}: {_short(reasons[0], trunc)}"
        else:
            detail = st or _short(a, trunc)
        return f"FINAL SUBMIT {detail}".rstrip()
    else:
        detail = _short(a, trunc)
    # outcome marker
    mark = ""
    if isinstance(result, dict):
        if result.get("error"):
            mark = " \u2717 " + _short(result.get("error"), 60)
        elif result.get("status") == "needs_fix":
            mark = " \u26d4"
        elif result.get("status") in ("accepted", "received"):
            mark = " \u2713"
    return f"{tool} {detail}{mark}".rstrip()


def _plan_step_summaries(args: object, result: object, trunc: int) -> list[str]:
    """Render up to twelve user-visible plan steps without implying ownership."""
    request = args if isinstance(args, dict) else {}
    response = result if isinstance(result, dict) else {}
    plan = response.get("plan")
    if not isinstance(plan, list):
        plan = request.get("plan")
    if not isinstance(plan, list):
        return []
    lines: list[str] = []
    for index, raw_step in enumerate(plan[:12], start=1):
        if isinstance(raw_step, dict):
            status = str(raw_step.get("status") or "unknown")
            text = raw_step.get("step")
        else:
            status = "unknown"
            text = raw_step
        lines.append(f"PLAN STEP {index} [{status}] {_short(text, trunc)}".rstrip())
    if len(plan) > 12:
        lines.append(f"PLAN STEP … omitted={len(plan) - 12} (limit=12)")
    return lines


def _chain_id_from_response(response: object) -> str | None:
    if not isinstance(response, str):
        return None
    marker = "[sub_agent_session_id:"
    if marker not in response:
        return None
    value = response.rsplit(marker, 1)[1].split("]", 1)[0].strip()
    try:
        return str(UUID(value))
    except ValueError:
        return value or None


def _response_without_chain_marker(response: object) -> str:
    if not isinstance(response, str):
        return ""
    marker = "\n\n[sub_agent_session_id:"
    return response.rsplit(marker, 1)[0].strip() if marker in response else response.strip()


def _worker_output_summary(response: object, trunc: int) -> str | None:
    body = _response_without_chain_marker(response)
    try:
        output = json.loads(body)
    except (TypeError, json.JSONDecodeError):
        return None
    if not isinstance(output, dict) or "business_disposition" not in output:
        return None
    fact_refs = output.get("fact_refs")
    evidence_ids = output.get("evidence_ids")
    checked_empty = output.get("checked_empty_units")
    summary = (
        f"WORKER OUTPUT disposition={output.get('business_disposition')} "
        f"facts={len(fact_refs) if isinstance(fact_refs, list) else '?'} "
        f"evidence={evidence_ids if isinstance(evidence_ids, list) else '?'} "
        f"checked_empty={len(checked_empty) if isinstance(checked_empty, list) else '?'}"
    )
    if output.get("blocker_code"):
        summary += f" blocker={output['blocker_code']}"
    if output.get("summary"):
        summary += f" summary={_short(output['summary'], trunc)}"
    return summary


def _harness_summary(e: dict, trunc: int) -> str | None:
    """Render a `harness_trace` event. Returns None for kinds we don't surface.

    Keyed on the REAL `kind` tag (serde `#[serde(tag = "kind")]` on
    `HarnessTraceKind`): gate_decision / evidence_booked / deliverable_submitted /
    background_notes_injected / stage_run_org_progress. (The pre-fix script looked
    for `gate_pass`/`gate_block`, which never exist, so it silently dropped every
    gate verdict — the single most important debugging signal.)
    """
    kind = e.get("kind")
    if kind == "gate_decision":
        gate = e.get("gate", "?")
        bits = [f"GATE {gate}"]
        if e.get("findings") is not None:
            bits.append(f"findings={e['findings']}")
        if e.get("fabricated_evidence_refs"):
            bits.append(f"fabricated={e['fabricated_evidence_refs']}")
        if e.get("available_real_ids"):
            bits.append(f"available={e['available_real_ids']}")
        if e.get("first_blocking_reason"):
            bits.append(f"reason: {_short(e['first_blocking_reason'], trunc)}")
        return " ".join(bits)
    if kind == "evidence_booked":
        return f"evidence #{e.get('evidence_id')} ({e.get('tool')}, {e.get('source')})"
    if kind == "deliverable_submitted":
        s = f"submit {e.get('status', '?')}"
        if e.get("cited_evidence_refs"):
            s += f" cited={e['cited_evidence_refs']}"
        if e.get("available_real_ids"):
            s += f" available={e['available_real_ids']}"
        return s
    if kind == "background_notes_injected":
        return f"background notes injected x{e.get('count')} {_short(e.get('evidence_ids'), 60)}".rstrip()
    if kind == "stage_refiner_decision":
        bits = [
            f"refiner {e.get('repair_kind', '?')}",
            f"actions={e.get('action_count', 0)}",
            f"gaps={e.get('gap_count', 0)}",
            f"hash={e.get('directive_hash', '')}",
        ]
        if e.get("llm_escalated"):
            bits.append("llm=1")
        if e.get("root_cause"):
            bits.append(f"cause: {_short(e['root_cause'], trunc)}")
        return " ".join(bits)
    if kind == "runtime_supervisor_decision":
        bits = [
            f"runtime_supervisor {e.get('mode', '?')}",
            f"{e.get('tool', '?')}x{e.get('repeat_count', 0)}",
            f"{e.get('strategy_kind', '?')}",
            f"actions={e.get('action_count', 0)}",
            f"injected={str(e.get('injected', False)).lower()}",
            f"hash={e.get('directive_hash', '')}",
        ]
        if e.get("root_cause"):
            bits.append(f"cause: {_short(e['root_cause'], trunc)}")
        return " ".join(bits)
    if kind == "stage_run_org_progress":
        org = e.get("org_name") or (e.get("org_id") or "")[:8]
        ev = e.get("evidence_count")
        head = f"org {org}: {e.get('status', '?')} (evidence x{ev})"
        cov = e.get("coverage") or []
        if cov:
            # cov: list of [technique, "found"|"checked_empty"|"blocked"|"pending"]
            head += "  cov[" + " ".join(f"{t}={st}" for t, st in cov) + "]"
        act = e.get("activity")
        if act:
            head += f"  ~ {_short(act, 60)}"
        return head
    return None


# Sub-agent prose event types now persisted on disk (sub-agent-prose change).
_SUB_PROSE = {
    "sub_agent_reasoning": "\U0001f4ad think",
    "sub_agent_text_delta": "\U0001f4ac say",
    "sub_agent_completed": "\u2713 done",
}


def parse_subagent_dirname(name: str) -> tuple[str, str, str | None]:
    """`<agent_id>-<parent_request_id>::org::<org_id>` -> (agent_id, parent_req, org_id)."""
    org_id = None
    left = name
    if "::org::" in name:
        left, org_id = name.split("::org::", 1)
    if "-" in left:
        agent_id, parent_req = left.split("-", 1)
    else:
        agent_id, parent_req = left, ""
    return agent_id, parent_req, org_id


def _stage_team_parent_prefix(parent_request_id: str) -> str | None:
    """Return the shared team identity for current lead/worker transcript paths."""
    for marker in ("::lead:", "::worker:"):
        if marker in parent_request_id:
            return parent_request_id.split(marker, 1)[0]
    return None


def session_org_ids(session_dir: Path) -> list[str]:
    """Infer organization ids from sub-agent directory names for DB joins."""
    subs_root = session_dir / "subagents"
    if not subs_root.is_dir():
        return []
    orgs: set[str] = set()
    for d in subs_root.iterdir():
        if not d.is_dir():
            continue
        _agent, _parent_req, org_id = parse_subagent_dirname(d.name)
        if org_id:
            orgs.add(org_id)
    return sorted(orgs)


def collect_subagent_calls(events: list[dict], trunc: int) -> list[dict]:
    """Pair sub_agent_tool_request/result by request_id, in order (footer/nesting)."""
    results = {
        e.get("request_id"): e
        for e in events
        if e.get("type") == "sub_agent_tool_result"
    }
    calls = []
    for e in events:
        if e.get("type") != "sub_agent_tool_request":
            continue
        rid = e.get("request_id")
        res = results.get(rid, {})
        calls.append(
            {
                "request_id": rid,
                "tool": e.get("tool_name", "?"),
                "args": e.get("args"),
                "result": res.get("result"),
                "success": res.get("success"),
                "summary": _tool_summary(
                    e.get("tool_name", "?"), e.get("args"), res.get("result"), trunc
                ),
            }
        )
    return calls


def _logfield(line: str, key: str) -> str:
    """Extract a single-token `key=value` field from a tracing log line."""
    if key not in line:
        return "?"
    rest = line.split(key, 1)[1].split()
    return rest[0] if rest else "?"


def coverage_gaps_from_runlog(session_dir: Path, trunc: int) -> list[str]:
    """Surface gate BLOCK signals from this run's `run.log` (per-run folder).

    Two sources, both on disk:
      - `coverage gap matrix (full)` (target `harness::gate::coverage`) — the
        COMPLETE `(asset x technique)` gap set + exact `gaps_total`. Added so a
        stuck stage's full matrix is recoverable (the model-facing reason only
        carries the first 8 cells).
      - `gate_rule block ... first_reason=` (target `harness::gate::rule_engine`)
        — the per-rule block reason. Present in older runs too, so the tool is
        useful even before the full-matrix line exists.
    Identical entries (one per re-submit) are de-duplicated with a x<count> marker.
    """
    rl = session_dir / "run.log"
    if not rl.exists():
        return []
    full: list[tuple[str, str, str]] = []  # (op, gaps_total, gaps)
    blocks: list[tuple[str, str]] = []  # (op, first_reason)
    try:
        for line in rl.open():
            if "coverage gap matrix" in line:
                op = _logfield(line, "op=")
                total = _logfield(line, "gaps_total=")
                gaps = line.split("gaps=", 1)[1].rstrip("\n") if "gaps=" in line else ""
                full.append((op, total, gaps))
            elif "gate_rule block" in line and "first_reason=" in line:
                op = _logfield(line, "op=")
                reason = line.split("first_reason=", 1)[1].rstrip("\n")
                blocks.append((op, reason))
    except OSError:
        return []
    if not full and not blocks:
        return []
    import collections

    out = ["", "== gate BLOCK signals (from run.log) =="]
    for (op, total, gaps), cnt in collections.Counter(full).items():
        rep = f" (x{cnt})" if cnt > 1 else ""
        out.append(f"  [{op}] {total} cells never reached a terminal state{rep}:")
        out.append(f"    {gaps if trunc >= 10_000 else _short(gaps, 240)}")
    if not full:  # only show raw block reasons when the full matrix isn't available
        for (op, reason), cnt in collections.Counter(blocks).items():
            rep = f" (x{cnt})" if cnt > 1 else ""
            out.append(f"  [{op}] block{rep}: {reason if trunc >= 10_000 else _short(reason, 240)}")
    return out


def runlog_anomalies(session_dir: Path, trunc: int) -> list[str]:
    """Summarize runtime anomalies that otherwise hide in the full run.log."""
    rl = session_dir / "run.log"
    if not rl.exists():
        return []
    import collections

    repair_blocks: collections.Counter[tuple[str, str]] = collections.Counter()
    cancelled_tools: collections.Counter[tuple[str, str]] = collections.Counter()
    controller_failures: collections.Counter[str] = collections.Counter()
    controller_failure_signatures = (
        "Company Controller child drain exhausted its frozen lifetime budget",
        "Company Controller is waiting but no runnable child WorkItem remains",
        "Company Controller dispatched no accepted runnable child",
        "Company Controller prepared final submission before its child barrier was ready",
        "Company Controller Gate repair fuel was exhausted",
        "Company Controller exhausted its frozen coordination turn budget",
        "STAGE_TEAM_DISPATCH_NONE_ACCEPTED",
        "STAGE_TEAM_DISPATCH_PERSIST_FAILED",
        "STAGE_TEAM_WORKER_EXECUTION_FAILED",
        "STAGE_TEAM_WORKER_OUTPUT_INVALID",
        "STAGE_TEAM_ACTIVE_TOOL_RECOVERY_BLOCKED",
        "STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED",
    )
    try:
        with rl.open() as fh:
            for line in fh:
                if "sub-agent tool call BLOCKED by submit repair mode" in line:
                    repair_blocks[
                        (_logfield(line, "agent_id="), _logfield(line, "tool="))
                    ] += 1
                elif "cancelled while waiting for tool" in line:
                    agent = "?"
                    if "[sub-agent:" in line:
                        agent = line.split("[sub-agent:", 1)[1].split("]", 1)[0]
                    tool = "?"
                    if "tool '" in line:
                        tool = line.split("tool '", 1)[1].split("'", 1)[0]
                    cancelled_tools[(agent, tool)] += 1
                for signature in controller_failure_signatures:
                    if signature in line:
                        controller_failures[signature] += 1
    except OSError:
        return []
    if not repair_blocks and not cancelled_tools and not controller_failures:
        return []
    out = ["", "== runtime anomalies (from run.log) =="]
    for failure, cnt in controller_failures.most_common():
        out.append(f"  controller {failure}: x{cnt}")
    for (agent, tool), cnt in repair_blocks.most_common():
        out.append(f"  submit_repair blocked {agent}.{tool}: x{cnt}")
    for (agent, tool), cnt in cancelled_tools.most_common():
        out.append(f"  cancelled while waiting for {agent}.{tool}: x{cnt}")
    return out


def runlog_ai_calls(session_dir: Path) -> list[str]:
    """Summarize only the stable model-call signals recorded in run.log."""
    rl = session_dir / "run.log"
    if not rl.exists():
        return []
    import collections
    import re

    main_pattern = re.compile(
        r"\[main-agent\] Turn complete: provider=([^,\s]+), model=([^,\s]+), "
        r"tokens=\{input=(\d+), output=(\d+), total=(\d+)\}"
    )
    subagent_pattern = re.compile(
        r"\[sub-agent:([^\]]+)\] Executing with .*?"
        r"provider=([^,\s]+), model=([^,\s]+)"
    )
    main_turns: collections.Counter[tuple[str, str]] = collections.Counter()
    main_tokens: dict[tuple[str, str], list[int]] = collections.defaultdict(
        lambda: [0, 0, 0]
    )
    subagent_starts: collections.Counter[tuple[str, str, str]] = collections.Counter()
    try:
        with rl.open() as fh:
            for line in fh:
                main_match = main_pattern.search(line)
                if main_match:
                    provider, model = main_match.group(1), main_match.group(2)
                    key = (provider, model)
                    main_turns[key] += 1
                    for index, value in enumerate(main_match.groups()[2:]):
                        main_tokens[key][index] += int(value)
                subagent_match = subagent_pattern.search(line)
                if subagent_match:
                    subagent_starts[subagent_match.groups()] += 1
    except OSError:
        return []
    if not main_turns and not subagent_starts:
        return []

    out = ["", "== AI calls (from run.log) =="]
    if main_turns:
        total_tokens = [
            sum(values[index] for values in main_tokens.values()) for index in range(3)
        ]
        out.append(
            f"  main completed turns={sum(main_turns.values())} "
            f"tokens input={total_tokens[0]} output={total_tokens[1]} total={total_tokens[2]}"
        )
        for (provider, model), turns in sorted(main_turns.items()):
            tokens = main_tokens[(provider, model)]
            out.append(
                f"    provider={provider} model={model} turns={turns} "
                f"tokens input={tokens[0]} output={tokens[1]} total={tokens[2]}"
            )
    if subagent_starts:
        out.append(
            f"  sub-agent model starts={sum(subagent_starts.values())} "
            "(starts only; not tool calls or completed turns; child tokens unavailable)"
        )
        for (agent, provider, model), starts in sorted(subagent_starts.items()):
            out.append(
                f"    agent={agent} provider={provider} model={model} starts={starts}"
            )
    return out


def render_session_tree(session_dir: Path, trunc: int = TRUNC) -> list[str]:
    """Render the transcript-owned Controller -> children -> Gate timeline.

    This intentionally classifies a Company Controller only from current
    runtime identity signals (`::lead:` parent or Controller coordination
    tools). A generic update_plan call is never enough, and an older fixed
    Stage Team worker is labeled legacy rather than presented as a Controller.
    """
    main_events = _load_jsonl(session_dir / "transcript.json")
    subs_root = session_dir / "subagents"
    sub_dirs = (
        sorted((d for d in subs_root.iterdir() if d.is_dir()), key=lambda d: d.name)
        if subs_root.is_dir()
        else []
    )
    parsed_dirs = [(d, *parse_subagent_dirname(d.name)) for d in sub_dirs]
    subs_by_parent: dict[str, list[Path]] = {}
    for d, _agent, parent_req, _org in parsed_dirs:
        subs_by_parent.setdefault(parent_req, []).append(d)
    controller_team_prefixes = {
        prefix
        for _d, _agent, parent_req, _org in parsed_dirs
        if "::lead:" in parent_req
        for prefix in [_stage_team_parent_prefix(parent_req)]
        if prefix is not None
    }
    worker_dirs_by_team: dict[str, list[Path]] = {}
    for d, _agent, parent_req, _org in parsed_dirs:
        prefix = _stage_team_parent_prefix(parent_req)
        if prefix in controller_team_prefixes and "::worker:" in parent_req:
            worker_dirs_by_team.setdefault(prefix, []).append(d)

    def subdirs_for_request(request_id: object, include_team_children: bool = False) -> list[Path]:
        if not isinstance(request_id, str):
            return []
        matches = list(subs_by_parent.get(request_id, []))
        if include_team_children:
            team_matches = [
                d
                for d, _agent, parent_req, _org in parsed_dirs
                if parent_req.startswith(f"{request_id}::team::") and d not in matches
            ]
            current_controller_leads = [
                d
                for d, _agent, parent_req, _org in parsed_dirs
                if d in team_matches and "::lead:" in parent_req
            ]
            # Current runtime workers are siblings of the lead on disk. Render
            # only the lead here; workers belong under its dispatch timeline.
            matches.extend(current_controller_leads or team_matches)
        return sorted(matches, key=lambda d: d.name)

    stages: list[str] = []
    for event in main_events:
        stage = event.get("stage") if event.get("type") == "harness_trace" else None
        if isinstance(stage, str) and stage and stage not in stages:
            stages.append(stage)
    active_stage = stages[-1] if stages else "?"
    stage_detail = f"stage={active_stage}"
    if len(stages) > 1:
        stage_detail += f" (latest; seen={','.join(stages)})"

    lines = [
        f"RUN {session_dir.name}",
        f"{stage_detail}  events(main)={len(main_events)}  sub-agents={len(sub_dirs)}",
        "main agent",
    ]
    rendered_subs: set[str] = set()
    stats = {
        "controllers": 0,
        "resumes": 0,
        "dynamic_dispatches": 0,
        "worker_outputs": 0,
        "submits": 0,
        "needs_fix": 0,
        "gate_pass": 0,
        "gate_block": 0,
        "anomalies": 0,
    }

    def add_anomaly(indent: str, message: str) -> None:
        stats["anomalies"] += 1
        lines.append(f"{indent}⚠ anomaly: {message}")

    def render_sub(d: Path, indent: str, relation: str = "sub-agent") -> None:
        rendered_subs.add(d.name)
        agent_id, parent_req, org_id = parse_subagent_dirname(d.name)
        events = _load_jsonl(d / "transcript.json")
        events.sort(key=lambda event: event.get("_timestamp", ""))
        results = {
            event.get("request_id"): event
            for event in events
            if event.get("type") == "sub_agent_tool_result"
        }
        calls = [event for event in events if event.get("type") == "sub_agent_tool_request"]
        completed = [event for event in events if event.get("type") == "sub_agent_completed"]
        controller_tool_names = {
            event.get("tool_name")
            for event in calls
            if event.get("tool_name")
            in {
                "stage_team_dispatch_workers",
                "stage_team_prepare_final_submission",
            }
        }
        is_controller = bool(
            "::lead:" in parent_req
            or "::company-controller" in parent_req
            or controller_tool_names
        )
        team_prefix = _stage_team_parent_prefix(parent_req)
        is_current_dynamic_worker = bool(
            "::worker:" in parent_req and team_prefix in controller_team_prefixes
        )
        is_legacy_team = bool(
            not is_controller
            and not is_current_dynamic_worker
            and (
                "::team::" in parent_req
                or "::worker:" in parent_req
                or "::aggregator" in parent_req
            )
        )
        chain_sequence = [
            chain
            for chain in (
                _chain_id_from_response(event.get("response")) for event in completed
            )
            if chain
        ]
        unique_chains = list(dict.fromkeys(chain_sequence))
        org_label = org_id or "?"
        if is_controller:
            stats["controllers"] += 1
            turns = len(completed) or 1
            resume_count = max(0, turns - 1) if len(unique_chains) <= 1 else 0
            stats["resumes"] += resume_count
            chain = unique_chains[0] if unique_chains else "?"
            resume = (
                f" resume=same-chain x{resume_count}" if resume_count else " resume=no"
            )
            label = "Company Controller"
            header_tail = f"chain={chain} turns={turns}{resume}"
        elif is_legacy_team:
            label = "legacy Stage Team worker"
            header_tail = "runtime=legacy-fixed (not Company Controller)"
        else:
            label = (
                "dynamic SubAgent"
                if relation == "dynamic child" or is_current_dynamic_worker
                else "sub-agent"
            )
            chain = unique_chains[0] if unique_chains else "?"
            header_tail = f"chain={chain}"
        lines.append(
            f"{indent}└─ {label}: {agent_id}  [org {org_label}]  "
            f"({len(calls)} calls) {header_tail}"
        )
        child_indent = indent + "   "
        if is_controller and len(unique_chains) > 1:
            add_anomaly(
                child_indent,
                f"Controller resumed with divergent chains {unique_chains}",
            )

        completed_index = 0
        for event in events:
            event_type = event.get("type")
            if event_type == "sub_agent_tool_request":
                request_id = event.get("request_id")
                result_event = results.get(request_id, {})
                result = result_event.get("result")
                tool_name = event.get("tool_name", "?")
                lines.append(
                    f"{child_indent}├─ "
                    f"{_tool_summary(tool_name, event.get('args'), result, trunc)}"
                )
                if tool_name == "update_plan":
                    lines.extend(
                        f"{child_indent}│  {step}"
                        for step in _plan_step_summaries(
                            event.get("args"), result, trunc
                        )
                    )
                nested = subdirs_for_request(request_id)
                if is_controller and tool_name == "stage_team_dispatch_workers":
                    if team_prefix is not None:
                        nested.extend(
                            child_dir
                            for child_dir in worker_dirs_by_team.get(team_prefix, [])
                            if child_dir not in nested
                        )
                        nested.sort(key=lambda child_dir: child_dir.name)
                    status = result.get("status") if isinstance(result, dict) else None
                    accepted = (
                        result.get("accepted_count") if isinstance(result, dict) else None
                    )
                    if status == "dispatch_accepted":
                        stats["dynamic_dispatches"] += 1
                        lines.append(
                            f"{child_indent}│  ↳ WAIT Lead parked; scheduler draining accepted children"
                        )
                        if isinstance(accepted, int) and accepted <= 0:
                            add_anomaly(
                                child_indent + "│  ",
                                "dispatch_accepted reported no accepted child",
                            )
                        if isinstance(accepted, int) and accepted > 0 and not nested:
                            add_anomaly(
                                child_indent + "│  ",
                                f"accepted {accepted} child request(s) but no child transcript exists",
                            )
                    elif result_event and not result_event.get("success", False):
                        add_anomaly(
                            child_indent + "│  ",
                            f"dynamic dispatch failed: {_short(result, trunc)}",
                        )
                if tool_name == "submit_stage_deliverable":
                    stats["submits"] += 1
                    if isinstance(result, dict) and result.get("status") == "needs_fix":
                        stats["needs_fix"] += 1
                if result_event and (
                    result_event.get("success") is False
                    or (isinstance(result, dict) and result.get("error"))
                ):
                    add_anomaly(
                        child_indent + "│  ",
                        f"{tool_name} failed: {_short(result, trunc)}",
                    )
                for nested_dir in nested:
                    if nested_dir.name not in rendered_subs:
                        nested_relation = (
                            "dynamic child"
                            if is_controller
                            and tool_name == "stage_team_dispatch_workers"
                            else "sub-agent"
                        )
                        render_sub(nested_dir, child_indent + "   ", nested_relation)
            elif event_type == "sub_agent_completed":
                completed_index += 1
                chain = _chain_id_from_response(event.get("response")) or "?"
                if is_controller:
                    lines.append(
                        f"{child_indent}· TURN {completed_index} complete chain={chain}"
                    )
                    if completed_index < len(completed):
                        lines.append(
                            f"{child_indent}↻ RESUME same Company Controller chain={chain}"
                        )
                else:
                    worker_output = _worker_output_summary(event.get("response"), trunc)
                    if worker_output:
                        stats["worker_outputs"] += 1
                        lines.append(f"{child_indent}· {worker_output}")
                    response = _short(_response_without_chain_marker(event.get("response")), trunc)
                    if response:
                        lines.append(f"{child_indent}· ✓ done: {response}")
            elif event_type == "sub_agent_error":
                add_anomaly(
                    child_indent,
                    f"{agent_id} error: {_short(event.get('error'), trunc)}",
                )
            elif event_type in _SUB_PROSE and event_type != "sub_agent_completed":
                text = (
                    event.get("accumulated")
                    or event.get("response")
                    or event.get("delta")
                    or ""
                )
                text = _short(text, trunc)
                if text:
                    lines.append(f"{child_indent}· {_SUB_PROSE[event_type]}: {text}")

    results_by_request = {
        event.get("request_id"): event
        for event in main_events
        if event.get("type") == "tool_result"
    }
    seen_tool_requests: set[str] = set()
    for event in main_events:
        event_type = event.get("type")
        if event_type in ("tool_auto_approved", "tool_request"):
            request_id = event.get("request_id")
            if request_id in seen_tool_requests:
                continue
            seen_tool_requests.add(request_id)
            tool = event.get("tool_name", "?")
            result = (results_by_request.get(request_id) or {}).get("result")
            lines.append(
                f"   ├─ {_tool_summary(tool, event.get('args'), result, trunc)}"
            )
            if tool == "update_plan":
                lines.extend(
                    f"   │  {step}"
                    for step in _plan_step_summaries(event.get("args"), result, trunc)
                )
            for sub_dir in subdirs_for_request(request_id, include_team_children=True):
                if sub_dir.name not in rendered_subs:
                    render_sub(sub_dir, "   ")
        elif event_type == "harness_trace":
            summary = _harness_summary(event, trunc)
            if summary:
                lines.append(f"   • {summary}")
            if event.get("kind") == "gate_decision":
                gate = str(event.get("gate", "")).upper()
                if gate == "PASS":
                    stats["gate_pass"] += 1
                elif gate == "BLOCK":
                    stats["gate_block"] += 1
        elif event_type == "completed":
            response = _short(event.get("response"), trunc)
            if response:
                lines.append(f"   ⤴ FINAL: {response}")
            reasoning = _short(event.get("reasoning"), trunc)
            if reasoning:
                lines.append(f"      reasoning: {reasoning}")

    for sub_dir in sub_dirs:
        if sub_dir.name not in rendered_subs:
            render_sub(sub_dir, "   ")

    lines.append("")
    lines.append(
        "summary: "
        f"controllers={stats['controllers']} resumes={stats['resumes']} "
        f"dynamic_dispatches={stats['dynamic_dispatches']} "
        f"worker_outputs={stats['worker_outputs']} submits={stats['submits']} "
        f"needs_fix={stats['needs_fix']} gate_pass={stats['gate_pass']} "
        f"gate_block={stats['gate_block']} anomalies={stats['anomalies']}"
    )
    if stats["needs_fix"] >= 3:
        lines.append("  ⚠ repeated identical needs_fix likely — check the Gate reason above")
    return lines


def main() -> int:
    args = sys.argv[1:]
    trunc = TRUNC
    workspace = None
    positional = None
    want_db = False
    db_url = None
    i = 0
    while i < len(args):
        if args[i] == "--full":
            trunc = 10_000
        elif args[i] == "--db":
            want_db = True
        elif args[i] == "--db-url" and i + 1 < len(args):
            db_url = args[i + 1]
            want_db = True
            i += 1
        elif args[i] == "--workspace" and i + 1 < len(args):
            workspace = args[i + 1]
            i += 1
        elif not args[i].startswith("-"):
            positional = args[i]
        i += 1

    session_dir = resolve_session_dir(positional, workspace)
    print("\n".join(render_session_tree(session_dir, trunc)))

    ai_call_lines = runlog_ai_calls(session_dir)
    if ai_call_lines:
        print("\n".join(ai_call_lines))

    gap_lines = coverage_gaps_from_runlog(session_dir, trunc)
    if gap_lines:
        print("\n".join(gap_lines))

    anomaly_lines = runlog_anomalies(session_dir, trunc)
    if anomaly_lines:
        print("\n".join(anomaly_lines))

    if want_db:
        print("\n".join(run_db_diagnosis(session_dir, db_url, trunc)))
    return 0


def run_db_diagnosis(session_dir: Path, db_url: str | None, trunc: int) -> list[str]:
    """Deterministic DB checks for root causes transcripts can't show.

    Each query degrades independently: a missing table/column or a closed DB
    never aborts the report, it just prints what it could read.
    """
    session_id = session_dir.name
    org_ids = session_org_ids(session_dir)
    url = db_url or os.environ.get("GOLISH_DB_URL") or DEFAULT_DB_URL
    out: list[str] = ["", "== DB self-diagnosis =="]
    try:
        import psycopg2  # type: ignore
    except ImportError:
        out.append("  psycopg2 not installed (pip install psycopg2-binary) \u2014 skipped")
        return out
    try:
        conn = psycopg2.connect(url, connect_timeout=4)
    except Exception as exc:  # noqa: BLE001 - report any conn/ auth failure
        out.append(f"  connect failed [{url}]: {_short(str(exc), 120)}")
        out.append("  hint: start Golish (embedded PG) or set --db-url / $GOLISH_DB_URL")
        return out

    cur = conn.cursor()

    def q(sql: str, params: tuple = ()):  # returns rows or None on error
        try:
            cur.execute(sql, params)
            return cur.fetchall()
        except Exception as exc:  # noqa: BLE001 - per-query degrade
            conn.rollback()
            return [("ERR", _short(str(exc), 70))]

    out.append(f"  db={url.rsplit('@', 1)[-1]}  session={session_id}")
    if org_ids:
        out.append(f"  org_ids: {', '.join(org_ids)}")

    out.extend(
        _runtime_memory_lines(
            q,
            session_id=session_id,
            candidate_operation_ids=session_operation_ids(session_dir),
            trunc=trunc,
        )
    )

    session_start = None
    rows = q(
        "SELECT min(created_at), max(created_at), count(*) FROM audit_log WHERE session_id=%s",
        (session_id,),
    )
    if rows and rows[0][0] != "ERR":
        session_start, session_end, row_count = rows[0]
        out.append(f"  audit_log session window: {session_start} .. {session_end} ({row_count} rows)")

    # 1) targets + the organization_id=NULL root cause (gate skips per-org truth).
    rows = q("SELECT count(*), count(*) FILTER (WHERE organization_id IS NULL) FROM targets")
    if rows and rows[0][0] != "ERR":
        total, org_null = rows[0]
        flag = "  \u26a0 org_id=NULL \u2192 gate skips per-org DB-truth projection" if org_null else ""
        out.append(f"  targets: {total} total, {org_null} with organization_id=NULL{flag}")
    else:
        out.append(f"  targets: {rows[0][1] if rows else 'no rows'}")

    # 2) organizations.
    rows = q("SELECT count(*) FROM organizations")
    if rows and rows[0][0] != "ERR":
        out.append(f"  organizations: {rows[0][0]}")

    # 3) target_assets (subdomains/ips/services landed) + by type.
    assets_total = None
    rows = q("SELECT count(*) FROM target_assets")
    if rows and rows[0][0] != "ERR":
        assets_total = rows[0][0]
        by = q("SELECT asset_type, count(*) FROM target_assets GROUP BY 1 ORDER BY 2 DESC")
        bystr = ", ".join(f"{t}={n}" for t, n in by) if by and by[0][0] != "ERR" else ""
        out.append(f"  target_assets: {assets_total}" + (f" ({bystr})" if bystr else ""))

    # 4) dns_records + by type.
    rows = q("SELECT count(*) FROM dns_records")
    if rows and rows[0][0] != "ERR":
        by = q("SELECT record_type, count(*) FROM dns_records GROUP BY 1 ORDER BY 2 DESC")
        bystr = ", ".join(f"{t}={n}" for t, n in by) if by and by[0][0] != "ERR" else ""
        out.append(f"  dns_records: {rows[0][0]}" + (f" ({bystr})" if bystr else ""))

    ledger_rows = q(
        "SELECT tool_name, evidence_technique, evidence_outcome, count(*), "
        "(array_agg(id ORDER BY id DESC))[1:8] "
        "FROM audit_log WHERE audit_role='evidence' AND session_id=%s "
        "GROUP BY 1,2,3 ORDER BY 1,2,3",
        (session_id,),
    )
    if ledger_rows and ledger_rows[0][0] == "ERR":
        out.append(f"  evidence ledger rows: {ledger_rows[0][1]}")
    else:
        out.append("  evidence ledger rows (this session):")
        if not ledger_rows:
            out.append("    (none) ⚠ session has no audit_role=evidence rows")
        for tool, tech, outcome, count, ids in ledger_rows:
            label = tool or "?"
            if tech or outcome:
                label += f" [{tech or '?'} {outcome or '?'}]"
            out.append(f"    {label}: {count} ids={ids}")

    # 5) audit_log evidence facts for THIS session (the gate's coverage truth).
    facts = q(
        "SELECT evidence_technique, evidence_outcome, count(*) FROM audit_log "
        "WHERE audit_role='evidence' AND evidence_technique IS NOT NULL AND session_id=%s "
        "GROUP BY 1,2 ORDER BY 1,2",
        (session_id,),
    )
    scope = "this session"
    if (facts and facts[0][0] == "ERR") or not facts:
        # session had no rows (or session_id column mismatch) -> fall back to global
        gfacts = q(
            "SELECT evidence_technique, evidence_outcome, count(*) FROM audit_log "
            "WHERE audit_role='evidence' AND evidence_technique IS NOT NULL "
            "GROUP BY 1,2 ORDER BY 1,2"
        )
        if not (gfacts and gfacts[0][0] == "ERR"):
            facts, scope = gfacts, "all sessions (session had none)"
    found_subdomain = 0
    if facts and facts[0][0] == "ERR":
        out.append(f"  audit_log evidence facts: {facts[0][1]}")
    else:
        out.append(f"  audit_log evidence facts ({scope}):")
        if not facts:
            out.append("    (none) \u26a0 no evidence rows booked \u2192 gate sees nothing")
        for tech, outcome, n in facts:
            out.append(f"    {tech} {outcome}: {n}")
            if tech == "GOLISH-INTEL-SUBDOMAIN" and outcome == "found":
                found_subdomain += n

    # 6) cross-check: subdomains "found" as evidence but none landed into target_assets.
    if assets_total == 0 and found_subdomain > 0:
        out.append(
            f"  \u26a0 {found_subdomain} SUBDOMAIN 'found' evidence rows but target_assets=0 "
            "\u2192 landing gap (evidence booked, not projected into assets)"
        )

    stage_outcomes = q(
        "SELECT technique, outcome, source, count(*), "
        "(array_agg(asset ORDER BY asset))[1:8], "
        # Aggregate the array column as text: PostgreSQL cannot array_agg a mix
        # of empty and non-empty arrays because their dimensions differ.
        "(array_agg(evidence_ids::text ORDER BY updated_at DESC))[1:5] "
        "FROM technique_outcomes WHERE run_id=%s "
        "GROUP BY 1,2,3 ORDER BY 1,2,3",
        (session_id,),
    )
    enum_outcome_count = 0
    if stage_outcomes and stage_outcomes[0][0] == "ERR":
        out.append(f"  technique_outcomes (this run): {stage_outcomes[0][1]}")
    else:
        out.append("  technique_outcomes (this run):")
        if not stage_outcomes:
            out.append("    (none)")
        for tech, outcome, source, count, assets, evidence_ids in stage_outcomes:
            out.append(
                f"    {tech} {outcome} via {source}: {count} "
                f"assets={assets} evidence_ids={evidence_ids}"
            )
            if isinstance(tech, str) and tech.startswith("GOLISH-ENUM-"):
                enum_outcome_count += int(count)

    enum_ledger_rows = q(
        "SELECT count(*) FROM audit_log WHERE audit_role='evidence' AND session_id=%s "
        "AND (evidence_technique LIKE 'GOLISH-ENUM-%%' "
        "OR tool_name IN ('browser_collect_js_api','js_extract_apis','route_probe_paths'))",
        (session_id,),
    )
    if (
        enum_outcome_count
        and enum_ledger_rows
        and enum_ledger_rows[0][0] != "ERR"
        and int(enum_ledger_rows[0][0]) == 0
    ):
        out.append(
            "  \u26a0 ENUM outcomes exist but this session has no ENUM evidence ledger rows "
            "\u2192 gate/repair may list stale evidence ids only"
        )

    if org_ids and session_start is not None:
        out.append("  fresh content rows since session evidence started:")
        content_queries = [
            (
                "directory_entries",
                "SELECT count(*), min(d.created_at), max(d.created_at) "
                "FROM directory_entries d JOIN targets t ON t.id=d.target_id "
                "WHERE t.organization_id = ANY(%s::uuid[]) AND d.created_at >= %s",
            ),
            (
                "api_endpoints",
                "SELECT count(*), min(a.discovered_at), max(a.discovered_at) "
                "FROM api_endpoints a JOIN targets t ON t.id=a.target_id "
                "WHERE t.organization_id = ANY(%s::uuid[]) AND a.discovered_at >= %s",
            ),
            (
                "js_analysis_results",
                "SELECT count(*), min(j.analyzed_at), max(j.analyzed_at) "
                "FROM js_analysis_results j JOIN targets t ON t.id=j.target_id "
                "WHERE t.organization_id = ANY(%s::uuid[]) AND j.analyzed_at >= %s",
            ),
        ]
        for name, sql in content_queries:
            rows = q(sql, (org_ids, session_start))
            if rows and rows[0][0] == "ERR":
                out.append(f"    {name}: {rows[0][1]}")
            elif rows:
                count, first_seen, last_seen = rows[0]
                out.append(f"    {name}: {count} rows ({first_seen} .. {last_seen})")

        top_dirs = q(
            "SELECT t.value, count(*), "
            "count(*) FILTER (WHERE d.status_code BETWEEN 200 AND 399), "
            "count(*) FILTER (WHERE d.status_code >= 400) "
            "FROM directory_entries d JOIN targets t ON t.id=d.target_id "
            "WHERE t.organization_id = ANY(%s::uuid[]) AND d.created_at >= %s "
            "GROUP BY t.value ORDER BY count(*) DESC LIMIT 8",
            (org_ids, session_start),
        )
        if top_dirs and top_dirs[0][0] != "ERR":
            out.append("  directory_entries top targets:")
            for value, total, ok, err in top_dirs:
                out.append(f"    {value}: {total} rows, ok={ok}, >=400={err}")

        top_js = q(
            "SELECT t.value, count(*), "
            "count(*) FILTER (WHERE coalesce(jsonb_array_length(j.endpoints_found),0)>0), "
            "sum(j.size_bytes) "
            "FROM js_analysis_results j JOIN targets t ON t.id=j.target_id "
            "WHERE t.organization_id = ANY(%s::uuid[]) AND j.analyzed_at >= %s "
            "GROUP BY t.value ORDER BY count(*) DESC LIMIT 8",
            (org_ids, session_start),
        )
        if top_js and top_js[0][0] != "ERR":
            out.append("  js_analysis_results top targets:")
            for value, total, with_endpoints, bytes_total in top_js:
                out.append(
                    f"    {value}: {total} files, "
                    f"with_endpoints={with_endpoints}, bytes={bytes_total}"
                )

    # 7) source_query_log (#5): per-source passive-intel query log for THIS run.
    #    Proves which data sources were queried (CT / WHOIS / OSINT / code platforms)
    #    and with what result — finer-grained than the asset x technique coverage
    #    matrix (a technique covered by several sources shows one row per source).
    #    Degrades if the table is absent (write path is gray-switched, default off).
    sqlog = q(
        "SELECT technique, source, status, count(*) FROM source_query_log "
        "WHERE run_id=%s GROUP BY 1,2,3 ORDER BY 1,2,3",
        (session_id,),
    )
    if sqlog and sqlog[0][0] == "ERR":
        out.append(f"  source_query_log: {sqlog[0][1]}")
    elif not sqlog:
        out.append(
            "  source_query_log: (none for this run; write path off "
            "[GOLISH_SOURCE_QUERY_LOG_WRITE] or no source queries)"
        )
    else:
        out.append("  source_query_log (this run, per source):")
        for tech, source, status, n in sqlog:
            out.append(f"    {tech or '(unmapped)'} via {source} \u2192 {status}: {n}")

    # 8) expansion_queue (#6): discovered leads pending recursive expansion for
    #    THIS run. Proves whether high-confidence leads (subsidiaries / new domains
    #    / github orgs ...) were followed up. Degrades if the table is absent (write
    #    path is gray-switched, default off). Gate does NOT block on it (model A).
    eq = q(
        "SELECT lead_type, status, count(*), "
        "count(*) FILTER (WHERE confidence >= 0.8) "
        "FROM expansion_queue WHERE run_id=%s GROUP BY 1,2 ORDER BY 1,2",
        (session_id,),
    )
    if eq and eq[0][0] == "ERR":
        out.append(f"  expansion_queue: {eq[0][1]}")
    elif not eq:
        out.append(
            "  expansion_queue: (none for this run; no subsidiary leads "
            "discovered, or migration not applied yet)"
        )
    else:
        out.append("  expansion_queue (this run, discovered leads):")
        for lead_type, status, n, hi in eq:
            flag = (
                f"  \u26a0 {hi} high-confidence pending \u2192 follow up"
                if status == "pending" and hi
                else ""
            )
            out.append(f"    {lead_type} [{status}]: {n}{flag}")

    conn.close()
    return out


def session_operation_ids(session_dir: Path) -> list[str]:
    """Collect trusted-looking top-level operation/task UUIDs from transcripts.

    DB discovery still joins the audit/session tables. These candidates cover
    runs that failed before their first audit row without recursively trusting
    model-authored tool arguments.
    """
    ids: set[str] = set()
    transcript_paths = [session_dir / "transcript.json"]
    subs_root = session_dir / "subagents"
    if subs_root.is_dir():
        transcript_paths.extend(subs_root.glob("*/transcript.json"))
    for path in transcript_paths:
        for event in _load_jsonl(path):
            sources = [event]
            detail = event.get("detail")
            if event.get("type") == "harness_trace" and isinstance(detail, dict):
                sources.append(detail)
            for source in sources:
                for key in ("operation_id", "harness_operation_id", "task_id"):
                    value = source.get(key)
                    if not isinstance(value, str):
                        continue
                    try:
                        ids.add(str(UUID(value)))
                    except ValueError:
                        continue
    return sorted(ids)


def _runtime_records(rows: list[tuple] | None) -> tuple[list[dict], str | None]:
    if rows and rows[0] and rows[0][0] == "ERR":
        return [], str(rows[0][1]) if len(rows[0]) > 1 else "query failed"
    records: list[dict] = []
    for row in rows or []:
        if row and isinstance(row[0], dict):
            records.append(row[0])
    return records, None


def _compact_json(value: object, trunc: int) -> str:
    try:
        rendered = json.dumps(value, sort_keys=True, ensure_ascii=False, default=str)
    except (TypeError, ValueError):
        rendered = str(value)
    return _short(rendered, trunc)


def _yes_no(value: object) -> str:
    return "yes" if bool(value) else "no"


def _stage_team_tree_lines(
    plans: list[dict],
    items: list[dict],
    dependencies: list[dict],
    workers: list[dict],
    outputs: list[dict],
    requests: list[dict],
    trunc: int,
) -> list[str]:
    """Render the durable Unit -> TeamPlan -> WorkItem ownership tree.

    Inputs are already exact-operation DB projections. Raw checkpoint bodies,
    lease tokens and arbitrary canonical output JSON never enter this helper.
    """
    if not plans:
        return ["      stage_teams: (none; legacy/team-disabled execution)"]
    out = ["      stage_teams:"]
    dependencies_by_item: dict[str, list[str]] = {}
    for dependency in dependencies:
        item_id = str(dependency.get("work_item_id"))
        dependencies_by_item.setdefault(item_id, []).append(
            str(dependency.get("depends_on_work_item_id"))
        )
    workers_by_item: dict[str, list[dict]] = {}
    for worker in workers:
        work_item_id = worker.get("work_item_id")
        if work_item_id is not None:
            workers_by_item.setdefault(str(work_item_id), []).append(worker)
    outputs_by_item = {
        str(output.get("work_item_id")): output for output in outputs
    }

    for plan in plans:
        plan_id = str(plan.get("id"))
        plan_items = [item for item in items if str(item.get("team_plan_id")) == plan_id]
        plan_requests = [
            request for request in requests if str(request.get("team_plan_id")) == plan_id
        ]
        aggregator_role = plan.get("aggregator_role")
        controller_item = next(
            (
                item
                for item in plan_items
                if item.get("stable_key") == "leader:primary"
                and item.get("role") == plan.get("leader_role")
                and not item.get("required_for_barrier")
            ),
            None,
        )
        is_company_controller = controller_item is not None

        def is_legacy_aggregator(item: dict) -> bool:
            return bool(
                not is_company_controller
                and plan.get("aggregator_kind") == "worker"
                and aggregator_role == item.get("role")
                and not item.get("required_for_barrier")
            )

        def is_coordinator(item: dict) -> bool:
            return item is controller_item or is_legacy_aggregator(item)

        child_items = [item for item in plan_items if not is_coordinator(item)]
        child_ids = {str(item.get("id")) for item in child_items}
        child_workers = [
            worker
            for worker in workers
            if str(worker.get("work_item_id")) in child_ids
        ]
        terminal_count = sum(item.get("status") == "completed" for item in child_items)
        live_count = sum(
            worker.get("status") in {"queued", "running", "waiting_background"}
            for worker in child_workers
        )
        retry_count = sum(item.get("status") == "retry_pending" for item in child_items)
        recovery_count = sum(
            worker.get("status") == "recovery_required" for worker in child_workers
        )
        missing_outputs = sum(
            item.get("status") == "completed"
            and str(item.get("id")) not in outputs_by_item
            for item in child_items
        )
        requests_closed = plan.get("requests_closed_at") is not None
        barrier_ready = bool(
            requests_closed
            and terminal_count == len(child_items)
            and live_count == 0
            and retry_count == 0
            and recovery_count == 0
            and missing_outputs == 0
        )
        out.append(
            f"        unit={plan.get('stage_run_unit_id')} org={plan.get('organization_id')} "
            f"plan={plan_id} stage={plan.get('stage_kind')} v={plan.get('plan_version')} "
            f"hash={plan.get('plan_hash')}"
        )
        if is_company_controller:
            out.append(
                "          mode=company_controller "
                f"controller_item={controller_item.get('id')} "
                f"controller_role={plan.get('leader_role')}"
            )
        else:
            out.append("          mode=legacy_fixed_team (not Company Controller)")
        out.append(
            f"          roles leader={plan.get('leader_role')} "
            f"aggregator={aggregator_role or plan.get('aggregator_kind')} "
            f"allowed={_compact_json(plan.get('allowed_worker_roles'), trunc)}"
        )
        out.append(
            f"          concurrency active={plan.get('max_workers_active')} "
            f"total={plan.get('max_workers_total')} dynamic={_yes_no(plan.get('dynamic_requests_allowed'))} "
            f"epoch={plan.get('dispatch_epoch')} closed_at={plan.get('requests_closed_at')}"
        )
        barrier_subject = "children " if is_company_controller else ""
        out.append(
            f"          barrier ready={_yes_no(barrier_ready)} "
            f"{barrier_subject}terminal={terminal_count}/{len(child_items)} live={live_count} "
            f"retry={retry_count} recovery={recovery_count} missing_outputs={missing_outputs}"
        )
        if plan.get("final_submitter_worker_run_id") is not None:
            out.append(
                "          final_submitter="
                f"{plan.get('final_submitter_kind')}:{plan.get('final_submitter_worker_run_id')}"
            )
        for request in plan_requests:
            out.append(
                f"          request id={request.get('id')} "
                f"parent={request.get('parent_work_item_id')}/{request.get('parent_worker_run_id')} "
                f"role={request.get('requested_role')} kind={request.get('request_kind')} "
                f"subjects={request.get('subject_ref_count')} status={request.get('status')}"
            )
            out.append(
                f"            reason={request.get('reason_code')} "
                f"decision={request.get('decision_reason_code')} "
                f"accepted_item={request.get('accepted_work_item_id')} "
                f"hash={request.get('request_payload_hash')}"
            )
        for item in plan_items:
            item_id = str(item.get("id"))
            out.append(
                f"          work_item id={item_id} "
                f"kind={item.get('kind')} key={item.get('stable_key')} "
                f"role={item.get('role')} status={item.get('status')} "
                f"barrier={_yes_no(item.get('required_for_barrier'))} "
                f"controller={_yes_no(item is controller_item)} "
                f"legacy_aggregator={_yes_no(is_legacy_aggregator(item))}"
            )
            out.append(
                f"            priority={item.get('priority')} created_by={item.get('created_by')} "
                f"subjects={item.get('subject_ref_count')} schema={item.get('output_schema')} "
                f"input_hash={item.get('input_manifest_hash')} "
                f"dependencies={dependencies_by_item.get(item_id, [])}"
            )
            for worker in workers_by_item.get(item_id, []):
                out.append(
                    f"            worker id={worker.get('id')} generation={worker.get('worker_generation')} "
                    f"status={worker.get('status')} chain={worker.get('message_chain_id')} "
                    f"epoch={worker.get('attempt_epoch')} lease={_yes_no(worker.get('lease_present'))} "
                    f"expired={_yes_no(worker.get('lease_expired'))}"
                )
                if worker.get("status") == "recovery_required" or (
                    worker.get("lease_expired") and worker.get("active_tool_call_id") is not None
                ):
                    recovery = "manual_required"
                elif worker.get("lease_expired"):
                    recovery = "requeue_eligible"
                elif worker.get("lease_present"):
                    recovery = "wait_for_live_lease"
                elif worker.get("status") in {"passed", "failed", "exhausted", "superseded"}:
                    recovery = "terminal"
                else:
                    recovery = "unleased"
                out.append(
                    f"              specialist={worker.get('specialist')} "
                    f"active_tool={_yes_no(worker.get('active_tool_call_id'))} "
                    f"evidence_watermark={worker.get('evidence_watermark')} recovery={recovery}"
                )
            output = outputs_by_item.get(item_id)
            if output is not None:
                out.append(
                    f"            output id={output.get('id')} worker={output.get('worker_run_id')} "
                    f"disposition={output.get('business_disposition')} "
                    f"facts={output.get('canonical_fact_ref_count')} "
                    f"evidence={output.get('evidence_ids')} "
                    f"checked_empty={output.get('checked_empty_cell_count')} "
                    f"blockers={output.get('blocker_codes')} hash={output.get('output_hash')}"
                )
    return out


def _has_legacy_checkpoint(state_blob: object) -> bool:
    if not isinstance(state_blob, dict):
        return False
    graph_flow = state_blob.get("graph_flow")
    if isinstance(graph_flow, dict):
        state = graph_flow.get("state")
        next_node = graph_flow.get("next_node")
        if (
            isinstance(state, dict)
            and isinstance(state.get("seeded"), dict)
            and isinstance(state.get("visited"), list)
            and isinstance(state.get("applied"), dict)
            and isinstance(next_node, str)
            and bool(next_node.strip())
        ):
            return True
    run_id = state_blob.get("current_stage_run_id")
    try:
        valid_run_id = isinstance(run_id, str) and UUID(run_id).int != 0
    except ValueError:
        valid_run_id = False
    return bool(
        isinstance(state_blob.get("profile"), str)
        and state_blob["profile"].strip()
        and isinstance(state_blob.get("current_stage"), str)
        and state_blob["current_stage"].strip()
        and valid_run_id
        and isinstance(state_blob.get("queue_titles"), list)
        and isinstance(state_blob.get("completed_count"), int)
        and state_blob["completed_count"] >= 0
    )


def _runtime_memory_lines(
    q,
    session_id: str,
    candidate_operation_ids: list[str],
    trunc: int,
) -> list[str]:
    """Render Runtime Memory V2 state using an injected query function.

    The injection seam keeps the diagnostic deterministic and fully testable
    without PostgreSQL. Every SQL statement has a stable marker used only by
    offline fixtures; production still executes ordinary read-only SELECTs.
    """
    out = ["  runtime memory V2:"]

    rollout_rows = q(
        """/* run_tree:runtime_rollout */
        SELECT jsonb_build_object(
            'contract', contract,
            'contract_rank', contract_rank,
            'row_version', row_version,
            'updated_at', updated_at
        )
        FROM runtime_memory_rollout
        WHERE singleton_id = 1"""
    )
    rollout, rollout_error = _runtime_records(rollout_rows)
    if rollout_error:
        out.append(f"    rollout: {rollout_error}")
    elif rollout:
        record = rollout[0]
        out.append(
            "    rollout: "
            f"contract={record.get('contract')} rank={record.get('contract_rank')} "
            f"row_version={record.get('row_version')} updated_at={record.get('updated_at')}"
        )
    else:
        out.append("    rollout: (missing singleton) ⚠ anomaly: runtime rollout is undefined")

    attack_rollout_rows = q(
        """/* run_tree:attack_rollout */
        SELECT jsonb_build_object(
            'contract', contract,
            'rank', rank,
            'row_version', row_version,
            'updated_at', updated_at
        )
        FROM attack_execution_rollout
        WHERE singleton = TRUE"""
    )
    attack_rollout, attack_rollout_error = _runtime_records(attack_rollout_rows)
    if attack_rollout_error:
        out.append(f"    attack_rollout: {attack_rollout_error}")
    elif attack_rollout:
        record = attack_rollout[0]
        out.append(
            "    attack_rollout: "
            f"contract={record.get('contract')} rank={record.get('rank')} "
            f"row_version={record.get('row_version')} updated_at={record.get('updated_at')}"
        )
    else:
        out.append(
            "    attack_rollout: (missing singleton) "
            "⚠ anomaly: attack rollout is undefined"
        )

    operation_rows = q(
        """/* run_tree:runtime_operations */
        SELECT jsonb_build_object(
            'operation_id', os.operation_id,
            'runtime_memory_contract', os.runtime_memory_contract,
            'attack_execution_contract', os.attack_execution_contract,
            'task_status', (
                SELECT task.status::text
                FROM tasks AS task
                WHERE task.id = os.operation_id
            ),
            'profile', os.profile,
            'current_stage', os.current_stage,
            'project_scope_id', os.project_scope_id,
            'engagement_org_id', os.engagement_org_id,
            'superseded_by', os.superseded_by,
            'stage_started_at', os.stage_started_at,
            'state_blob', os.state_blob
        )
        FROM operation_state AS os
        WHERE os.operation_id = ANY(%s::uuid[])
           OR EXISTS (
                SELECT 1 FROM audit_log AS audit
                WHERE audit.session_id = %s AND audit.run_id = os.operation_id
           )
           OR EXISTS (
                SELECT 1
                FROM tasks AS task
                JOIN sessions AS session ON session.id = task.session_id
                WHERE task.id = os.operation_id
                  AND session.chat_session_key = %s
           )
        ORDER BY os.stage_started_at DESC, os.operation_id""",
        (candidate_operation_ids, session_id, session_id),
    )
    operations, operations_error = _runtime_records(operation_rows)
    if operations_error:
        out.append(f"    operations: {operations_error}")
        return out
    if not operations:
        out.append("    operations: (none for this session)")
        return out

    for operation in operations:
        operation_id = str(operation.get("operation_id"))
        contract = str(operation.get("runtime_memory_contract") or "unknown")
        current_stage = str(operation.get("current_stage") or "unknown")
        state_blob = operation.get("state_blob")
        legacy_present = _has_legacy_checkpoint(state_blob)
        out.append(
            f"    operation {operation_id}: contract={contract} "
            f"profile={operation.get('profile')} current_stage={current_stage}"
        )
        out.append(
            f"      attack_contract={operation.get('attack_execution_contract')} "
            f"project_scope={operation.get('project_scope_id')} "
            f"engagement_org={operation.get('engagement_org_id')} "
            f"stage_started_at={operation.get('stage_started_at')} "
            f"superseded_by={operation.get('superseded_by')}"
        )
        out.append(
            "      legacy_checkpoint_present=" + ("yes" if legacy_present else "no")
        )
        if operation.get("superseded_by") is not None:
            out.append(
                f"      ⚠ anomaly: operation is superseded by {operation.get('superseded_by')}"
            )

        def fetch_records(label: str, sql: str) -> list[dict]:
            records, error = _runtime_records(q(sql, (operation_id,)))
            if error:
                out.append(f"      {label}: {error}")
            return records

        attack_waves = fetch_records(
            "attack_waves",
            """/* run_tree:attack_waves */
            SELECT jsonb_build_object(
                'id', wave.id,
                'scope_snapshot_id', wave.scope_snapshot_id,
                'generation', wave.generation,
                'status', wave.status,
                'policy_hash', wave.policy_hash,
                'max_waves', wave.max_waves,
                'max_candidates_total', wave.max_candidates_total,
                'max_chain_depth', wave.max_chain_depth,
                'max_attempts_total', wave.max_attempts_total,
                'row_version', wave.row_version,
                'created_at', wave.created_at,
                'updated_at', wave.updated_at,
                'terminal_at', wave.terminal_at
            )
            FROM attack_wave_runs AS wave
            WHERE wave.operation_id = %s
            ORDER BY wave.generation, wave.id""",
        )
        out.append(f"      attack_waves: {len(attack_waves)}")
        for wave in attack_waves:
            out.append(
                f"        wave id={wave.get('id')} "
                f"snapshot={wave.get('scope_snapshot_id')} "
                f"generation={wave.get('generation')} status={wave.get('status')} "
                f"policy_hash={wave.get('policy_hash')}"
            )
            out.append(
                f"          caps waves={wave.get('max_waves')} "
                f"candidates={wave.get('max_candidates_total')} "
                f"depth={wave.get('max_chain_depth')} "
                f"attempts={wave.get('max_attempts_total')} "
                f"row_version={wave.get('row_version')}"
            )
            out.append(
                f"          created_at={wave.get('created_at')} "
                f"updated_at={wave.get('updated_at')} "
                f"terminal_at={wave.get('terminal_at')}"
            )

        wave_unit_counts = fetch_records(
            "attack_wave_unit_counts",
            """/* run_tree:attack_wave_unit_counts */
            SELECT jsonb_build_object(
                'wave_run_id', unit.wave_run_id,
                'total', COUNT(*),
                'open_count', COUNT(*) FILTER (WHERE unit.status = 'open'),
                'reasoning_count', COUNT(*) FILTER (WHERE unit.status = 'reasoning'),
                'review_count', COUNT(*) FILTER (WHERE unit.status = 'review'),
                'verification_count', COUNT(*) FILTER (WHERE unit.status = 'verification'),
                'terminal_count', COUNT(*) FILTER (WHERE unit.status = 'terminal'),
                'review_closed_count', COUNT(*) FILTER (WHERE unit.review_closed),
                'verification_closed_count', COUNT(*) FILTER (WHERE unit.verification_closed),
                'consolidation_pending_count', COUNT(*) FILTER (
                    WHERE unit.consolidation_status = 'pending'
                ),
                'consolidation_ready_count', COUNT(*) FILTER (
                    WHERE unit.consolidation_status = 'ready'
                ),
                'consolidation_consumed_count', COUNT(*) FILTER (
                    WHERE unit.consolidation_status = 'consumed'
                ),
                'consolidation_terminal_count', COUNT(*) FILTER (
                    WHERE unit.consolidation_status = 'terminal'
                )
            )
            FROM attack_wave_units AS unit
            WHERE unit.operation_id = %s
            GROUP BY unit.wave_run_id
            ORDER BY unit.wave_run_id""",
        )
        out.append("      attack_wave_unit_counts:")
        if not wave_unit_counts:
            out.append("        (none)")
        for counts in wave_unit_counts:
            out.append(
                f"        wave={counts.get('wave_run_id')} total={counts.get('total')} "
                f"status open={counts.get('open_count')} "
                f"reasoning={counts.get('reasoning_count')} "
                f"review={counts.get('review_count')} "
                f"verification={counts.get('verification_count')} "
                f"terminal={counts.get('terminal_count')}"
            )
            out.append(
                f"          closed review={counts.get('review_closed_count')} "
                f"verification={counts.get('verification_closed_count')} "
                f"consolidation pending={counts.get('consolidation_pending_count')} "
                f"ready={counts.get('consolidation_ready_count')} "
                f"consumed={counts.get('consolidation_consumed_count')} "
                f"terminal={counts.get('consolidation_terminal_count')}"
            )

        candidate_attempts = fetch_records(
            "candidate_attempt_ownership",
            """/* run_tree:candidate_attempt_ownership */
            SELECT jsonb_build_object(
                'id', attempt.id,
                'candidate_id', attempt.candidate_id,
                'wave_run_id', attempt.wave_run_id,
                'wave_unit_id', attempt.wave_unit_id,
                'organization_id', attempt.organization_id,
                'ordinal', attempt.ordinal,
                'status', attempt.status,
                'row_version', attempt.row_version,
                'terminal_at', attempt.terminal_at,
                'stage_worker_run_id', attempt.stage_worker_run_id,
                'worker_status', worker.status,
                'worker_generation', worker.worker_generation,
                'specialist', worker.specialist,
                'attempt_epoch', worker.attempt_epoch,
                'checkpoint_version', worker.checkpoint_version,
                'checkpoint_present', worker.checkpoint IS NOT NULL,
                'checkpoint_bytes', CASE
                    WHEN worker.checkpoint IS NULL THEN 0
                    ELSE OCTET_LENGTH(worker.checkpoint::text)
                END,
                'lease_present', worker.lease_token IS NOT NULL,
                'lease_owner', worker.lease_owner,
                'lease_expires_at', worker.lease_expires_at,
                'lease_expired', worker.lease_expires_at IS NOT NULL
                    AND worker.lease_expires_at <= NOW(),
                'active_tool_call_id', worker.active_tool_call_id,
                'active_tool_name', tool.name,
                'active_tool_status', tool.status,
                'ownership_matches', worker.id IS NULL OR (
                    worker.operation_id = attempt.operation_id
                    AND worker.organization_id = attempt.organization_id
                    AND worker.work_item_kind = 'candidate_attempt'
                    AND worker.work_item_key = attempt.id::text
                    AND worker.specialist = 'candidate_verifier'
                    AND unit.id IS NOT NULL
                    AND unit.operation_id = attempt.operation_id
                    AND unit.scope_snapshot_id = attempt.scope_snapshot_id
                    AND unit.organization_id = attempt.organization_id
                    AND unit.stage_kind = 'verification'
                )
            )
            FROM candidate_attempts AS attempt
            LEFT JOIN stage_worker_runs AS worker
              ON worker.id = attempt.stage_worker_run_id
            LEFT JOIN stage_run_units AS unit
              ON unit.id = worker.stage_run_unit_id
            LEFT JOIN tool_calls AS tool
              ON tool.id = worker.active_tool_call_id
            WHERE attempt.operation_id = %s
            ORDER BY attempt.created_at, attempt.id""",
        )
        out.append(f"      candidate_attempt_ownership: {len(candidate_attempts)}")
        for attempt in candidate_attempts:
            out.append(
                f"        attempt={attempt.get('id')} "
                f"candidate={attempt.get('candidate_id')} "
                f"wave={attempt.get('wave_run_id')} unit={attempt.get('wave_unit_id')} "
                f"org={attempt.get('organization_id')} ordinal={attempt.get('ordinal')} "
                f"status={attempt.get('status')} row_version={attempt.get('row_version')} "
                f"terminal_at={attempt.get('terminal_at')}"
            )
            out.append(
                f"          worker={attempt.get('stage_worker_run_id')} "
                f"status={attempt.get('worker_status')} "
                f"generation={attempt.get('worker_generation')} "
                f"specialist={attempt.get('specialist')} "
                f"epoch={attempt.get('attempt_epoch')}"
            )
            out.append(
                f"          checkpoint_version={attempt.get('checkpoint_version')} "
                f"checkpoint_present={_yes_no(attempt.get('checkpoint_present'))} "
                f"checkpoint_bytes={attempt.get('checkpoint_bytes')} "
                f"lease_present={_yes_no(attempt.get('lease_present'))} "
                f"lease_owner={attempt.get('lease_owner')} "
                f"expires={attempt.get('lease_expires_at')} "
                f"expired={_yes_no(attempt.get('lease_expired'))}"
            )
            if attempt.get("active_tool_call_id") is not None:
                out.append(
                    f"          active_tool={attempt.get('active_tool_call_id')} "
                    f"name={attempt.get('active_tool_name')} "
                    f"status={attempt.get('active_tool_status')}"
                )
            if not attempt.get("ownership_matches"):
                out.append(
                    f"      ⚠ cross-org rejection: candidate attempt={attempt.get('id')} "
                    "does not match its exact verification worker"
                )

        attack_lanes = fetch_records(
            "attack_lane",
            """/* run_tree:attack_lane */
            SELECT jsonb_build_object(
                'lane_key', lane.lane_key,
                'stage_worker_run_id', lane.stage_worker_run_id,
                'lease_present', lane.lease_token IS NOT NULL,
                'lease_owner', lane.lease_owner,
                'lease_expires_at', lane.lease_expires_at,
                'lease_expired', lane.lease_expires_at IS NOT NULL
                    AND lane.lease_expires_at <= NOW(),
                'updated_at', lane.updated_at
            )
            FROM attack_execution_lanes AS lane
            WHERE lane.stage_worker_run_id IS NULL
               OR EXISTS (
                    SELECT 1
                    FROM stage_worker_runs AS worker
                    WHERE worker.id = lane.stage_worker_run_id
                      AND worker.operation_id = %s
               )
            ORDER BY lane.lane_key""",
        )
        out.append(f"      attack_lane: {len(attack_lanes)}")
        for lane in attack_lanes:
            out.append(
                f"        lane={lane.get('lane_key')} "
                f"worker={lane.get('stage_worker_run_id')} "
                f"lease_present={_yes_no(lane.get('lease_present'))} "
                f"owner={lane.get('lease_owner')} "
                f"expires={lane.get('lease_expires_at')} "
                f"expired={_yes_no(lane.get('lease_expired'))} "
                f"updated_at={lane.get('updated_at')}"
            )

        fact_deltas = fetch_records(
            "attack_fact_deltas",
            """/* run_tree:attack_fact_deltas */
            SELECT jsonb_build_object(
                'id', delta.id,
                'source_attempt_id', delta.source_attempt_id,
                'candidate_id', delta.candidate_id,
                'wave_run_id', delta.wave_run_id,
                'wave_unit_id', delta.wave_unit_id,
                'organization_id', delta.organization_id,
                'canonical_ref_kind', delta.canonical_ref_kind,
                'canonical_ref_id', delta.canonical_ref_id,
                'canonical_ref_version', delta.canonical_ref_version,
                'canonical_ref_hash', delta.canonical_ref_hash,
                'delta_kind', delta.delta_kind,
                'dedupe_hash', delta.dedupe_hash,
                'status', delta.status,
                'consumed_by_wave_run_id', delta.consumed_by_wave_run_id,
                'evidence_count', (
                    SELECT COUNT(*)
                    FROM attack_fact_delta_evidence AS evidence
                    WHERE evidence.fact_delta_id = delta.id
                ),
                'created_at', delta.created_at,
                'consumed_at', delta.consumed_at
            )
            FROM attack_fact_deltas AS delta
            WHERE delta.operation_id = %s
            ORDER BY delta.created_at, delta.id""",
        )
        out.append(f"      attack_fact_deltas: {len(fact_deltas)}")
        for delta in fact_deltas:
            out.append(
                f"        delta={delta.get('id')} attempt={delta.get('source_attempt_id')} "
                f"candidate={delta.get('candidate_id')} wave={delta.get('wave_run_id')} "
                f"unit={delta.get('wave_unit_id')} org={delta.get('organization_id')} "
                f"kind={delta.get('delta_kind')}"
            )
            out.append(
                f"          canonical={delta.get('canonical_ref_kind')}:"
                f"{delta.get('canonical_ref_id')}@{delta.get('canonical_ref_version')} "
                f"hash={delta.get('canonical_ref_hash')} dedupe={delta.get('dedupe_hash')}"
            )
            out.append(
                f"          status={delta.get('status')} "
                f"consumer_wave={delta.get('consumed_by_wave_run_id')} "
                f"evidence_count={delta.get('evidence_count')} "
                f"created_at={delta.get('created_at')} "
                f"consumed_at={delta.get('consumed_at')}"
            )

        residual_risks = fetch_records(
            "attack_residual_risks",
            """/* run_tree:attack_residual_risks */
            SELECT jsonb_build_object(
                'id', risk.id,
                'wave_run_id', risk.wave_run_id,
                'wave_unit_id', risk.wave_unit_id,
                'organization_id', risk.organization_id,
                'reason_code', risk.reason_code,
                'policy_hash', risk.policy_hash,
                'wave_count', risk.wave_count,
                'candidate_count', risk.candidate_count,
                'chain_depth', risk.chain_depth,
                'attempt_count', risk.attempt_count,
                'disclosure_status', risk.disclosure_status,
                'evidence_count', (
                    SELECT COUNT(*)
                    FROM attack_residual_risk_evidence AS evidence
                    WHERE evidence.residual_risk_id = risk.id
                ),
                'created_at', risk.created_at,
                'disclosed_at', risk.disclosed_at
            )
            FROM attack_residual_risks AS risk
            WHERE risk.operation_id = %s
            ORDER BY risk.created_at, risk.id""",
        )
        out.append(f"      attack_residual_risks: {len(residual_risks)}")
        for risk in residual_risks:
            out.append(
                f"        residual={risk.get('id')} wave={risk.get('wave_run_id')} "
                f"unit={risk.get('wave_unit_id')} org={risk.get('organization_id')} "
                f"reason={risk.get('reason_code')} policy_hash={risk.get('policy_hash')}"
            )
            out.append(
                f"          counters waves={risk.get('wave_count')} "
                f"candidates={risk.get('candidate_count')} "
                f"depth={risk.get('chain_depth')} attempts={risk.get('attempt_count')} "
                f"disclosure={risk.get('disclosure_status')} "
                f"evidence_count={risk.get('evidence_count')}"
            )
            out.append(
                f"          created_at={risk.get('created_at')} "
                f"disclosed_at={risk.get('disclosed_at')}"
            )

        stage_executions = fetch_records(
            "stage_executions",
            """/* run_tree:stage_executions */
            SELECT jsonb_build_object(
                'id', run.id,
                'stage_kind', run.stage_kind,
                'status', run.status,
                'started_at', run.started_at,
                'completed_at', run.completed_at
            )
            FROM stage_runs AS run
            WHERE run.operation_id = %s
            ORDER BY run.started_at, run.id""",
        )
        active_executions = [
            record for record in stage_executions if record.get("status") == "started"
        ]
        terminal_execution = None
        if not active_executions and operation.get("task_status") == "finished":
            terminal_candidates = [
                record
                for record in stage_executions
                if record.get("status") == "completed"
                and record.get("completed_at") is not None
                and record.get("stage_kind") == current_stage
            ]
            if terminal_candidates:
                terminal_execution = max(
                    terminal_candidates,
                    key=lambda record: (
                        str(record.get("completed_at")),
                        str(record.get("id")),
                    ),
                )
        execution_summary = f"      stage_executions: exact_active={len(active_executions)}"
        if terminal_execution is not None:
            execution_summary += f" terminal_selected={terminal_execution.get('id')}"
        out.append(execution_summary)
        for execution in stage_executions:
            out.append(
                f"        id={execution.get('id')} stage={execution.get('stage_kind')} "
                f"status={execution.get('status')} started_at={execution.get('started_at')} "
                f"completed_at={execution.get('completed_at')}"
            )
        if not active_executions and terminal_execution is None:
            out.append("      ⚠ anomaly: missing active stage execution")
        elif len(active_executions) > 1:
            out.append(
                "      ⚠ anomaly: multiple active stage executions "
                + ", ".join(str(row.get("id")) for row in active_executions)
            )
        elif (
            len(active_executions) == 1
            and active_executions[0].get("stage_kind") != current_stage
        ):
            out.append(
                "      ⚠ anomaly: operation cursor does not match exact active stage execution"
            )
        selected_execution = (
            active_executions[0]
            if len(active_executions) == 1
            else terminal_execution
        )
        selected_execution_id = (
            str(selected_execution.get("id")) if selected_execution is not None else None
        )

        decisions = fetch_records(
            "scope_decisions",
            """/* run_tree:scope_decisions */
            SELECT jsonb_build_object(
                'id', decision.id,
                'stage_execution_id', decision.stage_execution_id,
                'root_organization_id', decision.root_organization_id,
                'mode', decision.mode,
                'decision_hash', decision.decision_hash,
                'choice_tool_call_id', decision.choice_tool_call_id,
                'proposal_tool_call_id', decision.proposal_tool_call_id,
                'review_tool_call_id', decision.review_tool_call_id
            )
            FROM operation_scope_decisions AS decision
            WHERE decision.operation_id = %s
            ORDER BY decision.created_at, decision.id""",
        )
        if not decisions:
            out.append("      scope_decisions: (none)")
        for decision in decisions:
            out.append(
                f"      scope_decision id={decision.get('id')} "
                f"stage_execution={decision.get('stage_execution_id')} "
                f"root_org={decision.get('root_organization_id')} "
                f"mode={decision.get('mode')} hash={decision.get('decision_hash')}"
            )
            out.append(
                f"        choice={decision.get('choice_tool_call_id')} "
                f"proposal={decision.get('proposal_tool_call_id')} "
                f"review={decision.get('review_tool_call_id')}"
            )

        snapshots = fetch_records(
            "scope_snapshots",
            """/* run_tree:scope_snapshots */
            SELECT jsonb_build_object(
                'id', snapshot.id,
                'scope_decision_id', snapshot.scope_decision_id,
                'project_scope_id', snapshot.project_scope_id,
                'project_path_at_freeze', snapshot.project_path_at_freeze,
                'root_organization_id', snapshot.root_organization_id,
                'mode', snapshot.mode,
                'scope_hash', snapshot.scope_hash,
                'schema_version', snapshot.schema_version,
                'frozen_at', snapshot.frozen_at,
                'sealed_at', snapshot.sealed_at
            )
            FROM operation_org_scope_snapshots AS snapshot
            WHERE snapshot.operation_id = %s
            ORDER BY snapshot.frozen_at, snapshot.id""",
        )
        if not snapshots:
            out.append("      scope_snapshots: (none)")
        for snapshot in snapshots:
            out.append(
                f"      scope_snapshot id={snapshot.get('id')} "
                f"decision={snapshot.get('scope_decision_id')} "
                f"project_scope={snapshot.get('project_scope_id')} "
                f"root_org={snapshot.get('root_organization_id')} "
                f"path={snapshot.get('project_path_at_freeze')}"
            )
            out.append(
                f"        mode={snapshot.get('mode')} hash={snapshot.get('scope_hash')} "
                f"schema={snapshot.get('schema_version')} "
                f"sealed_at={snapshot.get('sealed_at')} frozen_at={snapshot.get('frozen_at')}"
            )

        scope_units = fetch_records(
            "scope_units",
            """/* run_tree:scope_units */
            SELECT jsonb_build_object(
                'snapshot_id', unit.snapshot_id,
                'organization_id', unit.organization_id,
                'parent_organization_id', unit.parent_organization_id,
                'organization_name_at_freeze', unit.organization_name_at_freeze,
                'role', unit.role,
                'depth', unit.depth,
                'ordinal', unit.ordinal,
                'ownership_percent', unit.ownership_percent,
                'decision_row_id', unit.decision_row_id,
                'approval_source', unit.approval_source
            )
            FROM operation_org_scope_units AS unit
            JOIN operation_org_scope_snapshots AS snapshot ON snapshot.id = unit.snapshot_id
            WHERE snapshot.operation_id = %s
            ORDER BY unit.snapshot_id, unit.ordinal, unit.organization_id""",
        )
        if not scope_units:
            out.append("      scope_units: (none)")
        for unit in scope_units:
            out.append(
                f"      scope_unit org={unit.get('organization_id')} "
                f"parent={unit.get('parent_organization_id')} role={unit.get('role')} "
                f"depth={unit.get('depth')} ordinal={unit.get('ordinal')} "
                f"ownership={unit.get('ownership_percent')} name={unit.get('organization_name_at_freeze')}"
            )
            out.append(
                f"        snapshot={unit.get('snapshot_id')} "
                f"decision_row={unit.get('decision_row_id')} "
                f"approval_source={_compact_json(unit.get('approval_source'), trunc)}"
            )

        stage_units = fetch_records(
            "stage_units",
            """/* run_tree:stage_units */
            SELECT jsonb_build_object(
                'id', unit.id,
                'stage_execution_id', unit.stage_execution_id,
                'scope_snapshot_id', unit.scope_snapshot_id,
                'organization_id', unit.organization_id,
                'stage_kind', unit.stage_kind,
                'generation', unit.generation,
                'specialist', unit.specialist,
                'status', unit.status,
                'gate_attempt', unit.gate_attempt,
                'row_version', unit.row_version,
                'started_at', unit.started_at,
                'terminal_at', unit.terminal_at,
                'scope_member', EXISTS (
                    SELECT 1 FROM operation_org_scope_units AS member
                    WHERE member.snapshot_id = unit.scope_snapshot_id
                      AND member.organization_id = unit.organization_id
                )
            )
            FROM stage_run_units AS unit
            WHERE unit.operation_id = %s
            ORDER BY unit.updated_at, unit.id""",
        )
        if not stage_units:
            out.append("      stage_units: (none)")
        for unit in stage_units:
            out.append(
                f"      stage_unit id={unit.get('id')} execution={unit.get('stage_execution_id')} "
                f"snapshot={unit.get('scope_snapshot_id')} org={unit.get('organization_id')}"
            )
            out.append(
                f"        stage={unit.get('stage_kind')} generation={unit.get('generation')} "
                f"specialist={unit.get('specialist')} status={unit.get('status')} "
                f"gate_attempt={unit.get('gate_attempt')} row_version={unit.get('row_version')}"
            )
            if not unit.get("scope_member"):
                out.append(
                    f"      ⚠ cross-org rejection: stage_unit={unit.get('id')} "
                    f"org={unit.get('organization_id')} is not in snapshot={unit.get('scope_snapshot_id')}"
                )
            if (
                selected_execution_id
                and unit.get("status") in {"queued", "running", "gate_blocked"}
                and str(unit.get("stage_execution_id")) != selected_execution_id
            ):
                out.append(
                    f"      ⚠ anomaly: nonterminal stage unit {unit.get('id')} "
                    "does not belong to the exact active execution"
                )

        team_plans = fetch_records(
            "stage_team_plans",
            """/* run_tree:stage_team_plans */
            SELECT jsonb_build_object(
                'id', plan.id,
                'stage_execution_id', plan.stage_execution_id,
                'stage_run_unit_id', plan.stage_run_unit_id,
                'organization_id', plan.organization_id,
                'stage_kind', plan.stage_kind,
                'schema_version', plan.schema_version,
                'plan_version', plan.plan_version,
                'plan_hash', plan.plan_hash,
                'leader_role', plan.leader_role,
                'aggregator_kind', plan.aggregator_kind,
                'aggregator_role', plan.aggregator_role,
                'allowed_worker_roles', plan.allowed_worker_roles,
                'max_workers_total', plan.max_workers_total,
                'max_workers_active', plan.max_workers_active,
                'dynamic_requests_allowed', plan.dynamic_requests_allowed,
                'dispatch_epoch', plan.dispatch_epoch,
                'requests_closed_at', plan.requests_closed_at,
                'final_submitter_kind', plan.final_submitter_kind,
                'final_submitter_worker_run_id', plan.final_submitter_worker_run_id
            )
            FROM stage_team_plans AS plan
            WHERE plan.operation_id = %s
            ORDER BY plan.stage_execution_id,plan.organization_id,plan.id""",
        )
        team_items: list[dict] = []
        team_dependencies: list[dict] = []
        team_outputs: list[dict] = []
        team_requests: list[dict] = []
        if team_plans:
            team_items = fetch_records(
                "stage_team_work_items",
                """/* run_tree:stage_team_work_items */
                SELECT jsonb_build_object(
                    'id', item.id,
                    'team_plan_id', item.team_plan_id,
                    'kind', item.kind,
                    'stable_key', item.stable_key,
                    'role', item.role,
                    'input_manifest_hash', item.input_manifest_hash,
                    'subject_ref_count', jsonb_array_length(item.input_refs),
                    'required_for_barrier', item.required_for_barrier,
                    'conflict_key', item.conflict_key,
                    'priority', item.priority,
                    'status', item.status,
                    'output_schema', item.output_schema,
                    'created_by', item.created_by,
                    'started_at', item.started_at,
                    'terminal_at', item.terminal_at
                )
                FROM stage_work_items AS item
                WHERE item.operation_id = %s
                ORDER BY item.team_plan_id,item.priority,item.id""",
            )
            team_dependencies = fetch_records(
                "stage_team_dependencies",
                """/* run_tree:stage_team_dependencies */
                SELECT jsonb_build_object(
                    'team_plan_id', dependency.team_plan_id,
                    'work_item_id', dependency.work_item_id,
                    'depends_on_work_item_id', dependency.depends_on_work_item_id
                )
                FROM stage_work_item_dependencies AS dependency
                WHERE dependency.operation_id = %s
                ORDER BY dependency.work_item_id,dependency.depends_on_work_item_id""",
            )
            team_outputs = fetch_records(
                "stage_team_outputs",
                """/* run_tree:stage_team_outputs */
                SELECT jsonb_build_object(
                    'id', output.id,
                    'team_plan_id', output.team_plan_id,
                    'work_item_id', output.work_item_id,
                    'worker_run_id', output.worker_run_id,
                    'business_disposition', output.business_disposition,
                    'canonical_fact_ref_count', jsonb_array_length(output.canonical_fact_refs),
                    'evidence_ids', output.evidence_ids,
                    'checked_empty_cell_count', jsonb_array_length(output.checked_empty_cells),
                    'blocker_codes', output.blocker_codes,
                    'output_hash', output.output_hash,
                    'created_at', output.created_at
                )
                FROM stage_worker_outputs AS output
                WHERE output.operation_id = %s
                ORDER BY output.team_plan_id,output.created_at,output.id""",
            )
            team_requests = fetch_records(
                "stage_team_requests",
                """/* run_tree:stage_team_requests */
                SELECT jsonb_build_object(
                    'id', request.id,
                    'team_plan_id', request.team_plan_id,
                    'parent_work_item_id', request.parent_work_item_id,
                    'parent_worker_run_id', request.parent_worker_run_id,
                    'dispatch_epoch', request.dispatch_epoch,
                    'requested_role', request.requested_role,
                    'request_kind', request.request_kind,
                    'subject_ref_count', jsonb_array_length(request.bounded_subject_refs),
                    'reason_code', request.reason_code,
                    'request_payload_hash', request.request_payload_hash,
                    'status', request.status,
                    'decision_reason_code', request.decision_reason_code,
                    'accepted_work_item_id', request.accepted_work_item_id,
                    'created_at', request.created_at
                )
                FROM stage_worker_requests AS request
                WHERE request.operation_id = %s
                ORDER BY request.team_plan_id,request.created_at,request.id""",
            )

        workers = fetch_records(
            "stage_workers",
            """/* run_tree:stage_workers */
            SELECT jsonb_build_object(
                'id', worker.id,
                'stage_execution_id', worker.stage_execution_id,
                'stage_run_unit_id', worker.stage_run_unit_id,
                'work_item_id', worker.work_item_id,
                'organization_id', worker.organization_id,
                'worker_generation', worker.worker_generation,
                'specialist', worker.specialist,
                'work_item_kind', worker.work_item_kind,
                'work_item_key', worker.work_item_key,
                'agent_path', worker.agent_path,
                'parent_request_id', worker.parent_request_id,
                'message_chain_id', worker.message_chain_id,
                'status', worker.status,
                'gate_attempt', worker.gate_attempt,
                'checkpoint_version', worker.checkpoint_version,
                'checkpoint_present', worker.checkpoint IS NOT NULL,
                'checkpoint_bytes', CASE
                    WHEN worker.checkpoint IS NULL THEN 0
                    ELSE OCTET_LENGTH(worker.checkpoint::text)
                END,
                'lease_present', worker.lease_token IS NOT NULL,
                'lease_owner', worker.lease_owner,
                'lease_acquired_at', worker.lease_acquired_at,
                'lease_expires_at', worker.lease_expires_at,
                'heartbeat_at', worker.heartbeat_at,
                'attempt_epoch', worker.attempt_epoch,
                'active_tool_call_id', worker.active_tool_call_id,
                'active_tool_started_at', worker.active_tool_started_at,
                'active_tool_name', tool.name,
                'active_tool_status', tool.status,
                'active_tool_request_id', tool.call_id,
                'lease_expired', worker.lease_expires_at IS NOT NULL
                    AND worker.lease_expires_at <= NOW(),
                'unit_identity_matches', unit.id IS NOT NULL
                    AND unit.operation_id = worker.operation_id
                    AND unit.stage_execution_id = worker.stage_execution_id
                    AND unit.organization_id = worker.organization_id,
                'scope_member', member.organization_id IS NOT NULL
            )
            FROM stage_worker_runs AS worker
            LEFT JOIN stage_run_units AS unit ON unit.id = worker.stage_run_unit_id
            LEFT JOIN operation_org_scope_units AS member
              ON member.snapshot_id = unit.scope_snapshot_id
             AND member.organization_id = worker.organization_id
            LEFT JOIN tool_calls AS tool ON tool.id = worker.active_tool_call_id
            WHERE worker.operation_id = %s
            ORDER BY worker.updated_at, worker.id""",
        )
        if not workers:
            out.append("      stage_workers: (none)")
        for worker in workers:
            lease_present = worker.get("lease_present")
            if lease_present is None:
                lease_present = worker.get("lease_token") is not None
            checkpoint_present = worker.get("checkpoint_present")
            if checkpoint_present is None:
                checkpoint_present = worker.get("checkpoint") is not None
            out.append(
                f"      worker id={worker.get('id')} unit={worker.get('stage_run_unit_id')} "
                f"execution={worker.get('stage_execution_id')} org={worker.get('organization_id')} "
                f"generation={worker.get('worker_generation')}"
            )
            out.append(
                f"        specialist={worker.get('specialist')} "
                f"work_item={worker.get('work_item_kind')}:{worker.get('work_item_key')} "
                f"status={worker.get('status')} gate_attempt={worker.get('gate_attempt')} "
                f"agent_path={worker.get('agent_path')}"
            )
            out.append(
                f"        lease present={_yes_no(lease_present)} "
                f"owner={worker.get('lease_owner')} "
                f"epoch={worker.get('attempt_epoch')} expires={worker.get('lease_expires_at')} "
                f"expired={_yes_no(worker.get('lease_expired'))} "
                f"heartbeat={worker.get('heartbeat_at')}"
            )
            if worker.get("active_tool_call_id") is not None:
                out.append(
                    f"        active_tool id={worker.get('active_tool_call_id')} "
                    f"request={worker.get('active_tool_request_id')} "
                    f"name={worker.get('active_tool_name')} status={worker.get('active_tool_status')} "
                    f"started_at={worker.get('active_tool_started_at')}"
                )
                if worker.get("active_tool_name") is None or worker.get("active_tool_status") is None:
                    out.append(
                        f"      ⚠ anomaly: active tool row {worker.get('active_tool_call_id')} is missing"
                    )
                elif worker.get("active_tool_status") not in {"received", "running"}:
                    out.append(
                        f"      ⚠ anomaly: active tool {worker.get('active_tool_call_id')} "
                        f"has terminal status {worker.get('active_tool_status')}"
                    )
            out.append(
                f"        chain={worker.get('message_chain_id')} "
                f"checkpoint_version={worker.get('checkpoint_version')} "
                f"checkpoint_present={_yes_no(checkpoint_present)} "
                f"checkpoint_bytes={worker.get('checkpoint_bytes')} "
                f"parent_request={worker.get('parent_request_id')}"
            )
            if worker.get("status") == "recovery_required" or (
                worker.get("lease_expired") and worker.get("active_tool_call_id") is not None
            ):
                recovery = "manual_required"
            elif worker.get("lease_expired"):
                recovery = "requeue_eligible"
            elif lease_present:
                recovery = "wait_for_live_lease"
            elif worker.get("status") in {"passed", "failed", "exhausted", "superseded"}:
                recovery = "terminal"
            else:
                recovery = "unleased"
            out.append(f"        recovery={recovery}")
            if not worker.get("unit_identity_matches") or not worker.get("scope_member"):
                out.append(
                    f"      ⚠ cross-org rejection: worker={worker.get('id')} "
                    f"org={worker.get('organization_id')} does not match its stage unit/scope"
                )
            if (
                worker.get("lease_expired")
                and worker.get("active_tool_call_id") is not None
                and worker.get("status") != "recovery_required"
            ):
                out.append(
                    "      ⚠ anomaly: expired lease with active tool is not recovery_required"
                )

        out.extend(
            _stage_team_tree_lines(
                team_plans,
                team_items,
                team_dependencies,
                workers,
                team_outputs,
                team_requests,
                trunc,
            )
        )

        submissions = fetch_records(
            "stage_submissions",
            """/* run_tree:stage_submissions */
            SELECT jsonb_build_object(
                'id', submission.id,
                'stage_execution_id', submission.stage_execution_id,
                'stage_run_unit_id', submission.stage_run_unit_id,
                'worker_run_id', submission.worker_run_id,
                'organization_id', submission.organization_id,
                'tool_call_record_id', submission.tool_call_record_id,
                'tool_request_id', submission.tool_request_id,
                'stage_kind', submission.stage_kind,
                'attempt_epoch', submission.attempt_epoch,
                'payload_sha256', submission.payload_sha256,
                'submitted_at', submission.submitted_at,
                'scope_member', submission.organization_id IS NULL OR EXISTS (
                    SELECT 1
                    FROM stage_run_units AS unit
                    JOIN operation_org_scope_units AS member
                      ON member.snapshot_id = unit.scope_snapshot_id
                     AND member.organization_id = submission.organization_id
                    WHERE unit.id = submission.stage_run_unit_id
                )
            )
            FROM stage_deliverable_submissions AS submission
            WHERE submission.operation_id = %s
            ORDER BY submission.submitted_at, submission.id""",
        )
        if not submissions:
            out.append("      stage_submissions: (none)")
        for submission in submissions:
            out.append(
                f"      submission id={submission.get('id')} "
                f"execution={submission.get('stage_execution_id')} "
                f"unit={submission.get('stage_run_unit_id')} "
                f"worker={submission.get('worker_run_id')} org={submission.get('organization_id')}"
            )
            out.append(
                f"        tool={submission.get('tool_call_record_id')}/"
                f"{submission.get('tool_request_id')} stage={submission.get('stage_kind')} "
                f"epoch={submission.get('attempt_epoch')} "
                f"payload_sha256={submission.get('payload_sha256')} "
                f"submitted_at={submission.get('submitted_at')}"
            )
            if not submission.get("scope_member"):
                out.append(
                    f"      ⚠ cross-org rejection: submission={submission.get('id')} "
                    f"org={submission.get('organization_id')} is outside frozen scope"
                )

        handoffs = fetch_records(
            "stage_handoffs",
            """/* run_tree:stage_handoffs */
            SELECT jsonb_build_object(
                'id', handoff.id,
                'organization_id', handoff.organization_id,
                'scope_snapshot_id', handoff.scope_snapshot_id,
                'from_stage_kind', handoff.from_stage_kind,
                'stage_execution_id', handoff.stage_execution_id,
                'source_stage_run_unit_id', handoff.source_stage_run_unit_id,
                'deliverable_submission_id', handoff.deliverable_submission_id,
                'scope_hash', handoff.scope_hash,
                'payload_sha256', handoff.payload_sha256,
                'evidence_ids', handoff.evidence_ids,
                'unit_gate_decision_hash', handoff.unit_gate_decision_hash,
                'aggregate_pass_token_hash', handoff.aggregate_pass_token_hash,
                'gate_passed_at', handoff.gate_passed_at,
                'invalidated_at', handoff.invalidated_at,
                'scope_member', member.organization_id IS NOT NULL
            )
            FROM stage_handoffs AS handoff
            LEFT JOIN operation_org_scope_units AS member
              ON member.snapshot_id = handoff.scope_snapshot_id
             AND member.organization_id = handoff.organization_id
            WHERE handoff.operation_id = %s
            ORDER BY handoff.gate_passed_at, handoff.id""",
        )
        if not handoffs:
            out.append("      stage_handoffs: (none)")
        for handoff in handoffs:
            out.append(
                f"      handoff id={handoff.get('id')} org={handoff.get('organization_id')} "
                f"from_stage={handoff.get('from_stage_kind')} "
                f"execution={handoff.get('stage_execution_id')} "
                f"unit={handoff.get('source_stage_run_unit_id')} "
                f"submission={handoff.get('deliverable_submission_id')}"
            )
            out.append(
                f"        scope_hash={handoff.get('scope_hash')} "
                f"payload_sha256={handoff.get('payload_sha256')} "
                f"evidence_ids={handoff.get('evidence_ids')} "
                f"invalidated_at={handoff.get('invalidated_at')}"
            )
            out.append(
                f"        unit_gate_hash={handoff.get('unit_gate_decision_hash')} "
                f"aggregate_pass_hash={handoff.get('aggregate_pass_token_hash')} "
                f"gate_passed_at={handoff.get('gate_passed_at')}"
            )
            if not handoff.get("scope_member"):
                out.append(
                    f"      ⚠ cross-org rejection: handoff={handoff.get('id')} "
                    f"org={handoff.get('organization_id')} is outside frozen scope"
                )

        snapshot_sealed = bool(snapshots) and all(
            snapshot.get("sealed_at") is not None for snapshot in snapshots
        )
        current_units = [
            unit
            for unit in stage_units
            if selected_execution_id
            and str(unit.get("stage_execution_id")) == selected_execution_id
            and bool(unit.get("scope_member"))
        ]
        v2_complete = selected_execution is not None and (
            current_stage == "scoping" or (snapshot_sealed and bool(current_units))
        )
        if contract == "legacy_v1":
            selected_source, fallback = "legacy", "disabled"
        elif contract == "dual_write_legacy_read":
            selected_source, fallback = "legacy", "not_applicable"
        elif contract == "dual_write_v2_preferred":
            if v2_complete:
                selected_source, fallback = "v2", "not_used"
            elif legacy_present:
                selected_source, fallback = "legacy_fallback", "used"
            else:
                selected_source, fallback = "unavailable", "attempted_but_missing"
        elif contract == "v2_only":
            selected_source, fallback = ("v2" if v2_complete else "unavailable"), "forbidden"
        else:
            selected_source, fallback = "unavailable", "unknown_contract"
        out.append(
            f"      selected_read_source={selected_source} legacy_fallback={fallback}"
        )
        if contract == "v2_only" and not v2_complete:
            out.append("      ⚠ anomaly: v2_only operation has incomplete runtime state")

    return out


if __name__ == "__main__":
    raise SystemExit(main())
