# Target Intel organization-only submit parity

## Problem

The live run `pentest-chat-1783794304742-1` reached Target Intel after Scoping
passed, but `check_stage_asset_coverage` reported `ready_to_submit=true` while
`submit_stage_deliverable` repeatedly returned `needs_fix`.

An organization-only engagement has no executable `targets` rows. The coverage
read model and final per-organization gate synthesize a non-executable
`organization:<uuid>` row, but the submit preview did not. It therefore lost the
authoritative denominator, skipped organization WHOIS/ASN/OSINT DB truth, and
fell back to model-authored coverage. In the final gate, an organization-wide
provider error could also reopen DNS/CT/SUBDOMAIN cells that the backend had
deterministically marked not applicable on the organization row.

## Contract

- `organization:<uuid>` is the only coverage identity for organization context.
- The organization row owns WHOIS, ASN, and OSINT.
- DNS, CT, and SUBDOMAIN are deterministic `not_applicable` on that row; they
  remain required on applicable executable target rows.
- Submit preview and final per-org gate construct the identity, type, and N/A
  cells through one shared helper.
- A trusted exact `(asset, technique)` N/A may suppress a source-query error for
  that same structurally inapplicable cell. It must not suppress an exact
  evidence `Error`, nor any error for another asset or applicable cell.
- A slim Target Intel deliverable may keep `coverage: []`; Found truth remains
  database/ledger-authoritative.

## Implementation seams

- `golish-agent-kit::harness::org_gate::TargetIntelOrganizationContext` owns the
  canonical key, typed-axis injection, and deterministic N/A projection.
- `SubmitStageDeliverableTool::gate_context` injects that row before querying DB
  truth, including when the real target axis is empty.
- `coverage_complete` gives the trusted exact N/A pair precedence only over the
  source-query-derived nonterminal marker. Existing evidence-error and
  cross-asset fail-closed behavior stays intact.

## Validation

TDD covers the organization-only submit preview, the source-error/N/A conflict,
and a mismatched-asset negative guard. No schema, migration, generated IPC type,
provider call, or active scan is involved.
