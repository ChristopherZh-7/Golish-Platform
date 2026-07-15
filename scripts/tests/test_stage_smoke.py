import unittest

from scripts import stage_smoke


class StageSmokeRouteBudgetTests(unittest.TestCase):
    def parse(self, *args: str):
        return stage_smoke.build_parser().parse_args(list(args))

    def test_candidate_chain_from_scoping_includes_enumeration(self):
        args = self.parse(
            "--profile", "red_team", "--from", "scoping", "--to", "attack_candidate"
        )

        self.assertTrue(stage_smoke.includes_enumeration(args))
        self.assertEqual(
            stage_smoke.route_probe_budget(args),
            (
                stage_smoke.DEFAULT_SMOKE_ROUTE_PROBE_MAX_RUNTIME_MS,
                stage_smoke.DEFAULT_SMOKE_ROUTE_PROBE_MAX_REQUESTS,
            ),
        )

    def test_positional_candidate_defaults_to_scoping_and_includes_enumeration(self):
        args = self.parse("--profile", "red_team", "attack_candidate")

        self.assertTrue(stage_smoke.includes_enumeration(args))

    def test_slice_starting_after_enumeration_does_not_inject_budget(self):
        args = self.parse("--from", "vuln_triage", "--to", "attack_candidate")

        self.assertFalse(stage_smoke.includes_enumeration(args))
        self.assertIsNone(stage_smoke.route_probe_budget(args))

    def test_only_candidate_does_not_claim_enumeration(self):
        args = self.parse("--only", "attack_candidate")

        self.assertFalse(stage_smoke.includes_enumeration(args))

    def test_only_enumeration_keeps_the_existing_budget(self):
        args = self.parse("--only", "enumeration")

        self.assertTrue(stage_smoke.includes_enumeration(args))


if __name__ == "__main__":
    unittest.main()
