#!/usr/bin/env python3
"""Reconstruct one AI run as a readable call tree (+ optional DB self-diagnosis).

Why: a run's data is spread across `transcript.json` (main agent),
`subagents/<agent>-<parent_req>::org::<org>/transcript.json` (each sub-agent),
the per-run `run.log`, and the DB. Reading the sub->sub call chain by hand means
correlating `parent_request_id` across flat directories. This script stitches it
into ONE indented tree so a human (or a later AI) can see, at a glance:

  main agent -> tool calls -> stage_run fan-out -> sub-agent(recon) -> its
  reasoning + tool calls (subfinder/dig/enrich/...) -> nested sub-agents ->
  submit_stage_deliverable -> the gate verdict (PASS/BLOCK + blocking reason).

What it surfaces from `transcript.json` (all already on disk):
  - gate decisions      (`harness_trace` kind=gate_decision: gate + first_blocking_reason)
  - per-org coverage    (kind=stage_run_org_progress: per-technique found/empty/blocked)
  - evidence booked     (kind=evidence_booked), deliverable submits, background notes
  - main agent's FINAL  (`completed`: response + reasoning summary)
  - sub-agent prose      (`sub_agent_reasoning` / `sub_agent_text_delta` /
                          `sub_agent_completed` — the "why", persisted since
                          the 2026-06-16 sub-agent-prose change)

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
    detail = ""
    if tool == "pentest_run":
        cmd = (a.get("tool_name", "") + " " + a.get("args", "")).strip()
        detail = cmd or _short(a, trunc)
    elif tool in ("run_pty_cmd", "run_command"):
        detail = _short(a.get("command", a), trunc)
    elif tool == "submit_stage_deliverable":
        st = (result or {}).get("status") if isinstance(result, dict) else None
        reasons = (result or {}).get("reasons") if isinstance(result, dict) else None
        if isinstance(reasons, list) and reasons:
            detail = f"{st}: {_short(reasons[0], trunc)}"
        else:
            detail = st or _short(a, trunc)
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
    try:
        for line in rl.open():
            if "sub-agent tool call BLOCKED by submit repair mode" in line:
                repair_blocks[(_logfield(line, "agent_id="), _logfield(line, "tool="))] += 1
            elif "cancelled while waiting for tool" in line:
                agent = "?"
                if "[sub-agent:" in line:
                    agent = line.split("[sub-agent:", 1)[1].split("]", 1)[0]
                tool = "?"
                if "tool '" in line:
                    tool = line.split("tool '", 1)[1].split("'", 1)[0]
                cancelled_tools[(agent, tool)] += 1
    except OSError:
        return []
    if not repair_blocks and not cancelled_tools:
        return []
    out = ["", "== runtime anomalies (from run.log) =="]
    for (agent, tool), cnt in repair_blocks.most_common():
        out.append(f"  submit_repair blocked {agent}.{tool}: x{cnt}")
    for (agent, tool), cnt in cancelled_tools.most_common():
        out.append(f"  cancelled while waiting for {agent}.{tool}: x{cnt}")
    return out


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
    main_events = _load_jsonl(session_dir / "transcript.json")

    # Index sub-agent dirs by their parent_request_id (for nesting).
    sub_dirs = []
    subs_root = session_dir / "subagents"
    if subs_root.is_dir():
        sub_dirs = [d for d in subs_root.iterdir() if d.is_dir()]
    subs_by_parent: dict[str, list[Path]] = {}
    for d in sub_dirs:
        _agent, parent_req, _org = parse_subagent_dirname(d.name)
        subs_by_parent.setdefault(parent_req, []).append(d)

    lines: list[str] = []
    # session header
    stage = next(
        (e.get("stage") for e in main_events if e.get("type") == "harness_trace" and e.get("stage")),
        "?",
    )
    lines.append(f"RUN {session_dir.name}")
    lines.append(f"stage={stage}  events(main)={len(main_events)}  sub-agents={len(sub_dirs)}")
    lines.append("main agent")

    rendered_subs: set[str] = set()

    def render_sub(d: Path, indent: str) -> None:
        rendered_subs.add(d.name)
        agent_id, parent_req, org_id = parse_subagent_dirname(d.name)
        ev = _load_jsonl(d / "transcript.json")
        # Stable order: by timestamp when present (prose appends are spawned, so
        # file order can race the tool appends; timestamps are authoritative).
        ev.sort(key=lambda e: e.get("_timestamp", ""))
        results = {
            e.get("request_id"): e for e in ev if e.get("type") == "sub_agent_tool_result"
        }
        ncalls = sum(1 for e in ev if e.get("type") == "sub_agent_tool_request")
        org8 = (org_id or "")[:8]
        lines.append(f"{indent}\u2514\u2500 sub-agent: {agent_id}  [org {org8}]  ({ncalls} calls)")
        child = indent + "   "
        for e in ev:
            t = e.get("type")
            if t == "sub_agent_tool_request":
                rid = e.get("request_id")
                res = results.get(rid, {})
                summary = _tool_summary(
                    e.get("tool_name", "?"), e.get("args"), res.get("result"), trunc
                )
                lines.append(f"{child}\u251c\u2500 {summary}")
                for nd in subs_by_parent.get(rid, []):
                    if nd.name not in rendered_subs:
                        render_sub(nd, child + "   ")
            elif t in _SUB_PROSE:
                text = e.get("accumulated") or e.get("response") or e.get("delta") or ""
                text = _short(text, trunc)
                if text:
                    lines.append(f"{child}\u00b7 {_SUB_PROSE[t]}: {text}")

    # main agent tool calls + harness decisions + final response, in file order.
    results_by_rid = {
        e.get("request_id"): e for e in main_events if e.get("type") == "tool_result"
    }
    seen_tool_rids: set[str] = set()
    for e in main_events:
        t = e.get("type")
        if t in ("tool_auto_approved", "tool_request"):
            rid = e.get("request_id")
            if rid in seen_tool_rids:
                continue
            seen_tool_rids.add(rid)
            tool = e.get("tool_name", "?")
            result = (results_by_rid.get(rid) or {}).get("result")
            lines.append(f"   \u251c\u2500 {_tool_summary(tool, e.get('args'), result, trunc)}")
            for nd in subs_by_parent.get(rid, []):
                if nd.name not in rendered_subs:
                    render_sub(nd, "   ")
        elif t == "harness_trace":
            summary = _harness_summary(e, trunc)
            if summary:
                lines.append(f"   \u2022 {summary}")
        elif t == "completed":
            resp = _short(e.get("response"), trunc)
            if resp:
                lines.append(f"   \u2934 FINAL: {resp}")
            reasoning = _short(e.get("reasoning"), trunc)
            if reasoning:
                lines.append(f"      reasoning: {reasoning}")

    # Orphan sub-agents (parent_request_id not matched to any rendered call).
    for d in sub_dirs:
        if d.name not in rendered_subs:
            render_sub(d, "   ")

    # footer summary (quick problem-finding)
    submits = 0
    needs_fix = 0
    for d in sub_dirs:
        for c in collect_subagent_calls(_load_jsonl(d / "transcript.json"), trunc):
            if c["tool"] == "submit_stage_deliverable":
                submits += 1
                if isinstance(c["result"], dict) and c["result"].get("status") == "needs_fix":
                    needs_fix += 1
    lines.append("")
    lines.append(f"summary: submits={submits} needs_fix={needs_fix}")
    if needs_fix >= 3:
        lines.append("  \u26a0 repeated identical needs_fix likely \u2014 check the gate reason above")

    print("\n".join(lines))

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

    operation_rows = q(
        """/* run_tree:runtime_operations */
        SELECT jsonb_build_object(
            'operation_id', os.operation_id,
            'runtime_memory_contract', os.runtime_memory_contract,
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
                SELECT 1 FROM tasks AS task
                WHERE task.id = os.operation_id AND task.session_id::text = %s
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
            f"      project_scope={operation.get('project_scope_id')} "
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
        out.append(f"      stage_executions: exact_active={len(active_executions)}")
        for execution in stage_executions:
            out.append(
                f"        id={execution.get('id')} stage={execution.get('stage_kind')} "
                f"status={execution.get('status')} started_at={execution.get('started_at')} "
                f"completed_at={execution.get('completed_at')}"
            )
        if not active_executions:
            out.append("      ⚠ anomaly: missing active stage execution")
        elif len(active_executions) > 1:
            out.append(
                "      ⚠ anomaly: multiple active stage executions "
                + ", ".join(str(row.get("id")) for row in active_executions)
            )
        elif active_executions[0].get("stage_kind") != current_stage:
            out.append(
                "      ⚠ anomaly: operation cursor does not match exact active stage execution"
            )
        active_execution_id = (
            str(active_executions[0].get("id")) if len(active_executions) == 1 else None
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
                active_execution_id
                and unit.get("status") in {"queued", "running", "gate_blocked"}
                and str(unit.get("stage_execution_id")) != active_execution_id
            ):
                out.append(
                    f"      ⚠ anomaly: nonterminal stage unit {unit.get('id')} "
                    "does not belong to the exact active execution"
                )

        workers = fetch_records(
            "stage_workers",
            """/* run_tree:stage_workers */
            SELECT jsonb_build_object(
                'id', worker.id,
                'stage_execution_id', worker.stage_execution_id,
                'stage_run_unit_id', worker.stage_run_unit_id,
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
                'checkpoint', worker.checkpoint,
                'checkpoint_version', worker.checkpoint_version,
                'lease_token', worker.lease_token,
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
                f"        lease token={worker.get('lease_token')} owner={worker.get('lease_owner')} "
                f"epoch={worker.get('attempt_epoch')} expires={worker.get('lease_expires_at')} "
                f"expired={'yes' if worker.get('lease_expired') else 'no'} "
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
                f"parent_request={worker.get('parent_request_id')}"
            )
            out.append(
                f"        checkpoint={_compact_json(worker.get('checkpoint'), trunc)}"
            )
            if worker.get("status") == "recovery_required" or (
                worker.get("lease_expired") and worker.get("active_tool_call_id") is not None
            ):
                recovery = "manual_required"
            elif worker.get("lease_expired"):
                recovery = "requeue_eligible"
            elif worker.get("lease_token") is not None:
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
                'lease_token', submission.lease_token,
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
            if active_execution_id
            and str(unit.get("stage_execution_id")) == active_execution_id
            and bool(unit.get("scope_member"))
        ]
        v2_complete = len(active_executions) == 1 and (
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
