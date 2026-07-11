# Enumeration trusted transport preflight implementation plan

> Implements `docs/design/2026-07-10-enumeration-trusted-transport-preflight.md`.

1. Add the batch direct tool with active session/operation/org/workspace/exact-
   origin authorization, pre/post target witness checks, finite direct/proxy
   HEAD→GET-Range transport attempts, permissive certificate policy matching the
   real producers, scheme-aware environment proxy + NO_PROXY handling, and stable
   ordered result groups.
2. Capture the operation epoch and allocate a per-origin generation. In one short
   target/operation-guarded transaction, verify the epoch and atomically publish
   four generated partial markers before I/O. On proven transport failure,
   prepare four target-bound evidence rows and use a second short CAS transaction
   to replace the group with blocked outcomes only if epoch + generation remain
   current. Never hold a DB transaction across external I/O.
3. Add `EvidenceOutcome::Blocked`; require matching fresh target-bound evidence
   in the Enumeration read model and org gate, while keeping error/partial
   nonterminal.
4. Disable non-empty `terminal_exceptions`, reject non-empty Enumeration submit
   coverage, and update schema, taxonomy, capabilities, repair fences, prompts,
   methodology, tool selection, and module cards.
5. Add regression coverage for any-HTTP-response reachability (including
   self-signed and hostname-mismatched HTTPS 4xx/5xx), strict inputs, HTTP/HTTPS/
   ALL proxy selection and NO_PROXY propagation, stable result ordering,
   operation restart rejection, A/B generation interleaving, all-four partial
   reset, evidence-matched blocked projection, safe batch/origin progress output,
   and rejection of model-authored terminal cells.
6. Run focused nextest/check/clippy/fmt/diff validation; live DB/run validation
   is performed by the parent Enumeration closeout task.
