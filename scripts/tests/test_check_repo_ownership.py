import contextlib
import importlib.util
import io
import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "check_repo_ownership.py"
SPEC = importlib.util.spec_from_file_location("check_repo_ownership", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
ownership = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ownership)


class FindingWriteAuthorityRatchetTests(unittest.TestCase):
    def test_only_guarded_repository_files_may_insert_findings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def write(relative: str, text: str) -> None:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)

            for allowed in ownership.FINDING_INSERT_ALLOWED:
                write(allowed, "sqlx::query(\"INSERT INTO findings(id) VALUES ($1)\");")
            write(
                "backend/crates/golish-agent-app/src/bypass.rs",
                "sqlx::query(\"insert into public.findings(id) values ($1)\");",
            )
            write(
                "backend/crates/golish-agent-app/src/safe.rs",
                "golish_db::repo::findings::insert_guarded(input).await?;",
            )
            for ignored in [
                "backend/crates/golish-db/migrations/seed.sql",
                "backend/crates/golish-db/tests/fixture.rs",
                "backend/crates/golish-db/fixtures/finding.sql",
            ]:
                write(ignored, "INSERT INTO findings(id) VALUES ('fixture');")

            self.assertEqual(
                ownership.scan_finding_insertions(root),
                [
                    "backend/crates/golish-agent-app/src/bypass.rs: raw INSERT INTO "
                    "findings bypasses the guarded Finding repository"
                ],
            )


class BaselineRefRatchetTests(unittest.TestCase):
    def test_current_rules_report_only_exact_violations_added_since_git_ref(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def write(relative: str, text: str) -> None:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)

            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            write(
                "backend/crates/golish-db/src/repo/mod.rs",
                "pub mod targets;\n",
            )
            write("backend/crates/golish/src/lib.rs", "// fixture root\n")
            write(
                "backend/crates/golish-agent-app/src/legacy.rs",
                "golish_db::repo::targets::get();\n",
            )
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(root),
                    "-c",
                    "user.name=Golish Test",
                    "-c",
                    "user.email=golish-test@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "baseline",
                ],
                check=True,
            )
            baseline_ref = subprocess.run(
                ["git", "-C", str(root), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()

            write(
                "backend/crates/golish-agent-app/src/new_raw_sql.rs",
                'let _ = sqlx::query("SELECT 1");\n',
            )

            baseline = ownership.collect_violations(
                ownership.GitRefSnapshot(root, baseline_ref)
            )
            current = ownership.collect_violations(ownership.WorktreeSnapshot(root))
            added, removed = ownership.compare_violation_sets(current, baseline)

            self.assertIn(
                (
                    "ownership",
                    "golish-agent-app/legacy.rs: agent -> repo::targets "
                    "(owned by recon)",
                ),
                baseline,
            )
            self.assertEqual(
                added,
                {
                    (
                        "raw-sql",
                        "golish-agent-app/new_raw_sql.rs: raw sqlx::query in command "
                        "layer — route via golish-db repo",
                    )
                },
            )
            self.assertEqual(removed, set())

            stdout = io.StringIO()
            stderr = io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                exit_code = ownership.main(
                    ["--baseline-ref", baseline_ref], root=root
                )
            self.assertEqual(exit_code, 1)
            self.assertIn("new_raw_sql.rs", stderr.getvalue())
            self.assertNotIn("legacy.rs", stderr.getvalue())

            (root / "backend/crates/golish-agent-app/src/legacy.rs").unlink()
            current_after_cleanup = ownership.collect_violations(
                ownership.WorktreeSnapshot(root)
            )
            added_after_cleanup, removed_after_cleanup = ownership.compare_violation_sets(
                current_after_cleanup, baseline
            )
            self.assertEqual(added_after_cleanup, added)
            self.assertEqual(
                removed_after_cleanup,
                {
                    (
                        "ownership",
                        "golish-agent-app/legacy.rs: agent -> repo::targets "
                        "(owned by recon)",
                    )
                },
            )

            (root / "backend/crates/golish-agent-app/src/new_raw_sql.rs").unlink()
            stdout = io.StringIO()
            stderr = io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                exit_code = ownership.main(
                    ["--baseline-ref", baseline_ref], root=root
                )
            self.assertEqual(exit_code, 0)
            self.assertIn("historical violations not asserted clean", stdout.getvalue())
            self.assertEqual(stderr.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
