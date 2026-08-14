# Investigation company and asset queues

> Superseded for asset-internal orchestration by
> `2026-08-14-investigation-asset-primary-dynamic-team.md`. Company/asset ordering and exact lane
> ownership remain authoritative; the fixed role roster, role order and exact-role barrier do not.

> **Status:** Approved for implementation on 2026-08-13.
>
> **Supersedes:** the organization-wide Analysis wave and one-Primary-per-VerificationTask
> topology in `2026-08-12-investigation-primary-led-verification-execution.md`. The exact
> PreparedAction/JIT/scope/budget/credential/evidence/Oracle safety boundary remains in force.

## 1. Product contract

Investigation is a durable nested queue, not an operation-wide hypothesis batch:

```text
Investigation
└─ company queue (strictly ordered)
   └─ asset queue (strictly ordered)
      └─ one durable Asset Primary and message chain
         ├─ browser
         ├─ researcher
         ├─ pentester
         └─ adviser
```

For each asset, the four reasoning roles first read only the predecessor-stage evidence, the
content-addressed CyberStrike methodology corpus, Knowledge Base, Memory and Knowledge Graph.
They challenge one another and propose zero or more evidence-bound hypotheses for the current
asset. The Asset Primary then drains the asset's hypothesis backlog one hypothesis at a time.

Verification is the active penetration-testing phase. The existing Tool Manager is the sole
catalog and execution source: there is no second Investigation-only shortlist. The Asset Primary
and its verification roles receive the current installed/enabled/ready Tool Manager inventory and
may autonomously invoke its browser, HTTP, CLI, scanner, script, PoC, or later-added tools. Browser
means a real managed browser session, and CLI/script/PoC execution uses the same managed-process
and tool-config infrastructure as the rest of the product. A hypothesis may make several tool
calls, inspect their results, change strategy, and enqueue another action. The host still injects or
validates the immutable target/scope/budget/credential authority immediately before each I/O and
lands a durable redacted invocation/audit record. Oracle or evidence authority may be attached when
the selected Tool Manager adapter can produce it, but neither is required one-for-one. A role name
alone never grants a tool.

New evidence may cause the Asset Primary and reasoning team to append successor hypotheses to the
same asset backlog. The scheduler must not advance to the next asset until every current canonical
hypothesis has a durable resolved conclusion. `supported`, `refuted`, and `dismissed` are resolved;
`open`, `untested`, `inconclusive`, and `blocked` are not. A blocked tool call pauses the current
asset and invites another strategy; it never turns an unresolved hypothesis into a completed one.
The scheduler must not advance to the next company until every frozen asset lane in the current
company has reached this resolved fixed point.

## 2. Identities and ordering

- Company order is frozen from `operation_org_scope_units(depth, ordinal, organization_id)`.
- The canonical asset lane key is a frozen in-scope `targets.id`. Its value/type/source are
  immutable queue-member snapshots, not authorization derived from prompt text.
- Web origins and API endpoints remain hypothesis subjects, but must resolve through durable
  observations/foreign keys to the current lane's `target_id`. A subject that cannot be proven to
  belong to the lane is a typed unassigned residual and cannot create a hypothesis or action.
- A hypothesis root permanently belongs to one `asset_lane_id`. Revisions, generations,
  VerificationTasks, Campaigns, FactDelta, pending evolution and successor generations may never
  change lanes.

## 3. State machines

Company lane: `queued -> active -> completed|blocked`.

Asset lane:

```text
queued -> analyzing -> verifying -> consolidating
                    ^                  |
                    |---- evolving <---|  material FactDelta / new hypothesis
                               |
                               +-> fixed_point
                               +-> blocked/residual
```

Exactly one company and, within it, exactly one asset may be active. Every transition is a CAS
event with an immutable previous/new state witness. Asset evolution fuel is frozen and persisted;
process restart cannot reset it.

## 4. Asset team and hypothesis backlog

The Asset Primary is the only coordinator for the complete asset lifetime. Analysis and
Verification phases reuse its durable WorkItem, Worker identity and message chain. Each initial or
evolution analysis epoch must have an exact terminal census for browser, researcher, pentester and
adviser before synthesis. This roster is exact; verification may vary tools and tactics freely, but
does not create a second role topology.

The business backlog is the lane-scoped projection of current canonical hypothesis heads plus
typed hypothesis discoveries that have not yet been admitted or dismissed as exact duplicates.
VerificationTask, Campaign, action, Oracle, FactDelta and evolution rows remain diagnostic/audit
projections only. The backlog is server-derived; the model cannot delete, skip, reorder across
assets or claim that a queue is empty.

Within one current asset, hypotheses are processed in stable generation/member ordinal order.
One hypothesis may yield multiple sequential or bounded-parallel attack actions. Execution results
return to the same Asset Primary, which can revise strategy or append a new hypothesis. Cross-asset
signals are retained as data-only residuals for the later asset lane; they do not switch the active
lane.

## 5. Fixed point

An asset fixed point means only that the frozen asset queue has no remaining unresolved hypothesis.
It is not a safety claim. A fixed-point receipt requires an exact census proving:

- every lane hypothesis has one current canonical `supported`, `refuted`, or `dismissed` resolution;
- no lane hypothesis remains `open`, `untested`, `inconclusive`, or `blocked`;
- every lane-valid hypothesis proposed during verification has first been admitted as a canonical
  hypothesis (and therefore appears in the same resolution census);
- the exact canonical-hypothesis set hash and terminal-resolution set hash are sealed.

Tool names, tool-call counts, per-role tool allocation, PreparedAction counts, execution counts,
receipt counts and Oracle counts are deliberately absent from the stage gate. They remain durable
audit/evidence inputs, but a hypothesis can require zero, one or many calls and those cardinalities
must never decide whether it is resolved.

Campaign, Wave, FactDelta, Oracle and evolution bookkeeping are likewise not independent business
completion gates. They may feed evidence or create another canonical hypothesis, but once every
current canonical hypothesis has a terminal resolution and there is no unadmitted hypothesis
proposal, those internal records cannot keep the asset open.

Zero bounded hypotheses also require a typed asset fixed-point receipt; the absence of a Campaign
is not sufficient. Exhausted durable evolution fuel produces `evolution_fuel_exhausted` blocked or
residual authority, never a false fixed point.

## 6. Tool execution

Reasoning workers are read-only while proposing/challenging hypotheses. In verification mode the
same asset team may select any Tool Manager entry whose durable state is installed, enabled and
ready for the current host policy. Availability is discovered dynamically from Tool Manager; it is
not encoded as a frozen Rust enum of a few well-known scanners. Each concrete invocation still
receives an exact execution assignment, and model-visible arguments are not authorization: the
runtime intersects them with the managed tool schema and immutable current-asset authority. This
permits AI to operate the real browser, sqlmap, curl, Nuclei, other managed CLI tools, and to write
and run temporary scripts or PoCs through the existing managed execution tools without widening
scope, budgets, credentials or targets.

Each invocation keeps its normal Tool Manager audit/evidence record. The Asset Primary records a
separate evidence-citing hypothesis resolution after the team has finished testing it. The gate
validates the resolution's lane/hypothesis ownership and cited durable observations, but does not
prescribe which tools produced them or require an execution/receipt/Oracle one-to-one mapping.

## 7. Cutover and acceptance

The schema migration is forward-only, but the runtime contract is singular. Historical
organization-wide rows remain readable only as audit history; they are never resumable execution
authority. Every new or resumed runnable Investigation requires company and asset queue seals plus
non-null `asset_lane_id` with exact lane guards. No historical row
is reinterpreted or backfilled as an asset fixed point.

Acceptance requires an Investigation-only fork from the retained, final-sealed Application
Understanding operation. Local loopback tests are component evidence only; completion requires a
real CLI run showing ordered companies/assets, the four-role discussion, dynamic Tool Manager
verification, an explicit terminal resolution for every current canonical hypothesis, admission or
exact-duplicate dismissal of every newly discovered hypothesis, and final closure.
