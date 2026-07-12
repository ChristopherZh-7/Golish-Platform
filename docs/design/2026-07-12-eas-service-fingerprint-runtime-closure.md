# EAS SERVICE-FINGERPRINT runtime closure

## Status

Implemented with focused verification for the current
`scoping-session-lifecycle-contract-2026-07-12` feature after live run
`pentest-chat-1783826001542-1` exposed two independent runtime failures. Fresh
compiled EAS acceptance remains pending. This follow-up does not change
authorization scope, DB schema, IPC types, or the EAS stage denominator.

## Problem

The current Prober has `idle_timeout_secs=300`. The sub-agent dispatcher reuses
that idle budget as a wall-clock timeout for every ordinary tool call. Guarded
EAS wrappers are not exempt, so a valid `eas_fingerprint_services` call is
dropped after exactly 300 seconds even while its sequential `nmap -sV` batches
are still making progress.

Dropping the wrapper future is unsafe for the stage contract. The underlying
foreground process is owned by the process-wide job manager and may continue
until its own hard deadline, but the dropped wrapper can no longer complete the
authorized landing, evidence append, or technique-outcome publication.

The same live run also showed valid Nmap stdout being classified as `naabu`.
Generic output detection scans broad regexes in filesystem order; the Naabu
`host:port` detector can match the time in Nmap's banner before the Nmap config
is selected. The guarded wrapper then reports `wrapped output was not recognized
as structured EAS data` even though the output contains real service/version
rows.

## Contract

1. `eas_probe_http_liveness`, `eas_discover_ports`,
   `eas_fingerprint_services`, and `eas_fingerprint_web_stack` remain
   foreground guarded capabilities. They return only after business rows,
   evidence, and terminal outcomes are published or an honest partial/error is
   returned.
2. The generic sub-agent idle timeout must not cap these wrappers. User Stop and
   the wrapper/runner's own bounded command timeout remain authoritative.
3. Service fingerprinting gets a service-specific default per-batch command
   budget of 600 seconds, while an explicit smaller caller value remains
   honored. The existing runner cap remains the final upper bound.
4. A guarded wrapper must select its output config from the known
   `wrapped_tool_name`; heuristic regex detection is only a compatibility
   fallback for callers without trusted tool identity.
5. Each completed batch keeps its existing guarded landing semantics. This
   slice does not introduce background execution or broaden scan authorization.

## Design

### Sub-agent timeout boundary

Extend the existing direct long-running-tool exemption to all four `eas_*`
wrappers. The surrounding `tokio::select!` still observes the shared
`cancelled` flag, so Stop remains immediate without a fixed wall-clock cutoff.

### Service command budget

Keep the generic pentest runner default unchanged. Only
`eas_fingerprint_services` injects `timeout_secs=600` when the caller omitted
it. This avoids changing unrelated tools while giving Nmap version detection a
realistic bounded budget.

### Deterministic output routing

Add an optional trusted tool identity to `StoreContext`. When present, output
store loads that exact toolsconfig entry and never guesses another tool from
stdout. EAS guarded landing passes `wrapped_tool_name` through this field.
Callers without trusted identity retain current command/output detection.

## Failure semantics

- Internal Nmap timeout remains `partial/error`; it is never converted to
  checked-empty.
- Missing or invalid exact toolsconfig fails closed as unrecognized output.
- Parser/storage/evidence failure keeps `outcome_persisted=false`.
- Cancellation stops the waiting wrapper; no synthetic success or evidence is
  produced.

## Verification

- Sub-agent regression proves every guarded EAS wrapper bypasses the generic
  outer tool timeout while ordinary tools still use it.
- EAS capability regression proves service calls default to 600 seconds,
  preserve an explicit timeout, and remain foreground-only.
- Output-store regression feeds an Nmap banner containing a clock timestamp and
  proves trusted identity selects Nmap, parses service/version rows, and never
  selects Naabu.
- Focused crate tests, Clippy, rustfmt, JSON validation, and `git diff --check`
  must pass. Per the user's existing constraint, do not run `./init.sh` or the
  full `just precommit` in this slice.
