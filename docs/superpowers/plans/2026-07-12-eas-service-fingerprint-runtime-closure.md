# EAS SERVICE-FINGERPRINT runtime closure implementation plan

> Design: `docs/design/2026-07-12-eas-service-fingerprint-runtime-closure.md`

## Task 1: Lock the sub-agent timeout contract

**Files**

- Modify `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`

**Steps**

1. Extend the existing timeout-policy unit test with all four guarded `eas_*`
   wrappers and run it RED.
2. Add the four wrapper names to the long-running direct-tool exemption.
3. Re-run the focused executor test GREEN and confirm an ordinary tool such as
   `query_target_data` remains timeout-bound.

## Task 2: Give service fingerprinting its own bounded command budget

**Files**

- Modify `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

**Steps**

1. Add RED tests proving an omitted timeout becomes 600 seconds and an explicit
   timeout is preserved.
2. Introduce a service-only run-args helper; keep `background=false` and
   `__foreground_only=true` unchanged.
3. Run the focused EAS capability tests GREEN.

## Task 3: Route guarded output by trusted tool identity

**Files**

- Modify `backend/crates/golish-pentest/src/output_store/mod.rs`
- Modify `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

**Steps**

1. Add a RED output-store regression using valid Nmap stdout whose banner
   contains a `HH:MM` timestamp that the broad Naabu detector can match.
2. Add optional trusted tool identity to `StoreContext` and load the matching
   toolsconfig deterministically when present.
3. Pass `wrapped_tool_name` from guarded EAS landing.
4. Assert the result reports `tool_id=nmap`, parses service/version records, and
   stores every parsed record.

## Task 4: Synchronize docs and verify

**Files**

- Update affected cards under `docs/modules/backend/`
- Update `docs/modules/INDEX.md`
- Update `agent-progress.md`
- Update the current `feature_list.json` entry

**Steps**

1. Record the timeout and trusted-output-routing contracts in the module cards.
2. Run focused nextest suites for `golish-sub-agents`, `golish-pentest`, and
   `golish-pentest-app`.
3. Run scoped Clippy, rustfmt, JSON parsing, and `git diff --check`.
4. Keep the feature `in_progress` until a fresh compiled EAS run proves
   SERVICE-FINGERPRINT rows, evidence, outcomes, and gate coverage land.
5. Do not run `./init.sh` or full `just precommit` under the user's existing
   validation constraint.
