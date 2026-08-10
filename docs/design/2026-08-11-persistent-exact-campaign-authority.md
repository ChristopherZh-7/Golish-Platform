# Persistent exact Campaign authority

## Problem

Investigation Campaign dispatch is held by default. The existing explicit
`stage_run_campaign_authority.v1` packet can release that hold and adjudicate an
exact Prepared Action, but the CLI accepts it only for retained ephemeral
PostgreSQL. A real operation in the local application database therefore
reaches a genuine authorization boundary with no non-UI local-operator path to
continue the same durable operation.

## Decision

Allow the existing packet on an exact Investigation resume against the local
application database as well as retained ephemeral PostgreSQL. Do not add a
second authority contract or a model/Tauri mutation surface.

The command still requires exact session, task, operation, organization and
stage selectors. It is applied only after the resume identity and advisory
claim are validated. Hold release remains generation/row-version CAS-bound and
append-only; Prepared Action decisions remain bound to the frozen private and
display hashes, renderer version, row version and stable request id. The local
operator principal must be active. `--auto-approve` grants none of this
authority.

## Acceptance

- clap accepts the packet without `--stage-run-resume-pgdata` only when every
  exact resume selector is present;
- a packet on any stage other than Investigation is rejected;
- malformed, symlinked, oversized, stale or identity-mismatched packets remain
  fail-closed;
- the real retained operation can release the held-by-default Campaign boundary
  through the canonical repository and continue from the same checkpoint.
