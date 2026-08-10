#!/usr/bin/env python3
"""Run a real headless Golish stage with an isolated embedded database.

This is intentionally a thin wrapper around `golish --stage-run`: it does not
mock the agent, stage gate, tools, transcript writer, or database writes.
"""

from __future__ import annotations

import argparse
import functools
import http.server
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading
import urllib.parse
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

# The built-in closed route plan is ~2k entries. A smoke default below that
# cannot ever reach a terminal DIR receipt within the Controller's two bounded
# repair attempts, so size this budget for one complete root pass plus baseline
# and curated probes.
DEFAULT_SMOKE_ROUTE_PROBE_MAX_RUNTIME_MS = 75_000
DEFAULT_SMOKE_ROUTE_PROBE_MAX_REQUESTS = 2_400
FIXTURE_AUDIT_SAMPLE_LIMIT = 24
LEGACY_STAGE_ORDER = (
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

UNIFIED_STAGE_ORDER = (
    "scoping",
    "target_intel",
    "external_attack_surface",
    "enumeration",
    "vuln_triage",
    "application_understanding",
    "investigation",
    "reporting",
)

REQUIRED_OPERATION_EXACT_SET_LABELS = (
    "stage_runs",
    "deliverable_submissions",
    "capability_receipts",
    "evidence_ledger",
    "enumeration_lane_receipts",
    "enumeration_endpoint_occurrences",
    "enumeration_parameter_assessments",
    "enumeration_occurrence_parameters",
    "enumeration_parameter_provenance",
    "enumeration_resolution_closeouts",
    "target_intel_goal_epochs",
    "target_intel_goal_reviews",
    "target_intel_goal_frontier",
    "target_intel_goal_work_journal",
    "target_intel_semantic_artifacts",
    "target_intel_asset_observations",
    "stage_handoffs",
    "application_model_revisions",
    "investigation_run_heads",
    "investigation_main_read_sessions",
    "investigation_analysis_bindings",
    "investigation_task_plans",
    "investigation_delegation_census",
    "hypothesis_revisions",
    "hypothesis_verification_tasks",
    "verification_campaigns",
    "verification_prepared_actions",
    "verification_action_authorizations",
    "verification_action_executions",
    "verification_fact_deltas",
    "hypothesis_residual_risks",
    "investigation_run_closures",
    "investigation_stop_intents",
    "investigation_projection_outbox",
    "investigation_closure_publications",
    "report_revisions",
    "report_input_seals",
    "report_revision_artifacts",
    "report_source_manifest",
    "operation_contract_adoptions",
)

SHA256_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
STATE_CHANGING_METHODS = frozenset({"POST", "PUT", "PATCH", "DELETE"})


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format: str, *args: object) -> None:
        return


class ControlledFixtureServer(http.server.ThreadingHTTPServer):
    def __init__(self, server_address: tuple[str, int], handler: object) -> None:
        super().__init__(server_address, handler)
        self._request_lock = threading.Lock()
        self._request_counts: Counter[tuple[str, str]] = Counter()

    def record_request(self, method: str, route: str) -> None:
        with self._request_lock:
            self._request_counts[(method, route)] += 1

    def request_counts(self) -> dict[tuple[str, str], int]:
        with self._request_lock:
            return dict(self._request_counts)


class ControlledFixtureHandler(QuietHandler):
    def _record(self, method: str, route: str) -> None:
        server = self.server
        if isinstance(server, ControlledFixtureServer):
            server.record_request(method, route)

    def _json(self, status: int, payload: str) -> None:
        encoded = payload.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("X-Golish-Fixture", "controlled-red-team-v2")
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self) -> None:  # noqa: N802 - stdlib handler contract
        route = urllib.parse.urlsplit(self.path).path
        self._record("GET", route)
        if route == "/api/users":
            self._json(200, '{"users":[],"next_cursor":null}\n')
            return
        if route == "/api/debug/config":
            self._json(
                200,
                '{"debug":true,"environment":"controlled-fixture",'
                '"diagnostic_token":"fixture-only-not-a-secret"}\n',
            )
            return
        if route.startswith("/api/orders/"):
            self._json(200, '{"order":{"id":"fixture-order","state":"preview"}}\n')
            return
        if route == "/api/search":
            self._json(200, '{"results":[],"source":"controlled-fixture"}\n')
            return
        if route == "/graphql":
            self._json(200, '{"data":{"__typename":"Query"}}\n')
            return
        if route in ("/dashboard", "/settings/profile"):
            self.path = "/index.html"
        super().do_GET()

    def do_POST(self) -> None:  # noqa: N802 - stdlib handler contract
        route = urllib.parse.urlsplit(self.path).path
        self._record("POST", route)
        content_length = min(int(self.headers.get("Content-Length", "0") or 0), 65_536)
        if content_length:
            self.rfile.read(content_length)
        if route == "/api/orders":
            self._json(202, '{"accepted":true,"id":"fixture-order"}\n')
            return
        if route == "/api/session":
            self._json(200, '{"authenticated":false,"fixture":true}\n')
            return
        if route == "/graphql":
            self._json(200, '{"data":{"user":null}}\n')
            return
        self._json(404, '{"error":"fixture_route_not_found"}\n')

    def do_OPTIONS(self) -> None:  # noqa: N802 - stdlib handler contract
        route = urllib.parse.urlsplit(self.path).path
        self._record("OPTIONS", route)
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "content-type, x-fixture-trace")
        self.end_headers()


@dataclass
class FixtureWeb:
    server: ControlledFixtureServer
    thread: threading.Thread
    auxiliary_servers: list[tuple[ControlledFixtureServer, threading.Thread]]
    root: Path
    url: str
    external_sdk_url: str
    host: str
    port: int

    def assert_read_only_safety(self) -> None:
        counts = self.server.request_counts()
        sorted_counts = sorted(counts.items())
        rendered = ", ".join(
            f"{method} {route}={count}"
            for (method, route), count in sorted_counts[:FIXTURE_AUDIT_SAMPLE_LIMIT]
        )
        omitted = max(len(sorted_counts) - FIXTURE_AUDIT_SAMPLE_LIMIT, 0)
        suffix = f", ... {omitted} route(s) omitted" if omitted else ""
        print(
            "[stage-smoke] fixture request audit: "
            f"total={sum(counts.values())} unique_routes={len(sorted_counts)}; "
            f"sample={rendered or 'no requests'}{suffix}"
        )
        unsafe_requests = {
            f"{method} {route}": count
            for (method, route), count in sorted(counts.items())
            if method.upper() in STATE_CHANGING_METHODS and count > 0
        }
        logout_count = counts.get(("GET", "/logout"), 0)
        if logout_count:
            unsafe_requests["GET /logout"] = logout_count
        if unsafe_requests:
            raise RuntimeError(
                "controlled fixture safety violation: unexpected state-changing or "
                f"logout requests={unsafe_requests}"
            )

    def assert_enumeration_capture(self) -> None:
        self.assert_read_only_safety()
        required_main_gets = (
            "/assets/bootstrap.js",
            "/assets/app.chunk.js",
            "/assets/profile.chunk.js",
            "/api/users",
            "/api/debug/config",
            "/api/orders/fixture-order",
        )
        main_counts = self.server.request_counts()
        missing = [
            route for route in required_main_gets if main_counts.get(("GET", route), 0) == 0
        ]
        vendor_counts = self.auxiliary_servers[0][0].request_counts()
        external_sdk_hits = vendor_counts.get(("GET", "/vendor/analytics-sdk.js"), 0)
        print(f"[stage-smoke] blocked external SDK hits: {external_sdk_hits}")
        if missing or external_sdk_hits != 0:
            raise RuntimeError(
                "controlled fixture Enumeration did not preserve the expected exact-origin browser boundary: "
                f"missing_main_gets={missing}, external_sdk_hits={external_sdk_hits}"
            )

    def assert_controlled_intel_transport(self) -> None:
        count = self.server.request_counts().get(("GET", "/intel/company.json"), 0)
        print(f"[stage-smoke] controlled Intel provider hits: {count}")
        if count == 0:
            raise RuntimeError(
                "controlled fixture Target Intel did not use the local provider transport"
            )

    def stop(self) -> None:
        self.server.shutdown()
        self.thread.join(timeout=5)
        self.server.server_close()
        for server, thread in self.auxiliary_servers:
            server.shutdown()
            thread.join(timeout=5)
            server.server_close()


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def start_fixture_web() -> FixtureWeb:
    root = Path(tempfile.mkdtemp(prefix="golish-stage-fixture-")).resolve()
    (root / "assets").mkdir(parents=True, exist_ok=True)
    (root / "vendor").mkdir(parents=True, exist_ok=True)
    vendor_handler = functools.partial(ControlledFixtureHandler, directory=str(root))
    vendor_server = ControlledFixtureServer(("127.0.0.1", 0), vendor_handler)
    vendor_thread = threading.Thread(target=vendor_server.serve_forever, daemon=True)
    vendor_thread.start()
    vendor_port = int(vendor_server.server_address[1])
    external_sdk_url = f"http://localhost:{vendor_port}/vendor/analytics-sdk.js"
    (root / "index.html").write_text(
        f"""<!doctype html>
<html>
<head><title>Golish smoke fixture</title></head>
<body>
  <h1>Golish controlled red-team fixture</h1>
  <nav>
    <a href="/dashboard">Dashboard</a>
    <a href="/settings/profile">Profile settings</a>
    <a href="/logout">Dangerous route must remain unsent</a>
  </nav>
  <form action="/api/session" method="post">
    <input name="email" type="email" required>
    <input name="password" type="password" required>
    <button type="submit">Sign in</button>
  </form>
  <script src="{external_sdk_url}?tenant=fixture-tenant"></script>
  <script type="module" src="/assets/bootstrap.js?build=fixture-build-42"></script>
</body>
</html>
""",
        encoding="utf-8",
    )
    (root / "vendor" / "analytics-sdk.js").write_text(
        """window.fixtureAnalytics = {
  track(eventName, properties) {
    return fetch(`/api/search?q=${encodeURIComponent(eventName)}&limit=5`, {
      headers: { "x-fixture-trace": properties?.traceId ?? "anonymous" },
    });
  },
};
//# sourceMappingURL=analytics-sdk.js.map
""",
        encoding="utf-8",
    )
    (root / "vendor" / "analytics-sdk.js.map").write_text(
        '{"version":3,"file":"analytics-sdk.js","sources":["sdk.ts"],"names":[],"mappings":"AAAA"}\n',
        encoding="utf-8",
    )
    (root / "assets" / "bootstrap.js").write_text(
        """import("/assets/app.chunk.js?chunk=primary").then(({ boot }) => boot());
import("/assets/app.chunk.js?chunk=duplicate-reference");
window.fixtureAnalytics?.track("page_boot", { traceId: "fixture-trace" });
""",
        encoding="utf-8",
    )
    (root / "assets" / "app.chunk.js").write_text(
        """const apiBase = "/api";
export async function boot() {
  await fetch(`${apiBase}/users?limit=25&cursor=fixture-cursor`);
  await fetch(`${apiBase}/debug/config`);
  await fetch(`${apiBase}/orders/fixture-order?include=history`);
  await import("/assets/profile.chunk.js?chunk=profile");
}
export async function createOrder({ sku, quantity, coupon }) {
  return fetch(`${apiBase}/orders`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-fixture-trace": "order-flow" },
    body: JSON.stringify({ sku, quantity, coupon }),
  });
}
//# sourceMappingURL=app.chunk.js.map
""",
        encoding="utf-8",
    )
    (root / "assets" / "profile.chunk.js").write_text(
        """export async function loadProfile(userId, includeHistory = false) {
  return fetch("/graphql", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      operationName: "ProfileById",
      query: "query ProfileById($userId: ID!, $includeHistory: Boolean!) { user(id: $userId) { id } }",
      variables: { userId, includeHistory },
    }),
  });
}
""",
        encoding="utf-8",
    )
    (root / "assets" / "app.chunk.js.map").write_text(
        '{"version":3,"file":"app.chunk.js","sources":["app.ts"],"names":[],"mappings":"AAAA"}\n',
        encoding="utf-8",
    )
    (root / "openapi.json").write_text(
        """{
  "openapi": "3.0.0",
  "info": {"title": "Golish controlled fixture", "version": "2"},
  "paths": {
    "/api/users": {"get": {"parameters": [{"name": "limit", "in": "query"}, {"name": "cursor", "in": "query"}]}},
    "/api/debug/config": {"get": {}},
    "/api/orders": {"post": {"requestBody": {"required": true}}},
    "/graphql": {"post": {}}
  }
}
""",
        encoding="utf-8",
    )
    (root / "intel").mkdir(parents=True, exist_ok=True)
    (root / "intel" / "company.json").write_text(
        '{"items":[],"source":"controlled-fixture-intel","checked":true}\n',
        encoding="utf-8",
    )
    (root / "robots.txt").write_text(
        "User-agent: *\nDisallow: /api/debug/\nSitemap: /openapi.json\n",
        encoding="utf-8",
    )

    handler = functools.partial(ControlledFixtureHandler, directory=str(root))
    server = ControlledFixtureServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    host, port = server.server_address[:2]
    return FixtureWeb(
        server=server,
        thread=thread,
        auxiliary_servers=[(vendor_server, vendor_thread)],
        root=root,
        url=f"http://{host}:{port}",
        external_sdk_url=external_sdk_url,
        host=str(host),
        port=int(port),
    )


def build_controlled_provider_overlay(fixture: FixtureWeb) -> tuple[Path, Path]:
    """Build a local-only provider registry without changing production resources."""
    root = repo_root()
    source_toolsconfig = root / "resources" / "toolsconfig"
    toolsconfig_dir = fixture.root / "controlled-toolsconfig"
    intel_providers_dir = fixture.root / "controlled-intel-providers"
    toolsconfig_dir.mkdir(parents=True, exist_ok=True)
    intel_providers_dir.mkdir(parents=True, exist_ok=True)

    for source in source_toolsconfig.glob("*.json"):
        # ENScan is an asset-intel provider embedded in toolsconfig. Keeping it
        # would silently preserve a real transport even if the standalone
        # provider directory were replaced.
        if source.name == "enscan-go.json":
            continue
        shutil.copy2(source, toolsconfig_dir / source.name)

    descriptor = {
        "tool": {
            "id": "controlled-fixture-intel",
            "name": "Controlled Fixture Asset Intel",
            "description": "Local-only deterministic provider for stage-smoke acceptance.",
            "executable": "",
            "runtime": "native",
            "launchMode": "cli",
            "category": "recon",
            "subcategory": "osint",
            "tags": ["recon", "osint", "controlled-fixture", "no-credential"],
            "pentestPhase": ["recon"],
            "tier": "recommended",
            "asset_intel": {
                "enabled": True,
                "provider_id": "controlled-fixture-intel",
                "display_name": "Controlled Fixture Asset Intel",
                "capabilities": [
                    "domains",
                    "dns_records",
                    "asns",
                    "certificate_transparency",
                    "contacts",
                ],
                "auto": {"default": True, "priority": 1},
                "runtime": {
                    "kind": "http_json",
                    "requests": [
                        {
                            "id": "company_profile",
                            "method": "GET",
                            "url": f"{fixture.url}/intel/company.json?company={{{{company_name}}}}",
                            "headers": {"Accept": "application/json"},
                            "timeout_secs": 5,
                            "applicable_pivot_kinds": ["company_name", "brand"],
                            "wire_query_type": "controlled_fixture",
                            "adapter_version": "controlled_fixture_intel.v1",
                            "literal_encoder": "url_query_component.v1",
                        }
                    ],
                },
                "normalize": {"target": [], "profile_fields": []},
            },
        }
    }
    (intel_providers_dir / "controlled-fixture-intel.json").write_text(
        json.dumps(descriptor, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    return toolsconfig_dir, intel_providers_dir


def build_controlled_nuclei_overlay(fixture: FixtureWeb) -> Path:
    """Build a closed, GET-only Nuclei tree for controlled acceptance.

    Receipt-v1 deliberately treats a generic CLI no-match as inconclusive, so
    the controlled fixture must provide deterministic positive signals for the
    eight general Vuln techniques.  These templates still run through the real
    Nuclei binary, parser, guarded landing, evidence ledger and worklist; the
    only substituted input is the local template supply.  Every request stays
    on the fixture's exact origin and uses GET.
    """
    root = fixture.root / "controlled-nuclei-templates"
    http_dir = root / "http"
    dast_dir = root / "dast"
    http_dir.mkdir(parents=True, exist_ok=True)
    dast_dir.mkdir(parents=True, exist_ok=True)

    baseline_tags = ("default-login", "csrf", "exposure", "ssl", "disclosure")
    for tag in baseline_tags:
        template_id = f"golish-controlled-{tag}"
        (http_dir / f"{template_id}.yaml").write_text(
            f"""id: {template_id}

info:
  name: Golish controlled {tag} signal
  author: golish
  severity: info
  tags: {tag}

http:
  - method: GET
    path:
      - \"{{{{BaseURL}}}}/api/debug/config\"
    redirects: false
    max-redirects: 0
    matchers:
      - type: word
        part: header
        words:
          - \"controlled-red-team-v2\"
""",
            encoding="utf-8",
        )

    for tag in ("sqli", "xss", "rce"):
        template_id = f"golish-controlled-{tag}-dast"
        (dast_dir / f"{template_id}.yaml").write_text(
            f"""id: {template_id}

info:
  name: Golish controlled {tag} DAST signal
  author: golish
  severity: info
  tags: {tag},dast

http:
  - pre-condition:
      - type: dsl
        dsl:
          - 'method == \"GET\"'
          - 'len(query_params) > 0'
    fuzzing:
      - part: query
        mode: single
        type: replace
        fuzz:
          - \"golish-controlled-{tag}\"
    matchers:
      - type: word
        part: header
        words:
          - \"controlled-red-team-v2\"
""",
            encoding="utf-8",
        )
    return root


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
        "--org-id",
        help=(
            "Preserve an already-resolved organization UUID inside the isolated "
            "ephemeral database. Requires --org."
        ),
    )
    parser.add_argument(
        "--target",
        action="append",
        default=[],
        help="Seed in-scope target. Repeatable.",
    )
    parser.add_argument(
        "--fixture-web",
        dest="fixture_web",
        action="store_true",
        help="Start the controlled local HTTP fixture and seed it as an in-scope URL target.",
    )
    parser.add_argument(
        "--controlled-fixture",
        action="store_true",
        help=(
            "Acceptance preset: start the controlled local fixture and select "
            "the unified topology plus JSON exact-set validation."
        ),
    )
    parser.add_argument(
        "--unified-topology",
        action="store_true",
        help=(
            "Select the new-only unified AU/Investigation topology in the fresh "
            "ephemeral DB before creating the operation."
        ),
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
        help="Run scripts/run_tree.py --db --full while the embedded DB is live.",
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


def apply_acceptance_preset(args: argparse.Namespace) -> None:
    if not args.controlled_fixture:
        return
    args.fixture_web = True
    args.unified_topology = True
    args.json = True


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


V2_COMPANY_CONTROLLER_STAGES = frozenset(
    ("target_intel", "external_attack_surface", "enumeration", "vuln_triage")
)


def selected_stage_slice(args: argparse.Namespace) -> tuple[str, ...]:
    if args.only_stage:
        return (args.only_stage,)
    terminal = args.to_stage or args.stage
    if terminal is None:
        return ()
    entry = args.from_stage or "scoping"
    for order in (UNIFIED_STAGE_ORDER, LEGACY_STAGE_ORDER):
        if entry not in order or terminal not in order:
            continue
        entry_index = order.index(entry)
        terminal_index = order.index(terminal)
        if entry_index <= terminal_index:
            return tuple(order[entry_index : terminal_index + 1])
    return ()


def validate_stage_contract_args(
    args: argparse.Namespace, parser: argparse.ArgumentParser
) -> None:
    selected = set(selected_stage_slice(args))
    if (
        args.profile == "red_team"
        and selected.intersection(V2_COMPANY_CONTROLLER_STAGES)
        and not args.unified_topology
    ):
        parser.error(
            "red_team smoke slices containing Target Intel/EAS/Enumeration/Vuln "
            "require --unified-topology so the isolated operation is frozen to "
            "the v2_only/new_only contract before it is created"
        )


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
    orders = (UNIFIED_STAGE_ORDER,) if args.unified_topology else (
        LEGACY_STAGE_ORDER,
        UNIFIED_STAGE_ORDER,
    )
    for order in orders:
        if entry not in order or terminal not in order:
            continue
        entry_index = order.index(entry)
        enumeration_index = order.index("enumeration")
        terminal_index = order.index(terminal)
        return entry_index <= enumeration_index <= terminal_index
    else:
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


MAX_STREAMED_JSON_EVENT_CHARS = 32_768
SUPPRESSED_STREAM_EVENT_TYPES = {
    "assistant_delta",
    "reasoning_delta",
    "sub_agent_started",
    "sub_agent_reasoning",
    "sub_agent_text_delta",
}


def emit_bounded_json_event(event: dict[str, object], line: str) -> None:
    if event.get("type") in SUPPRESSED_STREAM_EVENT_TYPES:
        return
    if len(line) <= MAX_STREAMED_JSON_EVENT_CHARS:
        print(line, end="")
        return
    identity = {
        key: event[key]
        for key in ("type", "stage", "status", "code")
        if key in event and isinstance(event[key], (str, int, float, bool))
    }
    error = event.get("error")
    if isinstance(error, str):
        identity["error"] = error[:512]
    print(
        "[stage-smoke] bounded oversized JSON event: "
        f"chars={len(line)} identity={json.dumps(identity, ensure_ascii=False, sort_keys=True)}"
    )


def run_stage_process(
    cmd: list[str],
    backend_dir: Path,
    run_env: dict[str, str],
    run_tree_cmd: list[str] | None = None,
    run_tree_ack_path: Path | None = None,
) -> tuple[int, dict[str, object] | None]:
    if (run_tree_cmd is None) != (run_tree_ack_path is None):
        raise ValueError("run-tree command and acknowledgement path must be configured together")
    process = subprocess.Popen(
        cmd,
        cwd=backend_dir,
        env=run_env,
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    summary: dict[str, object] | None = None
    diagnostic_error: subprocess.CalledProcessError | RuntimeError | None = None
    diagnostic_ran = False
    assert process.stdout is not None
    for line in process.stdout:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            print(line, end="")
            continue
        is_diagnostic_ready = (
            isinstance(event, dict)
            and event.get("type") == "db_smoke_diagnostic_ready"
        )
        is_db_smoke_summary = (
            isinstance(event, dict) and event.get("type") == "db_smoke_summary"
        )
        if not is_diagnostic_ready and not is_db_smoke_summary:
            emit_bounded_json_event(event, line)
        if is_db_smoke_summary:
            candidate = event.get("summary")
            if isinstance(candidate, dict):
                summary = candidate
        if isinstance(event, dict) and event.get("type") == "db_smoke_diagnostic_ready":
            if run_tree_cmd is None or run_tree_ack_path is None:
                diagnostic_error = RuntimeError(
                    "stage emitted a live DB diagnostic handshake without --run-tree"
                )
                continue
            if diagnostic_ran:
                diagnostic_error = RuntimeError(
                    "stage emitted more than one live DB diagnostic handshake"
                )
                continue
            diagnostic_ran = True
            db_url = event.get("dbUrl")
            try:
                if not isinstance(db_url, str) or not db_url.startswith(
                    "postgres://golish:"
                ):
                    raise RuntimeError("stage emitted an invalid live embedded DB URL")
                print("[stage-smoke] run_tree --db --full against live embedded DB")
                subprocess.run(
                    [*run_tree_cmd, "--db", "--db-url", db_url],
                    cwd=backend_dir.parent,
                    check=True,
                )
            except (subprocess.CalledProcessError, RuntimeError) as error:
                diagnostic_error = error
            finally:
                run_tree_ack_path.touch(exist_ok=False)
    process.stdout.close()
    returncode = process.wait()
    if run_tree_cmd is not None and not diagnostic_ran and diagnostic_error is None:
        diagnostic_error = RuntimeError(
            "stage exited without a live DB diagnostic handshake"
        )
    if diagnostic_error is not None:
        raise RuntimeError(f"live run_tree diagnostic failed: {diagnostic_error}")
    return returncode, summary


def validate_unified_reporting_summary(summary: dict[str, object] | None) -> None:
    if summary is None:
        raise RuntimeError("unified reporting smoke did not emit db_smoke_summary JSON")
    identity = summary.get("operation_identity")
    if not isinstance(identity, dict):
        raise RuntimeError("db_smoke_summary operation_identity is missing")
    expected_identity = {
        "stageTopologyContract": "unified_investigation_v1",
        "investigationContractVersion": "hypothesis_registry_v1",
        "investigationRolloutMode": "new_only",
    }
    identity_mismatch = {
        key: {"expected": expected, "actual": identity.get(key)}
        for key, expected in expected_identity.items()
        if identity.get(key) != expected
    }
    exact_sets = summary.get("operation_exact_sets")
    if not isinstance(exact_sets, dict):
        raise RuntimeError("db_smoke_summary operation_exact_sets is missing")
    missing_exact_sets = sorted(set(REQUIRED_OPERATION_EXACT_SET_LABELS) - set(exact_sets))
    malformed_exact_sets = []
    for label in REQUIRED_OPERATION_EXACT_SET_LABELS:
        exact_set = exact_sets.get(label)
        if not isinstance(exact_set, dict):
            if label in exact_sets:
                malformed_exact_sets.append(label)
            continue
        members = exact_set.get("members")
        count = exact_set.get("memberCount")
        member_hash = exact_set.get("memberSetHash")
        if (
            not isinstance(count, int)
            or count < 0
            or not isinstance(members, list)
            or count != len(members)
            or not isinstance(member_hash, str)
            or SHA256_RE.fullmatch(member_hash) is None
        ):
            malformed_exact_sets.append(label)

    stage_set = exact_sets.get("stage_runs")
    stage_members = stage_set.get("members") if isinstance(stage_set, dict) else None
    observed_stage_order = [
        member.get("stage")
        for member in stage_members or []
        if isinstance(member, dict)
    ]
    stage_order_mismatch = observed_stage_order != list(UNIFIED_STAGE_ORDER)
    non_completed_stages = [
        member.get("stage")
        for member in stage_members or []
        if not isinstance(member, dict) or member.get("status") != "completed"
    ]
    investigation_run_count = observed_stage_order.count("investigation")

    lane_set = exact_sets.get("enumeration_lane_receipts")
    lane_members = lane_set.get("members") if isinstance(lane_set, dict) else None
    expected_lanes = {"browser", "js_api", "parameter", "resolution", "coverage"}
    observed_lanes = {
        member.get("lane") for member in lane_members or [] if isinstance(member, dict)
    }
    missing_lanes = sorted(expected_lanes - observed_lanes)
    unexpected_lanes = sorted(observed_lanes - expected_lanes)
    lane_subject_keys = []
    invalid_lane_subjects = []
    for member in lane_members or []:
        if not isinstance(member, dict):
            invalid_lane_subjects.append(None)
            continue
        lane = member.get("lane")
        target_id = member.get("targetId")
        if lane == "resolution":
            subject_id = member.get("resolutionOccurrenceId")
        else:
            subject_id = target_id
        if not isinstance(lane, str) or not isinstance(subject_id, str) or not subject_id:
            invalid_lane_subjects.append(member.get("receiptId"))
            continue
        lane_subject_keys.append((lane, subject_id))
    duplicate_lane_subjects = len(lane_subject_keys) != len(set(lane_subject_keys))
    nonempty_lane_missing = [
        member.get("receiptId")
        for member in lane_members or []
        if isinstance(member, dict) and member.get("missing")
    ]

    required_nonempty_sets = (
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
        "investigation_run_heads",
        "investigation_main_read_sessions",
        "investigation_analysis_bindings",
        "investigation_task_plans",
        "investigation_delegation_census",
        "hypothesis_revisions",
        "hypothesis_verification_tasks",
        "verification_campaigns",
        "verification_fact_deltas",
        "hypothesis_residual_risks",
        "investigation_run_closures",
        "investigation_stop_intents",
        "investigation_projection_outbox",
        "investigation_closure_publications",
        "report_revisions",
        "report_source_manifest",
    )
    empty_sets = [
        label
        for label in required_nonempty_sets
        if not isinstance(exact_sets.get(label), dict)
        or int(exact_sets[label].get("memberCount", 0)) < 1
    ]

    run_head_members = exact_sets.get("investigation_run_heads", {}).get("members", [])
    run_head_mismatch = len(run_head_members) != 1

    read_session_members = exact_sets.get("investigation_main_read_sessions", {}).get(
        "members", []
    )
    read_session_orgs = [
        member.get("organizationId")
        for member in read_session_members
        if isinstance(member, dict)
    ]
    read_session_isolation_mismatch = (
        len(read_session_orgs) != len(set(read_session_orgs))
        or any(not organization_id for organization_id in read_session_orgs)
        or len(
            {
                member.get("contextChainId")
                for member in read_session_members
                if isinstance(member, dict)
            }
        )
        != len(read_session_members)
        or len(
            {
                member.get("transcriptPartitionId")
                for member in read_session_members
                if isinstance(member, dict)
            }
        )
        != len(read_session_members)
    )

    task_plan_members = exact_sets.get("investigation_task_plans", {}).get("members", [])
    task_plan_ids = {
        member.get("taskPlanId") for member in task_plan_members if isinstance(member, dict)
    }
    census_members = exact_sets.get("investigation_delegation_census", {}).get("members", [])
    census_task_plan_ids = {
        member.get("taskPlanId") for member in census_members if isinstance(member, dict)
    }
    delegation_census_mismatch = (
        len(task_plan_ids) != len(task_plan_members)
        or len(census_task_plan_ids) != len(census_members)
        or task_plan_ids != census_task_plan_ids
        or any(
            not member.get("primaryDispatchReceiptId")
            or not member.get("primaryWorkerRunId")
            for member in census_members
            if isinstance(member, dict)
        )
    )

    verification_task_members = exact_sets.get("hypothesis_verification_tasks", {}).get(
        "members", []
    )
    undrained_verification_tasks = [
        member.get("taskId")
        for member in verification_task_members
        if not isinstance(member, dict)
        or member.get("currentState") not in {"terminal", "cancelled", "blocked"}
    ]
    campaign_members = exact_sets.get("verification_campaigns", {}).get("members", [])
    unterminated_campaigns = [
        member.get("campaignId")
        for member in campaign_members
        if not isinstance(member, dict)
        or member.get("state") not in {"terminal", "superseded"}
    ]
    report_members = exact_sets.get("report_revisions", {}).get("members", [])
    invalid_summary_reports = [
        member.get("revisionId")
        for member in report_members
        if not isinstance(member, dict)
        or member.get("validationStatus") != "validated"
        or member.get("publicationStatus") != "unpublished"
    ]
    report_cardinality_mismatch = len(report_members) != 1
    unexpected_report_input_seals = int(
        exact_sets.get("report_input_seals", {}).get("memberCount", -1)
    ) != 0
    unexpected_report_artifacts = int(
        exact_sets.get("report_revision_artifacts", {}).get("memberCount", -1)
    ) != 0
    if (
        identity_mismatch
        or missing_exact_sets
        or malformed_exact_sets
        or stage_order_mismatch
        or non_completed_stages
        or investigation_run_count != 1
        or missing_lanes
        or unexpected_lanes
        or duplicate_lane_subjects
        or invalid_lane_subjects
        or nonempty_lane_missing
        or empty_sets
        or run_head_mismatch
        or read_session_isolation_mismatch
        or delegation_census_mismatch
        or undrained_verification_tasks
        or unterminated_campaigns
        or report_cardinality_mismatch
        or invalid_summary_reports
        or unexpected_report_input_seals
        or unexpected_report_artifacts
    ):
        raise RuntimeError(
            "unified reporting exact-set validation failed: "
            f"identity_mismatch={identity_mismatch}, missing_exact_sets={missing_exact_sets}, "
            f"malformed_exact_sets={malformed_exact_sets}, stage_order_mismatch={stage_order_mismatch}, "
            f"non_completed_stages={non_completed_stages}, investigation_run_count={investigation_run_count}, "
            f"missing_lanes={missing_lanes}, unexpected_lanes={unexpected_lanes}, "
            f"duplicate_lane_subjects={duplicate_lane_subjects}, "
            f"invalid_lane_subjects={invalid_lane_subjects}, "
            f"lane_missing_receipts={nonempty_lane_missing}, "
            f"empty_sets={empty_sets}, run_head_mismatch={run_head_mismatch}, "
            f"read_session_isolation_mismatch={read_session_isolation_mismatch}, "
            f"delegation_census_mismatch={delegation_census_mismatch}, "
            f"undrained_verification_tasks={undrained_verification_tasks}, "
            f"unterminated_campaigns={unterminated_campaigns}, "
            f"report_cardinality_mismatch={report_cardinality_mismatch}, "
            f"invalid_summary_reports={invalid_summary_reports}, "
            f"unexpected_report_input_seals={unexpected_report_input_seals}, "
            f"unexpected_report_artifacts={unexpected_report_artifacts}"
        )


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    apply_acceptance_preset(args)
    stage_args = resolve_stage_args(args, parser)
    validate_stage_contract_args(args, parser)

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
    if args.unified_topology:
        cmd.extend(["--stage-run-test-joint-rank", "6"])
    if args.controlled_fixture:
        if fixture is None:
            raise RuntimeError("controlled fixture provider overlay requires fixture web")
        toolsconfig_dir, intel_providers_dir = build_controlled_provider_overlay(fixture)
        controlled_nuclei_dir = build_controlled_nuclei_overlay(fixture)
        cmd.extend(
            [
                "--stage-run-test-toolsconfig-dir",
                str(toolsconfig_dir),
                "--stage-run-test-intel-providers-dir",
                str(intel_providers_dir),
                "--stage-run-test-intel-provider-endpoint",
                f"{fixture.url}/intel/company.json",
            ]
        )
    if args.json:
        cmd.append("--json")
    if args.org:
        cmd.extend(["--org", args.org])
    if args.org_id:
        if not args.org:
            parser.error("--org-id requires --org")
        cmd.extend(["--stage-run-test-organization-id", args.org_id])
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
    run_env.pop("GOLISH_STAGE_RUN_DB_DIAGNOSTIC_ACK", None)
    run_tree_tempdir: tempfile.TemporaryDirectory[str] | None = None
    run_tree_cmd: list[str] | None = None
    run_tree_ack_path: Path | None = None
    if args.run_tree:
        run_tree_tempdir = tempfile.TemporaryDirectory(
            prefix="golish-stage-run-db-diagnostic-"
        )
        run_tree_ack_path = Path(run_tree_tempdir.name) / "run-tree.complete"
        run_env["GOLISH_STAGE_RUN_DB_DIAGNOSTIC_ACK"] = str(run_tree_ack_path)
        run_tree_cmd = [
            sys.executable,
            str(root / "scripts" / "run_tree.py"),
            "--workspace",
            str(workspace),
            "--full",
        ]
    route_budget = route_probe_budget(args)
    if route_budget is not None:
        runtime_ms, max_requests = route_budget
        run_env["GOLISH_ROUTE_PROBE_DEFAULT_MAX_RUNTIME_MS"] = str(runtime_ms)
        run_env["GOLISH_ROUTE_PROBE_DEFAULT_MAX_REQUESTS"] = str(max_requests)
        print(
            "[stage-smoke] route_probe_paths budget: "
            f"max_runtime_ms={runtime_ms} max_requests={max_requests}"
        )
    if fixture is not None:
        run_env["GOLISH_STAGE_RUN_SEED_OPEN_PORTS"] = f"{fixture.host}={fixture.port}"
        print(
            "[stage-smoke] seeded confirmed-open fixture port: "
            f"{fixture.host}:{fixture.port}"
        )
        if args.controlled_fixture and selected_stage_slice(args)[0] == "enumeration":
            run_env["GOLISH_STAGE_RUN_SEED_CONFIRMED_WEB_ORIGINS"] = fixture.url
            print(
                "[stage-smoke] seeded controlled direct-Enumeration Web Origin: "
                f"{fixture.url}"
            )
        if args.controlled_fixture:
            run_env["GOLISH_NUCLEI_TEMPLATES_DIR"] = str(controlled_nuclei_dir)
            print(
                "[stage-smoke] controlled GET-only Nuclei templates: "
                f"{controlled_nuclei_dir}"
            )

    try:
        returncode, summary = run_stage_process(
            cmd,
            backend_dir,
            run_env,
            run_tree_cmd=run_tree_cmd,
            run_tree_ack_path=run_tree_ack_path,
        )
        if fixture is not None:
            if returncode == 0 and includes_enumeration(args):
                fixture.assert_enumeration_capture()
            else:
                fixture.assert_read_only_safety()
            if returncode == 0 and "target_intel" in selected_stage_slice(args):
                fixture.assert_controlled_intel_transport()
        terminal_stage = args.only_stage or args.to_stage or args.stage
        if returncode == 0 and args.unified_topology and terminal_stage == "reporting":
            if not args.json:
                raise RuntimeError(
                    "unified reporting smoke requires --json so exact DB sets can be validated"
                )
            validate_unified_reporting_summary(summary)
        return returncode
    finally:
        if fixture is not None:
            fixture.stop()
        if run_tree_tempdir is not None:
            run_tree_tempdir.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())
