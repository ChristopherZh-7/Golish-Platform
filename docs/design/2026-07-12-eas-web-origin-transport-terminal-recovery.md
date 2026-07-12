# EAS Web-Origin Transport Terminal Recovery

Status: Implemented with focused verification; fresh live acceptance pending.

## Live failure

The latest continuation of `pentest-chat-1783829178322-1` reached 306/308 EAS
coverage cells. Two authoritative exact Web Origins remained:

- `http://222.186.129.58:80` — WhatWeb returned `end of file reached`;
- `https://43.248.77.17:443` — WhatWeb returned
  `Connection reset by peer - SSL_connect`.

Naabu/Nmap had already established open HTTP(S) service truth for both origins.
WhatWeb therefore attempted the correct authorized identities, but its non-zero
exit and stderr caused the wrapper to publish no WEB-FINGERPRINT evidence or
outcome. A later httpx `empty` row belonged to LIVENESS and could not close WEB.
Repeated continuation could only retry the same two cells.

## Decision

Keep the exact-origin gate strict and add a backend-authored target-side
transport terminal.

1. **Three negative meanings stay distinct.**
   - WhatWeb completed normally but found no stack: guarded `empty`.
   - WhatWeb emitted an exact, authorized `ERROR Opening: <origin> - <reason>`
     whose normalized reason is exactly EOF, bare connection reset, or the
     observed `connection reset by peer - SSL_connect`: guarded `blocked`.
   - Ruby/runtime/configuration failure, output truncation, unknown stderr,
     unbound origin, or persistence failure: non-terminal wrapper error/partial.
   `blocked` is not `checked_empty`, and neither is `found`.
2. **Attribution is exact and backend-owned.** Strip ANSI, parse only WhatWeb's
   `ERROR Opening:` grammar, canonicalize the embedded URL, and require an exact
   match to one current `{target_id,target_url}` authorization and the three-value
   EOF/reset reason allowlist. Scheme, host,
   port, organization, workspace and target ownership remain unchanged. Any
   unrecognized stderr line, duplicate/conflicting classification, or missing
   authorization keeps the batch fail-closed.
3. **Mixed batches preserve independent truth.** A successful stdout record
   remains eligible for normal structured landing and found/empty evidence. An
   attributed opening failure publishes only that origin's blocked evidence.
   Every requested origin must be represented by either a WhatWeb stdout record
   or an attributed opening failure, and at least one member must actually be an
   attributed blocked failure, before a non-zero mixed batch can be treated as
   terminal; otherwise no recovery projection is allowed.
4. **Wrapper success means durable terminal publication.** When all members are
   exactly classified and all guarded writes succeed, the wrapper returns
   `completion_state=complete`, `outcome_persisted=true`, and a bounded
   `terminal_blocked_origins` diagnostic while retaining the wrapped process
   exit/stderr separately. If any landing/evidence/outcome write fails, the
   wrapper remains non-terminal.
5. **Gate and preflight share the same strict fact.** EAS WEB may consume a
   producer-owned blocked exact-origin outcome only when its positive evidence
   id matches a fresh current-org target-bound audit fact with the same origin,
   technique, outcome and WhatWeb producer identity. Model-authored parent
   exceptions remain unable to close `details.missing_origins`.

## Invariants

- No schema, migration, generated IPC, or raw scanner exposure change.
- No nearby port, sibling origin, host-only fact, LIVENESS evidence or prose can
  close an exact WEB origin.
- A target-side opening refusal never becomes `empty` or `found`.
- Unknown/unattributed stderr and truncated output remain fail-closed.
- Current organization/workspace/scope/target-id/exact-origin guards stay in
  force immediately before evidence and outcome publication.

## Files

- `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`
  classifies WhatWeb output and publishes per-origin terminal outcomes.
- `backend/crates/golish-agent-kit/src/harness/gate/eas_web_origin_check.rs`
  accepts only matching guarded exact-origin blocked outcomes.
- `backend/crates/golish-agent-kit/src/harness/org_gate.rs` and
  `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs` project the
  trusted EAS WEB blocked fact without opening a model-authored bypass.
- `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs` and
  `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs` keep the
  read model and final gate in parity.

## Verification

Use RED-to-GREEN tests for exact error parsing, mixed-batch attribution,
unattributed stderr fail-closed behavior, strict evidence identity, exact-origin
barrier completion, and preflight terminal projection. Run the affected crate
suites, scoped Clippy, rustfmt, JSON parsing and diff checks. A fresh live EAS
continuation remains the final product acceptance step and must be initiated by
the user after the backend reloads.
