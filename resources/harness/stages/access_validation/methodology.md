# Access Validation methodology (C6 / P6b)

## Contract

Access Validation converts exactly one server-owned `candidate_attempt/<uuid>` or
`foothold_candidate/<uuid>` work item into an evidence-backed Foothold. It never
starts from `foothold/<uuid>` and never accepts free-form target, credential, or
actor identity from model text.

## P6b capability boundary

- The only callable business action is `post_exploit_validate_access`; the operation, project scope, organization, stage unit, worker lease, and tool-call fence come from trusted runtime context and are reloaded from DB.
- The wrapper is foreground-only and accepts no operation ID, lease token, background flag, raw command, exploit recipe, or actor identity.
- `VaultCredentialRef` is an opaque UUID whose frozen project ownership is rechecked by the repository; secret material never enters the deliverable or memory event.

## Canonical result

The repository atomically writes Foothold + exact ledger evidence +
`PostExploitFactTerminal.v1` outbox deliveries. A claimed success without those
rows is not a completed work item. Checked-empty or rejected input remains an
explicit typed terminal decision; prose is not authority.
