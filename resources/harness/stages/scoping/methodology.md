**Goal:** lock the authorized scope/ROE and build the in-scope **organization
tree** (who you are authorized to test). This is an L0 scoping stage: you MAY
query business registries (OSINT) to resolve *who* you were asked to test, but
you do NOT probe, resolve, or scan the target hosts here. **When scope is given
as company name(s) you also do NOT invent individual targets from organization
metadata. Concrete domains / IPs / CIDRs / URLs / wildcard patterns are written
by trusted UI/CLI intake **before** this stage. Scoping only reviews the exact DB
snapshot; `target_intel` enriches already-authorized roots and cannot manufacture
a seed from model text or an organization profile. `customer_provided` is a
trusted intake source; provider/discovery sources are evidence or descendant
observations and never become authorization roots by being rediscovered. The org
tree plus the review decision is the scoping deliverable.**

**A. Scope given as company name(s)** ("搞一下平安", "对 X 做红队", a pasted list):

**STEP 0 — Reuse before you rebuild (check the DB first).** FIRST call
`manage_organizations(action="list")` and match the user's subject group to any
existing ROOT org. If it already exists you are in REUSE mode: do NOT
re-normalize, re-discover, or re-create — go straight to the one human reconfirm
(step 4) on the EXISTING org tree, then submit (step 5). Only when no matching org
exists do you build it via steps 1–3 below. This keeps a confirmed scope stable
across sessions and never duplicates an org that is already in the database.

1. **Normalize the name first — look it up, never recall it.** For EACH company,
   call `recon_lookup_company` to resolve the canonical registered name
   (以企查查为准). NEVER guess or write a company's full name from memory.
   - **When the lookup returns candidates, let the user PICK — never make them
     type the legal name.** Call `ask_human(input_type="choice", options=[…])`
     with the canonical names the lookup returned (put the unified social credit
     code / legal representative in each option label so near-duplicate entities
     are distinguishable), so the user confirms the exact entity in one click;
     use the picked name as canonical. The choice UI already exposes an "Other"
     field for a custom answer, so you never need a bare free-text prompt here.
   - Only if the lookup provider is unavailable or returns nothing do you fall
     back: ask once with `ask_human(input_type="freetext")` for the exact
     registered name, or mark it checked-empty — do NOT invent a name.
2. **Subsidiaries are a SCOPE decision — ask before you discover.** Before
   calling `recon_discover_subsidiaries`, ask the human with `ask_human`
   (`input_type="choice"`,
   `context="{\"decision\":\"subsidiary_scope\",\"organization_id\":\"<root-id>\"}"`,
   options like ["不纳入子公司", "纳入：≥51% 控股", "纳入：≥100% 全资", "纳入：自定义比例"])
   whether subsidiaries/holdings are in scope and at what ownership threshold
   (and whether branch offices 分公司 are included). Do NOT pick a threshold yourself.
   - **Not in scope →** do not call discovery; record the exclusion in the
     `scope_confirmed` summary. Do NOT add `skipped_checks` for this normal scope
     decision, do NOT fabricate a tree, and do NOT call `propose_candidates` or
     manufacture an empty `unit_review`. The persisted parent-only choice is the
     deterministic human approval for this branch.
   - **In scope →** call `recon_discover_subsidiaries` with `min_ownership_percent`
     set to the human's threshold (and `include_branches: true` if they asked for
     branches). Discovery does NOT auto-create anything — it returns a
     `subsidiaries` array of candidates (each with `name`, `ownership_percent`,
     `status`, and `meets_threshold` against the chosen threshold). Found none?
     checked-empty + evidence — NEVER fabricate a subsidiary.
3. **Show the discovered subsidiaries and let the user PICK — never auto-add.**
   Call `ask_human(input_type="unit_review", context="{\"organization_id\":\"<id>\"}")`
   where `<id>` is the root org you just passed to `recon_discover_subsidiaries`
   in step 2. The review table loads that org's discovered candidates from the DB
   and shows them (with ownership) for the user to confirm/edit — you do NOT need
   to copy the candidate list into `context` yourself (that array is fragile and
   often arrives mangled). The user confirms/edits which subsidiaries are in scope.
   (No subsidiaries in scope? skip this and the root is the whole tree.)
4. **Trace, never recall.** Every name you record MUST come from a real
   `recon_lookup_company` (root) or `recon_discover_subsidiaries` (subsidiary)
   result and the user's step-3 picks — NEVER a name from memory or unrelated
   context.
5. **Record only what was confirmed, then submit.** Create the ROOT org(s) with
   `manage_organizations` (`create`, or `create_batch` for many canonical root
   names from step 1). Create ONLY the subsidiaries the user picked in step 3 in
   ONE call: `manage_organizations(action="create_batch", names=[<all picked>],
   parent_id=<root org id>)` — this get-or-creates every picked subsidiary as a
   child of the root. Do NOT loop `action="create"` per child (it is slow and
   trips the repeated-tool-call detector). Do NOT create unpicked candidates.
   **The org tree is now the deliverable — do not invent assets from it.** Do NOT
   add targets (`manage_targets`) or turn discovered subsidiaries into
   domains/IPs. For profiles with interactive target confirmation, the trusted
   DB snapshot decides whether a second review exists: an **empty** snapshot is a
   valid organization-only engagement, so do NOT manufacture an empty
   `scope_review`; the confirmed `unit_review` is sufficient. A **non-empty**
   snapshot requires exactly one `ask_human(input_type="scope_review")` containing
   only those trusted rows already present in the request/task context for UI
   rendering. That context is not authority: the backend independently reloads
   the DB snapshot and exact-compares the approved response. Then emit a
   `scope_confirmed` claim (plus `scope_human_approved` when approval was
   required) and CALL `submit_stage_deliverable` immediately. In the normal
   evidence-free scoping path, submit only the claim fields (`kind`, `subject`,
   `summary`) and omit empty `evidence_refs`, `findings`, `coverage`,
   `skipped_checks`, and `required_checks_done`; the submit tool canonicalizes
   omitted empty fields.

**B. Scope given as concrete hosts / IPs / URLs (the user handed you an explicit
target list):** the engagement subject is already concrete. Create the owning
organization if one is clearly named (`manage_organizations`). The trusted UI/CLI
must already have ingested the exact seed rows before Scoping starts;
`customer_provided` rows are part of this trusted intake. Because this path has a
non-empty concrete snapshot, an interactive-approval profile must call
`ask_human(input_type="scope_review")` exactly once and use `context` only to
render the exact trusted rows already supplied by the UI/CLI/task input. The
backend separately loads the current org's trusted snapshot; model-controlled
context cannot authorize or rewrite it. An unchanged confirmation must preserve
each row's canonical value, `target_type`, and `scope`. A user edit is a proposal,
not a mutation; stop immediately—a later second review cannot replace or wash out
that decision. Continue only after trusted intake writes the revised row, then
start a fresh Scoping attempt. `manage_targets` is
intentionally unavailable here. If an approved seed is absent from the target
store, never ask Target Intel to invent it.

**Stop conditions / red lines:**

- **Never fabricate scope.** Every organization, subsidiary, and target you record
  MUST come from a real `recon_lookup_company` / `recon_discover_subsidiaries`
  result or from the user-provided scope — never from memory. NEVER pull unrelated
  companies or public test sites (vulnweb.com, testphp.*, acunetix demo hosts)
  into the scope.
- **Target mutation is outside the stage.** Scoping outputs the ORG TREE and the
  review decision only.
  `manage_targets` is NOT available in this stage (removed from the tool list), so
  do not try to record targets; never turn discovered subsidiaries into targets,
  and never turn a `scope_review` into scope expansion. The optional second
  review is only for the user's concrete target list when the active profile
  requires it. Exact seed rows come from the trusted pre-stage UI/CLI ingestion;
  Target Intel may enrich their authorized descendants but cannot create scope
  from model assertions.
- **Profile metadata is not authorization.** `organizations.domains`,
  `app_domains`, `ip_ranges`, provider results, DNS answers, certificate names,
  and HTTP redirects are observations/hints. None can independently create an
  executable root target. The trusted intake tier includes
  `manual` / `imported` / `customer_provided` / `stage-run-seed` / `seed` / `cli`;
  `discovered`, `asset_intel`, and other provider-derived source labels stay
  outside that tier even after repeated runs. A trusted exact domain target may authorize its strict
  descendants; `*.example.com` authorizes only strict children, never the apex,
  and the wildcard row itself is never executed. Target Intel nevertheless owns
  one passive SUBDOMAIN cell on that pattern so child expansion cannot disappear
  as vacuous N/A; a promoted strict-child target is required for `found`.
- Ask the subsidiary-scope `choice` at most once, then at most one review per
  applicable decision type (`unit_review` only for the included branch, followed
  by profile-required `scope_review` for a non-empty trusted target snapshot).
  Do not ask for an empty table and do not loop asking the user repeatedly. The
  backend reads the latest parseable same-root choice only to recover already
  in-flight legacy retries; that compatibility rule is not permission to re-ask.
- do NOT probe or scan the target hosts here: no dig/whois/subdomain/port/http. The
  runtime blocks those scan tools in this stage. Business-registry OSINT
  (`recon_lookup_company` / `recon_discover_subsidiaries`) is allowed because it
  DEFINES scope; touching the target is not.
- The moment the scope (org tree for path A; the user-provided target list for
  path B) is confirmed and recorded, submit and let the harness advance to
  `target_intel`.
