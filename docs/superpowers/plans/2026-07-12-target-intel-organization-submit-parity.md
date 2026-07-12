# Target Intel organization-only submit parity plan

1. Add a failing submit-preview regression with zero target rows, organization
   WHOIS/ASN/OSINT DB facts, and a slim `coverage: []` deliverable.
2. Add a failing rule-engine regression for an exact deterministic organization
   N/A cell combined with an organization-wide source error, plus a cross-asset
   negative guard.
3. Extract the canonical organization-context identity/axis/N/A projection into
   `TargetIntelOrganizationContext` and reuse it from the final gate and submit
   preview.
4. Query submit-preview DB truth only after the organization row is present.
5. Suppress only source-query-derived nonterminal markers for a matching trusted
   N/A pair; preserve exact evidence errors and applicable-asset errors.
6. Run focused `golish-agent-kit` and `golish-agent-app` tests, scoped Clippy,
   Rust formatting, JSON validation, and `git diff --check`. Per user direction,
   do not run `./init.sh` or the broad precommit suite.
7. After the application reloads the new backend, verify a fresh Target Intel
   run reaches accepted submit and per-org gate PASS without a resubmit loop.
