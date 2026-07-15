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
Use `--baseline-ref <git-ref>` to fail only on exact violations that are new
relative to that ref, while evaluating both trees with THIS script's rules.
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import Protocol

# repo (table group) -> owning service. Mirrors design doc §5 five-service map.
# NOTE: starting assignment, reviewable. The first --emit-allowlist run will
# reveal where caller locations disagree; those become allowlist entries or a
# corrected mapping. The mechanism — not perfect classification — is the point.
REPO_OWNER: dict[str, str] = {
    # recon — asset / attack surface
    "targets": "recon",
    "target_assets": "recon",
    "dns_records": "recon",
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
    # scan_queue is the recon scan queue (sole user = golish-recon-app/
    # scan_queue.rs); reclassified vuln->recon (S1-2f ownership-map fix).
    "scan_queue": "recon",
    # vuln — vulnerability intelligence
    "vuln_intel": "vuln",
    "vuln_scan": "vuln",
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
    "runtime_memory_rollout": "agent",
    # Owns the runtime rollout admission/sample/promotion-receipt tables; the
    # admission and receipt writers themselves remain DB-trigger internal.
    "runtime_memory_shadow": "agent",
    "project_scopes": "agent",
    "operation_scope_decisions": "agent",
    "operation_org_scope": "agent",
    "stage_run_units": "agent",
    "stage_worker_runs": "agent",
    "stage_deliverable_submissions": "agent",
    "canonical_fact_refs": "agent",
    "stage_handoffs": "agent",
    "runtime_memory_tx": "agent",
    "stage_episodes": "agent",
    "knowledge_assertions": "agent",
    "knowledge_documents": "agent",
    "knowledge_embeddings": "agent",
    "knowledge_outbox": "agent",
    "knowledge_graph": "agent",
    "operator_principals": "agent",
    # Candidate V2 runtime/harness state. The agent service owns immutable
    # manifest materialization, final-seal acceptance, lanes and attempts;
    # pentest owns human review and Finding terminal lineage.
    "attack_candidate_seeds": "agent",
    "attack_candidate_work_items": "agent",
    "attack_candidates": "agent",
    "attack_execution_lanes": "agent",
    "attack_execution_rollout": "agent",
    "attack_execution_shadow": "agent",
    "attack_fact_deltas": "agent",
    "attack_wave_consolidations": "agent",
    "attack_waves": "agent",
    "candidate_attempts": "agent",
    "verification_truth": "agent",
    "attack_candidate_approvals": "pentest",
    "finding_lineage": "pentest",
    # Post-Exploit C4/P6a canonical spine. The dedicated app crate owns these
    # repos; access to runtime scope/evidence is encapsulated inside repo
    # compound methods rather than issued as command-layer SQL.
    "foothold_candidates": "post_exploit",
    "footholds": "post_exploit",
    "internal_asset_observations": "post_exploit",
    "attack_paths": "post_exploit",
    "post_exploit_actions": "post_exploit",
    "post_exploit_approvals": "post_exploit",
    "objective_attempts": "post_exploit",
    # Cleanup P7a retained obligation/attempt/absence/waiver ledger.
    "cleanup_obligations": "cleanup",
    "cleanup_attempts": "cleanup",
    "cleanup_absence_checks": "cleanup",
    "cleanup_waivers": "cleanup",
    "organization_deletion_jobs": "cleanup",
    # Reporting owns immutable report aggregates, frozen source manifests,
    # cited claims and content-addressed artifact references.
    "reports": "reporting",
    "report_revisions": "reporting",
    "report_sections": "reporting",
    "report_claims": "reporting",
    "report_claim_citations": "reporting",
    "report_source_manifest": "reporting",
    "report_artifact_blobs": "reporting",
    "report_revision_artifacts": "reporting",
    # platform — vault / notes / os logs
    "vault": "platform",
    "notes": "platform",
    "terminal_logs": "platform",
}

# Cross-cutting repos any service may use. Not owned by a single service:
#   audit  — OpenFang evidence ledger (every service appends).
#   scoped — generic project-scope SQL helper.
#   coverage_truth — harness coverage gate's READ-ONLY cross-table truth
#     projection (design 2026-06-12 §5.3). Aggregates organizations/targets/
#     target_assets/dns_records into "(asset × technique) has real data?" facts
#     for the harness (an orchestration concern spanning all services), mirroring
#     how `audit` is a cross-cutting ledger rather than a single service's CRUD.
#   engagement_truth — engagement overview's READ-ONLY cross-table truth
#     projection (design 2026-06-13-engagement-scoping-fanout §6.4): per-org
#     weakness counts + org-tree rows for the snapshot/scheduler oracle. Same
#     orchestration-spanning nature as coverage_truth.
SHARED_REPOS: frozenset[str] = frozenset(
    {"audit", "scoped", "coverage_truth", "engagement_truth"}
)

# Ordered (first-match-wins) caller-path-prefix -> service domain.
# Paths are relative to backend/crates/golish/src/.
DOMAIN_RULES: list[tuple[str, str]] = [
    # Provider-side port adapters belong to the service they expose (S1-2):
    # ports/platform/* legally reaches platform-owned repos (vault/notes/...).
    ("ports/platform", "platform"),
    # ports/recon/* (recon service adapters sunk into golish-app-core, S1-2b)
    # legally reach recon-owned repos (api_endpoints/js_analysis/fingerprints/
    # passive_scans/target_assets/targets/sitemap_store/directory_entries).
    ("ports/recon", "recon"),
    # ports/vuln/* (vuln service adapters in golish-app-core, S1-2c) legally
    # reach vuln-owned repos (vuln_intel / wiki_kb).
    ("ports/vuln", "vuln"),
    # ports/pentest/* (pentest service adapters in golish-app-core, S1-2d)
    # legally reach pentest-owned repos (execution_plans).
    ("ports/pentest", "pentest"),
    # ports/agent/* (agent service adapters in golish-app-core, S1-2e) legally
    # reach agent-owned repos (agent_logs / search_logs).
    ("ports/agent", "agent"),
    # NB: the recon command modules (targets / organizations / asset_intel /
    # scan_* / custom_rules / sensitive_scan / intel_providers / integrations)
    # were extracted to the golish-recon-app crate (crate-per-service M2); that
    # crate is scanned via SOURCE_ROOTS with a fixed `recon` domain, so no
    # per-path rule is needed here anymore.
    # NB: the pentest command modules (pentest / pentest_ai / pentest_bridge /
    # findings / methodology / pipeline / execution_plans / evidence /
    # security_analysis) were extracted to the golish-pentest-app crate
    # (crate-per-service M3); that crate is scanned via SOURCE_ROOTS with a fixed
    # `pentest` domain, so no per-path rule is needed here anymore.
    ("tools/vuln_intel", "vuln"),
    ("tools/wiki", "vuln"),
    # NB: the agent command surface (ai/* command bodies + bridges) and the
    # agent-owned conversation_store were extracted to the golish-agent-app crate
    # (crate-per-service M4-proper); that crate is scanned via SOURCE_ROOTS with a
    # fixed `agent` domain, so no per-path rule is needed here anymore.
    # NB: the platform command surface (vault / audit / notes / recordings) was
    # extracted to the golish-platform-app crate (crate-per-service M5); that crate
    # is scanned via SOURCE_ROOTS with a fixed `platform` domain, so no per-path
    # rule is needed here anymore.
]

# Baseline coupling frozen as a ratchet — seed via `--emit-allowlist` (Task 2).
# Each tuple = (caller_file_relative_to_src, repo_name). REMOVING an entry
# means you introduced the corresponding *Port (see design doc §6 S1-2).
ALLOWLIST: frozenset[tuple[str, str]] = frozenset(
    {
        # Cleanup C5/C8 consumes the C0 server-owned principal and the C6
        # post-exploit runtime authorizer behind its own application port. The
        # model-facing pentest adapter never reaches either repo directly.
        ("golish-cleanup-app/ports.rs", "operator_principals"),
        ("golish-cleanup-app/ports.rs", "post_exploit_actions"),
        # agent command surface extracted to the golish-agent-app crate
        # (crate-per-service M4-proper); keys are crate-prefixed (see SOURCE_ROOTS).
        # The db_bridge implements golish-agent-kit's DbRepoProvider, reading
        # cross-service recon/vuln/pentest tables (layer A; ReconPort/etc. cut to B).
        # NB: orchestration.rs's execution_plans read/write now routes through the
        # pentest service port (golish-app-core/ports/pentest, S1-2d), so its
        # ALLOWLIST entry was removed.
        # NB: recon.rs's 5 recon-table reads/writes (api_endpoints / fingerprints
        # / js_analysis / passive_scans / target_assets) now route through the
        # recon service ports (golish-app-core/ports/recon, S1-2b1), so their
        # ALLOWLIST entries were removed (ratchet net-forward).
        # NB: agent db_bridge's vuln reads (recon.rs → vuln_intel, wiki.rs →
        # wiki_kb) now route through the vuln service ports (golish-app-core/
        # ports/vuln, S1-2c), so their ALLOWLIST entries were removed.
        # platform service extracted to the golish-platform-app crate
        # (crate-per-service M5); keys are crate-prefixed (see SOURCE_ROOTS).
        # NB: audit.rs's agent_logs/search_logs reads now route through the agent
        # service port (golish-app-core/ports/agent, S1-2e); the recon
        # passive_scans read moved to the recon port (S1-2b5). All removed.
        # NB: pentest's recon-table reads/writes — pentest_bridge (anonymous access /
        # record_finding / browser_collect_js_api / js_extract_apis,
        # targets+sitemap_store+js_analysis) + pipeline/storage.rs
        # (targets+sitemap_store+directory_entries) — now route through the recon
        # service ports (golish-app-core/ports/recon, S1-2b3/b4), so their
        # ALLOWLIST entries were removed (ratchet net-forward).
        # NB: security_analysis.rs's 5 recon-table reads now route through the
        # recon service ports (golish-app-core/ports/recon, S1-2b2), so their
        # ALLOWLIST entries were removed (ratchet net-forward).
        # NB: matching.rs's recon `targets` read now routes through the recon
        # service port (golish-app-core/ports/recon, S1-2b6), so its ALLOWLIST
        # entry was removed (ratchet net-forward).
        # NB: scan_queue is now recon-owned (S1-2f false-positive fix), so
        # recon-app/scan_queue.rs reading it is own-domain — ALLOWLIST entry
        # removed. The cross-service ratchet is now EMPTY: every horizontal
        # repo coupling flows through a service port (S1-2 a–f complete).
    }
)

# These files still hold raw sqlx (P0-3 作用域 SQL 下沉 will drain them). The
# ratchet blocks NEW raw-sql files; existing ones are tracked here, not fixed
# by S1-1. See docs/design/2026-05-29-architecture-optimization.md P0-3.
RAW_SQL_ALLOWLIST: frozenset[str] = frozenset(
    {
        # agent service extracted to the golish-agent-app crate (crate-per-service
        # M4-proper); keys are crate-prefixed (see SOURCE_ROOTS).
        "golish-agent-app/ai/session_bridge.rs",
        "golish-agent-app/ai/tracking_bridge/chain.rs",
        "golish-agent-app/ai/tracking_bridge/memory.rs",
        "golish-agent-app/ai/tracking_bridge/records.rs",
        # C9 planned read-only adapter for frozen scope/source projections.
        "golish-agent-app/ai/db_bridge/reporting.rs",
        "golish-agent-app/conversation_store/batch.rs",
        "golish-agent-app/conversation_store/mod.rs",
        "projects/commands.rs",
        "tools/project_io.rs",
        # platform service extracted to the golish-platform-app crate (M5).
        "golish-platform-app/audit.rs",
        "golish-platform-app/recordings.rs",
        # pentest service extracted to the golish-pentest-app crate
        # (crate-per-service M3); keys are crate-prefixed (see SOURCE_ROOTS).
        "golish-pentest-app/evidence.rs",
        "golish-pentest-app/pentest_bridge/record_finding.rs",
        "golish-pentest-app/pentest_bridge/run_pipeline.rs",
        "golish-pentest-app/pentest_bridge/vault_ops.rs",
        "golish-pentest-app/pipeline/storage.rs",
        # recon extracted to the golish-recon-app crate (crate-per-service M2a/b);
        # keys are crate-prefixed (see SOURCE_ROOTS).
        "golish-recon-app/asset_intel/runtime/mod.rs",
        "golish-recon-app/custom_rules.rs",
        "golish-recon-app/intel_providers.rs",
        "golish-recon-app/scan_queue.rs",
        "golish-recon-app/sensitive_scan.rs",
        "golish-recon-app/targets/cmds.rs",
        # NB: targets/db.rs + targets/directory.rs raw sqlx sank to
        # golish_db::repo::{targets,directory_entries} (S1-3 sibling-dep cut), so
        # their RAW_SQL_ALLOWLIST entries were removed (ratchet net-forward).
        # vuln-intel + wiki extracted to the golish-vuln-app crate
        # (crate-per-service M1); keys are crate-prefixed (see SOURCE_ROOTS).
        "golish-vuln-app/wiki/vuln_links.rs",
        "golish-vuln-app/vuln_intel/commands/feeds.rs",
        "golish-vuln-app/vuln_intel/commands/fetching.rs",
        "golish-vuln-app/vuln_intel/commands/matching.rs",
        "golish-vuln-app/vuln_intel/commands/search.rs",
    }
)

REPO_USE_RE = re.compile(r"golish_db::repo::([a-z_][a-z0-9_]*)")
RAW_SQL_RE = re.compile(r"\bsqlx::query")
PUB_MOD_RE = re.compile(r"^pub mod ([a-z_][a-z0-9_]*);", re.MULTILINE)
FINDING_INSERT_RE = re.compile(
    r"\binsert\s+into\s+(?:public\.)?findings\b", re.IGNORECASE
)

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "backend" / "crates" / "golish" / "src"
REPO_MOD = ROOT / "backend" / "crates" / "golish-db" / "src" / "repo" / "mod.rs"
REPO_MOD_REL = "backend/crates/golish-db/src/repo/mod.rs"

# Command-layer source roots to scan (crate name, fixed_domain).
# The god-crate `golish` uses per-path DOMAIN_RULES (fixed_domain=None); each
# extracted per-service app crate owns exactly one service, so its whole src
# tree maps to a fixed domain. ALLOWLIST / RAW_SQL_ALLOWLIST keys for app crates
# are prefixed with the crate name (e.g.
# `golish-vuln-app/vuln_intel/commands/matching.rs`) to stay unambiguous; the
# god-crate keeps its historical golish/src-relative keys.
SOURCE_ROOTS: list[tuple[str, str | None]] = [
    ("golish", None),
    ("golish-vuln-app", "vuln"),
    ("golish-recon-app", "recon"),
    ("golish-pentest-app", "pentest"),
    ("golish-agent-app", "agent"),
    ("golish-platform-app", "platform"),
    ("golish-post-exploit-app", "post_exploit"),
    ("golish-cleanup-app", "cleanup"),
    ("golish-reporting-app", "reporting"),
    # app-core houses the shared VaultReadPort + PgVaultAdapter (sunk in M3 so the
    # pentest app can use them without depending on golish). Scan it with
    # path-relative domains so the adapter's golish_db::repo::vault call stays
    # guarded — `ports/platform/vault.rs` maps to platform via DOMAIN_RULES.
    ("golish-app-core", None),
]

# Candidate V2 has one production Finding authority. Legacy, scanner and
# command-layer callers must enter through this repository, while the exact
# CandidateAttempt terminalizer owns the compound Finding + lineage write.
# Migrations and test fixtures may seed rows directly; no other production
# source may contain a raw INSERT, even if it is not part of SOURCE_ROOTS.
FINDING_INSERT_ALLOWED: frozenset[str] = frozenset(
    {
        "backend/crates/golish-db/src/repo/findings.rs",
        "backend/crates/golish-db/src/repo/finding_lineage.rs",
    }
)


def domain_of(rel: str) -> str | None:
    for prefix, dom in DOMAIN_RULES:
        if rel.startswith(prefix):
            return dom
    return None


def is_test_file(rel: str) -> bool:
    return rel.endswith("_tests.rs") or rel.endswith("tests.rs") or "/tests/" in rel


class SnapshotError(RuntimeError):
    """The requested source snapshot cannot be read deterministically."""


class SourceSnapshot(Protocol):
    def read_text(self, relative: str) -> str | None: ...

    def iter_paths(self, prefix: str, suffixes: frozenset[str]) -> list[str]: ...


class WorktreeSnapshot:
    """Read files exactly as they exist in the current worktree, including untracked files."""

    def __init__(self, root: Path = ROOT) -> None:
        self.root = root

    def read_text(self, relative: str) -> str | None:
        path = self.root / relative
        if not path.is_file():
            return None
        return path.read_text()

    def iter_paths(self, prefix: str, suffixes: frozenset[str]) -> list[str]:
        base = self.root / prefix
        if not base.is_dir():
            return []
        return [
            path.relative_to(self.root).as_posix()
            for path in sorted(base.rglob("*"))
            if path.is_file() and path.suffix in suffixes
        ]


class GitRefSnapshot:
    """Read an immutable git tree without checking it out or mutating the worktree."""

    def __init__(self, root: Path, ref: str) -> None:
        if not ref or ref.startswith("-"):
            raise SnapshotError("--baseline-ref requires a non-option git ref")
        self.root = root
        self.requested_ref = ref
        self.commit = self._git("rev-parse", "--verify", f"{ref}^{{commit}}").strip()
        if not re.fullmatch(r"[0-9a-fA-F]{40,64}", self.commit):
            raise SnapshotError(f"git ref `{ref}` did not resolve to a commit")

    def _git(self, *args: str) -> str:
        result = subprocess.run(
            ["git", "-C", str(self.root), *args],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            detail = result.stderr.strip() or result.stdout.strip() or "git command failed"
            raise SnapshotError(detail)
        return result.stdout

    def read_text(self, relative: str) -> str | None:
        result = subprocess.run(
            ["git", "-C", str(self.root), "show", f"{self.commit}:{relative}"],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            return None
        return result.stdout

    def iter_paths(self, prefix: str, suffixes: frozenset[str]) -> list[str]:
        output = self._git(
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            self.commit,
            "--",
            prefix,
        )
        return sorted(
            path
            for path in output.split("\0")
            if path and PurePosixPath(path).suffix in suffixes
        )


def _required_text(snapshot: SourceSnapshot, relative: str) -> str:
    text = snapshot.read_text(relative)
    if text is None:
        raise SnapshotError(f"required path not found in snapshot: {relative}")
    return text


def declared_repos_from_snapshot(snapshot: SourceSnapshot) -> set[str]:
    return set(PUB_MOD_RE.findall(_required_text(snapshot, REPO_MOD_REL)))


def declared_repos() -> set[str]:
    return declared_repos_from_snapshot(WorktreeSnapshot())


def scan_snapshot(
    snapshot: SourceSnapshot,
) -> tuple[list[str], list[str], set[tuple[str, str]], set[str]]:
    own_viol: list[str] = []
    raw_viol: list[str] = []
    emit_own: set[tuple[str, str]] = set()
    emit_raw: set[str] = set()
    for crate, fixed_dom in SOURCE_ROOTS:
        prefix = f"backend/crates/{crate}/src"
        for path in snapshot.iter_paths(prefix, frozenset({".rs"})):
            rel = PurePosixPath(path).relative_to(prefix).as_posix()
            if is_test_file(rel):
                continue
            # God-crate keeps its golish/src-relative key; app crates are
            # crate-prefixed so allowlist keys stay unambiguous across crates.
            key = rel if crate == "golish" else f"{crate}/{rel}"
            text = _required_text(snapshot, path)
            dom = fixed_dom if fixed_dom is not None else domain_of(rel)
            for m in REPO_USE_RE.finditer(text):
                repo = m.group(1)
                if repo in SHARED_REPOS:
                    continue
                owner = REPO_OWNER.get(repo)
                if owner is None:
                    own_viol.append(f"{key}: uses unregistered repo `{repo}` — add to REPO_OWNER")
                    continue
                if dom is None:
                    own_viol.append(f"{key}: caller path has no domain — add a DOMAIN_RULES prefix")
                    continue
                if owner == dom or (key, repo) in ALLOWLIST:
                    continue
                own_viol.append(f"{key}: {dom} -> repo::{repo} (owned by {owner})")
                emit_own.add((key, repo))
            if RAW_SQL_RE.search(text) and key not in RAW_SQL_ALLOWLIST:
                raw_viol.append(f"{key}: raw sqlx::query in command layer — route via golish-db repo")
                emit_raw.add(key)
    return own_viol, raw_viol, emit_own, emit_raw


def scan() -> tuple[list[str], list[str], set[tuple[str, str]], set[str]]:
    return scan_snapshot(WorktreeSnapshot())


def scan_finding_insertions_snapshot(snapshot: SourceSnapshot) -> list[str]:
    violations: list[str] = []
    prefix = "backend/crates"
    for rel in snapshot.iter_paths(prefix, frozenset({".rs", ".sql"})):
        path = PurePosixPath(rel)
        parts = set(path.relative_to(prefix).parts)
        if parts.intersection({"migrations", "tests", "fixtures"}):
            continue
        if rel in FINDING_INSERT_ALLOWED:
            continue
        if FINDING_INSERT_RE.search(_required_text(snapshot, rel)):
            violations.append(
                f"{rel}: raw INSERT INTO findings bypasses the guarded Finding repository"
            )
    return violations


def scan_finding_insertions(root: Path = ROOT) -> list[str]:
    return scan_finding_insertions_snapshot(WorktreeSnapshot(root))


Violation = tuple[str, str]


def collect_violations(snapshot: SourceSnapshot) -> set[Violation]:
    """Return the deduplicated exact violation set under the current rules."""

    own, raw, _, _ = scan_snapshot(snapshot)
    for repo in sorted(
        declared_repos_from_snapshot(snapshot) - set(REPO_OWNER) - SHARED_REPOS
    ):
        own.append(
            f"golish-db repo `{repo}` unregistered — add to REPO_OWNER or SHARED_REPOS"
        )
    finding_writes = scan_finding_insertions_snapshot(snapshot)
    return {
        *(("ownership", violation) for violation in own),
        *(("raw-sql", violation) for violation in raw),
        *(("finding-write", violation) for violation in finding_writes),
    }


def compare_violation_sets(
    current: set[Violation], baseline: set[Violation]
) -> tuple[set[Violation], set[Violation]]:
    """Return (added, removed); only `added` is gate-failing."""

    return current - baseline, baseline - current


def _parse_mode(argv: list[str]) -> tuple[str, str | None]:
    if not argv:
        return "full", None
    if argv == ["--finding-writes-only"]:
        return "finding-writes-only", None
    if argv == ["--emit-allowlist"]:
        return "emit-allowlist", None
    if len(argv) == 2 and argv[0] == "--baseline-ref":
        return "baseline-ref", argv[1]
    raise SnapshotError(
        "usage: check_repo_ownership.py [--emit-allowlist | "
        "--finding-writes-only | --baseline-ref <git-ref>]"
    )


def main(argv: list[str] | None = None, *, root: Path = ROOT) -> int:
    try:
        mode, baseline_ref = _parse_mode(list(sys.argv[1:] if argv is None else argv))
        src = root / "backend" / "crates" / "golish" / "src"
        repo_mod = root / REPO_MOD_REL
        if not src.is_dir() or not repo_mod.is_file():
            raise SnapshotError(f"paths not found ({src} / {repo_mod})")
        worktree = WorktreeSnapshot(root)

        if mode == "baseline-ref":
            assert baseline_ref is not None
            current = collect_violations(worktree)
            baseline_snapshot = GitRefSnapshot(root, baseline_ref)
            baseline = collect_violations(baseline_snapshot)
            added, removed = compare_violation_sets(current, baseline)
            if added:
                print(
                    f"[repo-ownership] FAIL {len(added)} new exact violation(s) "
                    f"vs {baseline_ref} (current={len(current)}, "
                    f"baseline={len(baseline)}, removed={len(removed)}):",
                    file=sys.stderr,
                )
                for category, violation in sorted(added):
                    print(f"  - [{category}] {violation}", file=sys.stderr)
                return 1
            print(
                f"[repo-ownership] OK no new exact violations vs {baseline_ref} "
                f"(current={len(current)}, baseline={len(baseline)}, "
                f"removed={len(removed)}; historical violations not asserted clean)"
            )
            return 0

        finding_writes = scan_finding_insertions_snapshot(worktree)
        own, raw, emit_own, emit_raw = scan_snapshot(worktree)
        for repo in sorted(
            declared_repos_from_snapshot(worktree) - set(REPO_OWNER) - SHARED_REPOS
        ):
            own.append(
                f"golish-db repo `{repo}` unregistered — add to REPO_OWNER or SHARED_REPOS"
            )
    except SnapshotError as exc:
        print(f"[repo-ownership] ERROR: {exc}", file=sys.stderr)
        return 2

    if mode == "finding-writes-only":
        if not finding_writes:
            print("[repo-ownership] OK Finding write authority clean")
            return 0
        print(
            f"[repo-ownership] FAIL {len(finding_writes)} Finding write authority violation(s):",
            file=sys.stderr,
        )
        for violation in finding_writes:
            print(f"  - {violation}", file=sys.stderr)
        return 1

    if mode == "emit-allowlist":
        print("# --- paste into ALLOWLIST ---")
        for rel, repo in sorted(emit_own):
            print(f'        ("{rel}", "{repo}"),')
        print("# --- paste into RAW_SQL_ALLOWLIST ---")
        for rel in sorted(emit_raw):
            print(f'        "{rel}",')
        return 0

    if not own and not raw and not finding_writes:
        print("[repo-ownership] OK clean")
        return 0
    print(
        f"[repo-ownership] FAIL {len(own)} ownership + {len(raw)} raw-sql + "
        f"{len(finding_writes)} Finding-write violation(s):",
        file=sys.stderr,
    )
    for v in own + raw + finding_writes:
        print(f"  - {v}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
