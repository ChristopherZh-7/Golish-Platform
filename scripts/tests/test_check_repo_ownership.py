import importlib.util
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


if __name__ == "__main__":
    unittest.main()
