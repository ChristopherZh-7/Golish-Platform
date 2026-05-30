#!/usr/bin/env python3
"""Data-ownership boundary guard for the Golish command layer.

golish-db holds ONE Postgres with ~42 repo modules for ALL services
(recon / vuln / pentest / agent / platform). To prepare for future
DB-per-service servitization, this guard enforces that a command-layer
module only reaches repos owned by ITS OWN service. Existing cross-service
coupling is frozen in ALLOWLIST (a ratchet); NEW coupling is blocked.
Each allowlist entry is a future `*Port` extraction candidate
(design: docs/design/2026-05-30-servitization-readiness.md §6 S1-2).

Rules:
- Every `pub mod X;` in golish-db/src/repo/mod.rs must appear in
  REPO_OWNER or SHARED_REPOS (forces new repos onto the ownership map).
- A `golish_db::repo::<name>` usage in golish/src is legal iff:
    name in SHARED_REPOS, OR
    owner(name) == domain(caller_file), OR
    (caller_file, name) in ALLOWLIST.
- (Secondary) raw `sqlx::query*` in golish/src is legal iff caller_file
  in RAW_SQL_ALLOWLIST (steers new DB access through golish-db repos;
  overlaps with P0-3 scoped-SQL-down-to-repo).

Exit code: 0 clean / 1 violations / 2 setup error.
Use `--emit-allowlist` to print copy-pasteable baseline entries.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

# repo (table group) -> owning service. Mirrors design doc §5 five-service map.
# NOTE: starting assignment, reviewable. The first --emit-allowlist run will
# reveal where caller locations disagree; those become allowlist entries or a
# corrected mapping. The mechanism — not perfect classification — is the point.
REPO_OWNER: dict[str, str] = {
    # recon — asset / attack surface
    "targets": "recon",
    "target_assets": "recon",
    "organizations": "recon",
    "api_endpoints": "recon",
    "sitemap_store": "recon",
    "directory_entries": "recon",
    "fingerprints": "recon",
    "js_analysis": "recon",
    "passive_scans": "recon",
    "sensitive_scan": "recon",
    "screenshots": "recon",
    "custom_rules": "recon",
    "endpoint_tests": "recon",
    # vuln — vulnerability intelligence
    "vuln_intel": "vuln",
    "vuln_scan": "vuln",
    "scan_queue": "vuln",
    "wiki_kb": "vuln",
    "kb_research": "vuln",
    # pentest — engine / pipeline / findings
    "findings": "pentest",
    "methodology": "pentest",
    "pipelines": "pentest",
    "stage_runs": "pentest",
    "execution_plans": "pentest",
    "evidence_classifications": "pentest",
    "operation_state": "pentest",
    "sprint_contracts": "pentest",
    # agent — LLM orchestration / sessions (session -> task -> subtask -> tool_call)
    "sessions": "agent",
    "tasks": "agent",
    "subtasks": "agent",
    "conversation_store": "agent",
    "message_chains": "agent",
    "agent_logs": "agent",
    "tool_calls": "agent",
    "sub_agent_dispatches": "agent",
    "memories": "agent",
    "msg_logs": "agent",
    "prompt_templates": "agent",
    "vector_store_logs": "agent",
    "search_logs": "agent",
    # platform — vault / notes / os logs
    "vault": "platform",
    "notes": "platform",
    "terminal_logs": "platform",
}

# Cross-cutting repos any service may use (evidence ledger + generic SQL
# helper). Not owned by a single service.
SHARED_REPOS: frozenset[str] = frozenset({"audit", "scoped"})

# Ordered (first-match-wins) caller-path-prefix -> service domain.
# Paths are relative to backend/crates/golish/src/.
DOMAIN_RULES: list[tuple[str, str]] = [
    ("tools/asset_intel", "recon"),
    ("tools/organizations", "recon"),
    ("tools/targets", "recon"),
    ("tools/custom_rules", "recon"),
    ("tools/sensitive_scan", "recon"),
    ("tools/scan_runner", "recon"),
    ("tools/scan_queue", "recon"),
    ("tools/intel_providers", "recon"),
    ("tools/integrations", "recon"),
    ("tools/pentest_bridge", "pentest"),
    ("tools/pentest_ai", "pentest"),
    ("tools/pentest", "pentest"),
    ("tools/findings", "pentest"),
    ("tools/methodology", "pentest"),
    ("tools/pipeline", "pentest"),
    ("tools/execution_plans", "pentest"),
    ("tools/evidence", "pentest"),
    ("tools/security_analysis", "pentest"),
    ("tools/vuln_intel", "vuln"),
    ("tools/wiki", "vuln"),
    ("tools/conversation_store", "agent"),
    ("ai/", "agent"),
    ("tools/vault", "platform"),
    ("tools/audit", "platform"),
    ("tools/notes", "platform"),
    ("tools/recordings", "platform"),
]

# Baseline coupling frozen as a ratchet — seed via `--emit-allowlist` (Task 2).
# Each tuple = (caller_file_relative_to_src, repo_name). REMOVING an entry
# means you introduced the corresponding *Port (see design doc §6 S1-2).
ALLOWLIST: frozenset[tuple[str, str]] = frozenset(
    {
        ("ai/db_bridge/orchestration.rs", "execution_plans"),
        ("ai/db_bridge/recon.rs", "api_endpoints"),
        ("ai/db_bridge/recon.rs", "fingerprints"),
        ("ai/db_bridge/recon.rs", "js_analysis"),
        ("ai/db_bridge/recon.rs", "passive_scans"),
        ("ai/db_bridge/recon.rs", "target_assets"),
        ("ai/db_bridge/recon.rs", "vuln_intel"),
        ("ai/db_bridge/wiki.rs", "wiki_kb"),
        ("tools/audit.rs", "agent_logs"),
        ("tools/audit.rs", "passive_scans"),
        ("tools/audit.rs", "search_logs"),
        ("tools/pentest_bridge/auth_probe.rs", "targets"),
        ("tools/pentest_bridge/auth_probe.rs", "vault"),
        ("tools/pentest_bridge/js_collect/sitemap.rs", "sitemap_store"),
        ("tools/pentest_bridge/js_collect/tool_impl.rs", "js_analysis"),
        ("tools/pentest_bridge/js_collect/tool_impl.rs", "targets"),
        ("tools/pentest_bridge/js_extract_apis.rs", "js_analysis"),
        ("tools/pentest_bridge/js_extract_apis.rs", "targets"),
        ("tools/pentest_bridge/record_finding.rs", "targets"),
        ("tools/pentest_bridge/vault_ops.rs", "vault"),
        ("tools/pipeline/storage.rs", "directory_entries"),
        ("tools/pipeline/storage.rs", "sitemap_store"),
        ("tools/pipeline/storage.rs", "targets"),
        ("tools/scan_queue.rs", "scan_queue"),
        ("tools/security_analysis.rs", "api_endpoints"),
        ("tools/security_analysis.rs", "fingerprints"),
        ("tools/security_analysis.rs", "js_analysis"),
        ("tools/security_analysis.rs", "passive_scans"),
        ("tools/security_analysis.rs", "target_assets"),
        ("tools/vuln_intel/commands/matching.rs", "targets"),
    }
)

# These files still hold raw sqlx (P0-3 作用域 SQL 下沉 will drain them). The
# ratchet blocks NEW raw-sql files; existing ones are tracked here, not fixed
# by S1-1. See docs/design/2026-05-29-architecture-optimization.md P0-3.
RAW_SQL_ALLOWLIST: frozenset[str] = frozenset(
    {
        "ai/session_bridge.rs",
        "ai/tracking_bridge/chain.rs",
        "ai/tracking_bridge/memory.rs",
        "ai/tracking_bridge/records.rs",
        "projects/commands.rs",
        "tools/asset_intel/runtime/mod.rs",
        "tools/audit.rs",
        "tools/conversation_store/batch.rs",
        "tools/conversation_store/mod.rs",
        "tools/custom_rules.rs",
        "tools/evidence.rs",
        "tools/intel_providers.rs",
        "tools/pentest_bridge/auth_probe.rs",
        "tools/pentest_bridge/js_collect/sitemap.rs",
        "tools/pentest_bridge/record_finding.rs",
        "tools/pentest_bridge/run_pipeline.rs",
        "tools/pentest_bridge/vault_ops.rs",
        "tools/pipeline/storage.rs",
        "tools/project_io.rs",
        "tools/recordings.rs",
        "tools/scan_queue.rs",
        "tools/sensitive_scan.rs",
        "tools/targets/cmds.rs",
        "tools/targets/db.rs",
        "tools/targets/directory.rs",
        "tools/vuln_intel/commands/feeds.rs",
        "tools/vuln_intel/commands/fetching.rs",
        "tools/vuln_intel/commands/matching.rs",
        "tools/vuln_intel/commands/search.rs",
        "tools/wiki/vuln_links.rs",
    }
)

REPO_USE_RE = re.compile(r"golish_db::repo::([a-z_][a-z0-9_]*)")
RAW_SQL_RE = re.compile(r"\bsqlx::query")
PUB_MOD_RE = re.compile(r"^pub mod ([a-z_][a-z0-9_]*);", re.MULTILINE)

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "backend" / "crates" / "golish" / "src"
REPO_MOD = ROOT / "backend" / "crates" / "golish-db" / "src" / "repo" / "mod.rs"


def domain_of(rel: str) -> str | None:
    for prefix, dom in DOMAIN_RULES:
        if rel.startswith(prefix):
            return dom
    return None


def is_test_file(rel: str) -> bool:
    return rel.endswith("_tests.rs") or rel.endswith("tests.rs") or "/tests/" in rel


def declared_repos() -> set[str]:
    return set(PUB_MOD_RE.findall(REPO_MOD.read_text()))


def scan() -> tuple[list[str], list[str], set[tuple[str, str]], set[str]]:
    own_viol: list[str] = []
    raw_viol: list[str] = []
    emit_own: set[tuple[str, str]] = set()
    emit_raw: set[str] = set()
    for path in sorted(SRC.rglob("*.rs")):
        rel = str(path.relative_to(SRC))
        if is_test_file(rel):
            continue
        text = path.read_text()
        dom = domain_of(rel)
        for m in REPO_USE_RE.finditer(text):
            repo = m.group(1)
            if repo in SHARED_REPOS:
                continue
            owner = REPO_OWNER.get(repo)
            if owner is None:
                own_viol.append(f"{rel}: uses unregistered repo `{repo}` — add to REPO_OWNER")
                continue
            if dom is None:
                own_viol.append(f"{rel}: caller path has no domain — add a DOMAIN_RULES prefix")
                continue
            if owner == dom or (rel, repo) in ALLOWLIST:
                continue
            own_viol.append(f"{rel}: {dom} -> repo::{repo} (owned by {owner})")
            emit_own.add((rel, repo))
        if RAW_SQL_RE.search(text) and rel not in RAW_SQL_ALLOWLIST:
            raw_viol.append(f"{rel}: raw sqlx::query in command layer — route via golish-db repo")
            emit_raw.add(rel)
    return own_viol, raw_viol, emit_own, emit_raw


def main() -> int:
    if not SRC.is_dir() or not REPO_MOD.is_file():
        print(f"[repo-ownership] ERROR: paths not found ({SRC} / {REPO_MOD})", file=sys.stderr)
        return 2

    own, raw, emit_own, emit_raw = scan()
    for r in sorted(declared_repos() - set(REPO_OWNER) - SHARED_REPOS):
        own.append(f"golish-db repo `{r}` unregistered — add to REPO_OWNER or SHARED_REPOS")

    if "--emit-allowlist" in sys.argv:
        print("# --- paste into ALLOWLIST ---")
        for rel, repo in sorted(emit_own):
            print(f'        ("{rel}", "{repo}"),')
        print("# --- paste into RAW_SQL_ALLOWLIST ---")
        for rel in sorted(emit_raw):
            print(f'        "{rel}",')
        return 0

    if not own and not raw:
        print("[repo-ownership] OK clean")
        return 0
    print(
        f"[repo-ownership] FAIL {len(own)} ownership + {len(raw)} raw-sql violation(s):",
        file=sys.stderr,
    )
    for v in own + raw:
        print(f"  - {v}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
