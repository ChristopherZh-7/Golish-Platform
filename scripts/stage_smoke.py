#!/usr/bin/env python3
"""Run a real headless Golish stage with an isolated embedded database.

This is intentionally a thin wrapper around `golish --stage-run`: it does not
mock the agent, stage gate, tools, transcript writer, or database writes.
"""

from __future__ import annotations

import argparse
import functools
import http.server
import os
import shlex
import subprocess
import sys
import tempfile
import threading
from dataclasses import dataclass
from pathlib import Path

DEFAULT_SMOKE_ROUTE_PROBE_MAX_RUNTIME_MS = 30_000
DEFAULT_SMOKE_ROUTE_PROBE_MAX_REQUESTS = 800
SMOKE_STAGE_ORDER = (
    "scoping",
    "target_intel",
    "external_attack_surface",
    "enumeration",
    "vuln_triage",
    "attack_candidate",
    "verification",
    "access_validation",
    "internal_discovery",
    "objective_pathing",
    "objective_simulation",
    "reporting",
    "cleanup",
)


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format: str, *args: object) -> None:
        return


@dataclass
class FixtureWeb:
    server: http.server.ThreadingHTTPServer
    thread: threading.Thread
    root: Path
    url: str

    def stop(self) -> None:
        self.server.shutdown()
        self.thread.join(timeout=5)
        self.server.server_close()


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def start_fixture_web() -> FixtureWeb:
    root = Path(tempfile.mkdtemp(prefix="golish-stage-fixture-")).resolve()
    (root / "api").mkdir(parents=True, exist_ok=True)
    (root / "index.html").write_text(
        """<!doctype html>
<html>
<head><title>Golish smoke fixture</title></head>
<body>
  <h1>Golish smoke fixture</h1>
  <script src="/app.js"></script>
</body>
</html>
""",
        encoding="utf-8",
    )
    (root / "app.js").write_text(
        """const apiBase = "/api";
fetch(`${apiBase}/users?limit=10`);
fetch(`${apiBase}/orders`, { method: "POST" });
""",
        encoding="utf-8",
    )
    (root / "api" / "users").write_text('{"users":[]}\n', encoding="utf-8")
    (root / "api" / "orders").write_text('{"orders":[]}\n', encoding="utf-8")

    handler = functools.partial(QuietHandler, directory=str(root))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    host, port = server.server_address[:2]
    return FixtureWeb(
        server=server,
        thread=thread,
        root=root,
        url=f"http://{host}:{port}",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Run golish --stage-run against an isolated temporary DB.",
    )
    parser.add_argument("stage", nargs="?", help="Stage to run through --to.")
    parser.add_argument("--profile", default="assessment", help="Harness profile id.")
    parser.add_argument("--provider", help="LLM provider to pass to golish, for example deepseek.")
    parser.add_argument("--model", help="LLM model id to pass to golish, for example deepseek-v4-flash.")
    parser.add_argument("--from", dest="from_stage", help="Inclusive start stage.")
    parser.add_argument("--to", dest="to_stage", help="Inclusive end stage.")
    parser.add_argument("--only", dest="only_stage", help="Run exactly one stage.")
    parser.add_argument("--workspace", type=Path, help="Workspace to use for transcripts.")
    parser.add_argument("--org", default="Golish Smoke Org", help="Seed organization name.")
    parser.add_argument(
        "--target",
        action="append",
        default=[],
        help="Seed in-scope target. Repeatable.",
    )
    parser.add_argument(
        "--fixture-web",
        action="store_true",
        help="Start a local HTTP fixture and seed it as an in-scope URL target.",
    )
    parser.add_argument("--objective", help="Override the generated stage objective.")
    parser.add_argument(
        "--keep-ephemeral-db",
        action="store_true",
        help="Pass through to golish --keep-ephemeral-db.",
    )
    parser.add_argument("--json", action="store_true", help="Emit stage-run JSON lines.")
    parser.add_argument(
        "--no-auto-approve",
        action="store_true",
        help="Do not pass --auto-approve to the runner.",
    )
    parser.add_argument(
        "--approve-phase-boundaries",
        action="store_true",
        help=(
            "Explicitly approve profile-defined phase confirmations so a multi-phase "
            "headless slice can cross the same boundary the GUI renders as a Confirm card."
        ),
    )
    parser.add_argument(
        "--include-subsidiaries",
        action="store_true",
        help="Pass through stage-run subsidiary scope mode.",
    )
    parser.add_argument(
        "--subsidiary-threshold",
        type=int,
        help="Pass through subsidiary ownership threshold.",
    )
    parser.add_argument(
        "--run-tree",
        action="store_true",
        help="Run scripts/run_tree.py --full after the stage exits.",
    )
    parser.add_argument(
        "--route-probe-max-runtime-ms",
        type=int,
        help=(
            "For enumeration smoke, set GOLISH_ROUTE_PROBE_DEFAULT_MAX_RUNTIME_MS "
            "and mention this budget in the objective."
        ),
    )
    parser.add_argument(
        "--route-probe-max-requests",
        type=int,
        help=(
            "For enumeration smoke, set GOLISH_ROUTE_PROBE_DEFAULT_MAX_REQUESTS "
            "and mention this budget in the objective."
        ),
    )
    parser.add_argument(
        "--full-route-probe",
        action="store_true",
        help="Do not auto-bound route_probe_paths for enumeration smoke.",
    )
    return parser


def resolve_stage_args(args: argparse.Namespace, parser: argparse.ArgumentParser) -> list[str]:
    if args.stage and (args.to_stage or args.only_stage):
        parser.error("use positional stage, --to, or --only; not multiple forms")
    if args.only_stage:
        return ["--only", args.only_stage]

    to_stage = args.to_stage or args.stage
    if not to_stage:
        parser.error("provide a stage positional, --to, or --only")

    stage_args: list[str] = []
    if args.from_stage:
        stage_args.extend(["--from", args.from_stage])
    stage_args.extend(["--to", to_stage])
    return stage_args


def default_objective(args: argparse.Namespace, stage_args: list[str], targets: list[str]) -> str:
    stage = args.only_stage or args.to_stage or args.stage or "requested"
    target_text = ", ".join(targets) if targets else "the seeded workspace scope"
    objective = (
        f"Run the {stage} smoke test stage against {target_text}. "
        "Use real stage tools when available, book evidence, and submit the stage deliverable."
    )
    route_budget = route_probe_budget(args)
    if includes_enumeration(args) and route_budget is not None:
        runtime_ms, max_requests = route_budget
        objective += (
            " For this smoke run, keep route_probe_paths bounded: pass "
            f"max_runtime_ms={runtime_ms} and max_requests={max_requests}; "
            "after timeout_partial or request_limited_partial, refresh coverage and submit only if "
            "the DB-backed preflight says ready."
        )
    return objective


def includes_enumeration(args: argparse.Namespace) -> bool:
    if args.only_stage:
        return args.only_stage == "enumeration"

    terminal = args.to_stage or args.stage
    if terminal is None:
        return False
    entry = args.from_stage or "scoping"
    try:
        entry_index = SMOKE_STAGE_ORDER.index(entry)
        enumeration_index = SMOKE_STAGE_ORDER.index("enumeration")
        terminal_index = SMOKE_STAGE_ORDER.index(terminal)
    except ValueError:
        # The Rust CLI owns stage validation. Unknown wrapper input must not
        # accidentally claim a route-probe budget for a route we cannot prove.
        return False
    return entry_index <= enumeration_index <= terminal_index


def route_probe_budget(args: argparse.Namespace) -> tuple[int, int] | None:
    if args.full_route_probe or not includes_enumeration(args):
        return None
    runtime_ms = args.route_probe_max_runtime_ms or DEFAULT_SMOKE_ROUTE_PROBE_MAX_RUNTIME_MS
    max_requests = args.route_probe_max_requests or DEFAULT_SMOKE_ROUTE_PROBE_MAX_REQUESTS
    return max(runtime_ms, 1_000), max(max_requests, 1)


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    stage_args = resolve_stage_args(args, parser)

    workspace = args.workspace
    if workspace is None:
        workspace = Path(tempfile.mkdtemp(prefix="golish-stage-workspace-"))
    workspace = workspace.resolve()
    workspace.mkdir(parents=True, exist_ok=True)

    fixture: FixtureWeb | None = None
    targets = list(args.target)
    if args.fixture_web:
        fixture = start_fixture_web()
        targets.append(fixture.url)
        print(f"[stage-smoke] fixture web: {fixture.url} (root={fixture.root})")

    objective = args.objective or default_objective(args, stage_args, targets)
    root = repo_root()
    backend_dir = root / "backend"

    cmd = [
        "cargo",
        "run",
        "-q",
        "-p",
        "golish",
        "--bin",
        "golish",
        "--",
        str(workspace),
        "--stage-run",
        "--ephemeral-db",
        "--db-smoke-summary",
        "--profile",
        args.profile,
        *stage_args,
    ]
    if args.provider:
        cmd.extend(["--provider", args.provider])
    if args.model:
        cmd.extend(["--model", args.model])
    if not args.no_auto_approve:
        cmd.append("--auto-approve")
    if args.approve_phase_boundaries:
        cmd.append("--approve-phase-boundaries")
    if args.keep_ephemeral_db:
        cmd.append("--keep-ephemeral-db")
    if args.json:
        cmd.append("--json")
    if args.org:
        cmd.extend(["--org", args.org])
    for target in targets:
        cmd.extend(["--target", target])
    if args.include_subsidiaries:
        cmd.append("--include-subsidiaries")
    if args.subsidiary_threshold is not None:
        cmd.extend(["--subsidiary-threshold", str(args.subsidiary_threshold)])
    cmd.extend(["-e", objective])

    print(f"[stage-smoke] workspace: {workspace}")
    print("[stage-smoke] command:")
    print("  " + " ".join(shlex.quote(part) for part in cmd))

    run_env = os.environ.copy()
    route_budget = route_probe_budget(args)
    if route_budget is not None:
        runtime_ms, max_requests = route_budget
        run_env["GOLISH_ROUTE_PROBE_DEFAULT_MAX_RUNTIME_MS"] = str(runtime_ms)
        run_env["GOLISH_ROUTE_PROBE_DEFAULT_MAX_REQUESTS"] = str(max_requests)
        print(
            "[stage-smoke] route_probe_paths budget: "
            f"max_runtime_ms={runtime_ms} max_requests={max_requests}"
        )

    try:
        result = subprocess.run(cmd, cwd=backend_dir, env=run_env, check=False)
        if args.run_tree:
            run_tree_cmd = [
                sys.executable,
                str(root / "scripts" / "run_tree.py"),
                "--workspace",
                str(workspace),
                "--full",
            ]
            print("[stage-smoke] run_tree:")
            print("  " + " ".join(shlex.quote(part) for part in run_tree_cmd))
            subprocess.run(run_tree_cmd, cwd=root, check=False)
        return result.returncode
    finally:
        if fixture is not None:
            fixture.stop()


if __name__ == "__main__":
    raise SystemExit(main())
