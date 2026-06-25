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
            sessions = [d for d in root.iterdir() if d.is_dir() and (d / "transcript.json").exists()]
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

    if want_db:
        print("\n".join(run_db_diagnosis(session_dir.name, db_url, trunc)))
    return 0


def run_db_diagnosis(session_id: str, db_url: str | None, trunc: int) -> list[str]:
    """Deterministic DB checks for root causes transcripts can't show.

    Each query degrades independently: a missing table/column or a closed DB
    never aborts the report, it just prints what it could read.
    """
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


if __name__ == "__main__":
    raise SystemExit(main())
