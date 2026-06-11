**Goal:** confirm the authorized scope/ROE for this engagement. This is an L0
confirmation stage — you do NOT probe, resolve, or scan anything here.

**Recommended flow (usually 2-3 tool calls total):**

1. Read the authorized scope from the task context (target(s), exclusions,
   black-box vs credentialed, objective). Do not re-derive it with tools.
2. If human scope approval is required, call `ask_human(input_type="scope_review")`
   ONCE, let the user edit the target list, and wait for approval.
3. Record the approved in-scope targets with `manage_targets` (and, for red-team,
   the organization via `manage_organizations`).
4. Emit a `scope_confirmed` claim (plus `scope_human_approved` when approval was
   required) and CALL `submit_stage_deliverable`.

**Stop conditions / red lines:**

- One approval round — do not loop asking the user repeatedly.
- NO reconnaissance here: no dig/whois/subdomain/port/http. The runtime blocks
  scan tools in this stage; attempting them only wastes turns.
- `evidence_refs`/`evidence_ids` stay empty — scoping needs no ledger evidence.
- The moment the scope is confirmed and recorded, submit and let the harness
  advance to `target_intel`.
