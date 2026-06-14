# 2026-06-14 · DeepSeek run review — fixes shipped + open decisions

> Review of the `target_intel` run `pentest-chat-1781436545266-1` (model
> `deepseek-v4-flash`, workspace `~/golish-platform/Test1`, 2026-06-14 ~11:31–11:47
> UTC). User reported 6 observations; each was checked against the **raw**
> `~/.golish/backend.log` (not the transcript) + source. This doc records the
> evidence, the fixes shipped in this session, and the two items that need a
> product decision before implementation.

## 0. What the run actually did (raw-log evidence)

- `11:31:44` scoping deliverable submitted → gate **PASS** (`claims=2 findings=0
  evidence_refs=0`; scoping is a human-approval gate, evidence not required there).
- `11:31:44` graph-flow entered `stage=target_intel`; plan built (v1/v2).
- `11:32–11:47` the `stage_run` tool fanned out a `recon` sub-agent **per org**
  (two `::org::` sub-agent dirs — parent + one subsidiary). They ran ~240
  `pentest_run` invocations (dig/amass/subfinder/gau/whois/waybackurls).
- `backend.log` **ends at 11:47:09** with the 2nd org's recon still spawning
  `amass` (iter 10). **No `submit_stage_deliverable` for `target_intel` and no
  `target_intel` gate decision ever appears.**

**Conclusion:** the run never *completed* `target_intel` through the gate. What
looked like "完成" was a per-org recon sub-agent's text output ending — including
the garbled tool-call markup (issue #5). This reframes #4 and #6 below.

## 1. Issue-by-issue verdict

| # | Observation | Verdict | Evidence |
|---|---|---|---|
| 1 | "Run Everything" should be on every tool call | **Bug → fixed** | `ToolCallCard` (timeline card) had no approval control; only `CollapsibleToolCall` (pending-approval) + toolbar did |
| 2 | DeepSeek context > 128k | **Bug → fixed** | `deepseek_defaults()` had `context_window:128_000 / max_output:8_192` (stale V3.1). Official: V4 = **1M ctx / 384K out** |
| 3 | run stage should skip already-done targets | **Real gap → design + decision** | `OrgCompletionOracle` exists but only `AlwaysRunOracle` wired; dig re-run 69×, no per-technique dedup |
| 4 | doesn't completion require evidence? | **Not a bug** | `target_intel.json` gate already requires evidence for every claim/finding/coverage cell; run never reached the gate |
| 5 | final output is garbled | **Bug → fixed** | deepseek leaked Anthropic-style `<invoke name=…><parameter name=…>` markup; strippers only handled MiMo `<function=>` |
| 6 | osint etc. not called | **Partly real → decision** | `ENScan_GO` IS installed; gate requires OSINT coverage but it has no `min_invocations` floor (softer than DNS/subdomain) and the model never ran it; run never reached the gate |

## 2. Fixes shipped this session

### #2 — DeepSeek context window (low risk)
`backend/crates/golish-models/src/capabilities.rs :: deepseek_defaults()`
- `context_window 128_000 → 1_000_000`; `max_output_tokens 8_192 → 65_536`
  (official ceiling 384K; 64K is a cost-aware default and 8× the old cap that was
  truncating long turns — a contributing cause of #5's cut-off tail).

### #1 — Approval control on every tool card (low risk, frontend)
- `frontend/.../ToolCallSummary.tsx`: new shared `ApprovalModeInlineDropdown`;
  `ToolCallCard` now renders it (outer `<button>` → keyboard-accessible `<div>`
  so the dropdown nests legally) and threads `approvalMode`/`onApprovalModeChange`.
- `frontend/.../MessageBlock.tsx`: passes `approvalMode`/`onApprovalModeChange`
  into `ToolCallSummary`.

### #5 — Garbled tool-call leak (medium risk, has tests)
Root cause: `deepseek-v4-flash` (`native_best_effort`) degrades to emitting
Anthropic-style `<tool_calls><invoke name="…"><parameter name="…">…</parameter>
</invoke></tool_calls>` as **text**; every stripper only knew the MiMo
`<function=>` / `<parameter=>` dialect, so the markup leaked into the "Agent
Output" panel (`SubAgentDetailView`).
- `backend/crates/golish-core/src/textual_tool_call.rs`: `parse_textual_tool_calls`
  now also parses `<invoke name=…>` blocks (ignoring extra attrs like
  `string="true"`); `strip_textual_tool_call_markup` strips `<tool_calls>` /
  `<invoke>` / orphan `<parameter>` too. New helpers `parse_invoke_parameters`,
  `extract_quoted_attr`; 7 new tests incl. the exact leak shape. This is the
  shared choke point used by both the main loop and the sub-agent executor
  (`stream_processing.rs`, `final_summary.rs`).
- Frontend defense-in-depth for live streaming: `SubAgentDetailView.stripAgentXmlTags`
  and `MessageBlock.stripToolCallXml` now also strip the Anthropic dialect.

## 3. OPEN DECISION A — #3 idempotent / resume-skip

Two independent levels; both need a call from you:

**A1 · Org level (scheduler).** `run_fleet_scheduler` already takes an
`OrgCompletionOracle`; only `AlwaysRunOracle` (always re-run) is wired. The
deferred plan (`fleet.rs` comment) is a DB-truth `org_stage_has_truth` oracle.
- *Decision:* what counts as "this org's `target_intel` is done so skip it"?
  Options: (a) coverage_truth says every expected technique × in-scope asset has a
  terminal state; (b) a prior PASS gate decision exists for the org+stage; (c)
  conservative — only skip if both. Risk: skipping work that *should* re-run
  (stale truth). Recommend (a) gated behind a `--resume`/setting, default off.

**A2 · Per-technique level (recon sub-agent).** The sub-agent re-ran `dig` 69×
and repeated MX/TXT/NS/SOA across both orgs. This is model behavior, not the
scheduler. Options: (a) prompt nudge ("check the DB/ledger before re-querying");
(b) a recon-tool dedup cache keyed by (asset, technique) within a stage run.
- *Decision:* prompt-only (cheap, soft) vs. a real dedup cache (more work, harder
  guarantee). Recommend starting with (a) + measure.

## 4. OPEN DECISION B — #6 OSINT coverage

`ENScan_GO` (the `recon/osint` tool) is installed and the `target_intel` gate's
`coverage_complete` already requires `GOLISH-INTEL-OSINT` (authoritative, DB-truth)
— but only as a coverage cell that may be marked `blocked+note`, whereas DNS and
subdomain are *hard* `min_invocations` floors. In this run the model simply never
ran OSINT and the run never reached the gate.
- *Decision:* make OSINT as mandatory as DNS/subdomain by adding an
  `min_invocations` floor (e.g. `osint_enum: 1`) to `target_intel.json`? Trade-off:
  ENScan_GO is China-corp-focused; not every engagement wants it forced. Recommend
  a prompt nudge first; add the floor only if you want OSINT always-on.

## 5. Not changed (in scope discipline)
- #4 needs no code change (gate already correct).
- The underlying model-quality problems (malformed `dig <file>`, `dig --list-tools`,
  `amass passive` invalid subcommand, never submitting a deliverable) are inherent
  to `deepseek-v4-flash` reliability; #2 + #5 mitigate the worst symptoms.

## 6. Validation
- `just precommit` run after all edits (see `agent-progress.md` for the captured
  command + result).

## 7. Follow-up session (2026-06-14, BaJie MCP-agent-1) — #3 / #4 / #6 closed at the decidable layer

User directive: *finish everything you can decide; defer only genuine product
calls; conclude from the raw `~/.golish/backend.log`, not the transcript.* #1/#2/#5
were committed first as a clean baseline (`01edd1eb` code + `4240849b` doc), then:

### #4 — re-verified NOT a bug (no code change)
Re-read the gate config directly: `resources/harness/stages/target_intel.json`
`gate_rules` already enforce evidence — `for_all claims require non_empty
evidence_ids`, `for_all findings require evidence_refs`, every `found` **and**
`checked_empty` coverage cell requires `evidence_refs`, and `coverage_complete`
is `authoritative_found:true` + `derive_from_evidence:true` (reads DB/ledger
truth, not self-report). The reviewed run simply never reached the gate (no
`submit_stage_deliverable`). Verdict from §1 stands; nothing to change.

### #3 (A2) + #6 — prompt-layer fixes shipped (low-risk, reversible)
Both `target_intel` prompts already carried the "run each technique once / don't
loop dig" line and listed OSINT as a coverage cell, yet the model still re-ran
`dig` 69× and never produced OSINT. The decidable hardening (no engine/gate
behaviour change):
- **#3 resume/skip** — `target_intel.methodology.md` + `build_recon_prompt()`
  now tell the agent to `list_in_scope_targets` / `search_knowledge_base` first
  and **skip in-scope targets already at `passive`+** (this stage already ran for
  them). This rides the per-target status lifecycle already shipped in
  `manage_targets` (`new→passive→active→enumerated→vuln_scan→verified`, with the
  in-tool "read each target's status to SKIP targets already at/after the stage"
  guidance) — so per-target resume is now wired end-to-end at the prompt layer.
- **#6 OSINT** — methodology step 4 promoted OSINT from "(optional)" to a
  **required** coverage technique: `recon_enrich_assets` (ENScan) must yield
  org records / contacts / social accounts / business systems; if no
  provider/credential, record OSINT `blocked+note` — never silently skip. Same
  line added to `build_recon_prompt()`. The deferred hard-floor rationale is now
  documented inline in `target_intel.json` (`$comment_min_invocations`).

### STILL OPEN — needs your call (deferred, with recommendation)
- **A1 · org-level resume oracle (#3).** Wire a DB-truth `org_stage_has_truth`
  oracle into `run_fleet_scheduler` (today only `AlwaysRunOracle`). Risk: skipping
  an org that *should* re-run on stale truth. *Recommendation:* implement gated
  behind a default-OFF `--resume`/setting so it never changes default behaviour;
  needs your yes before I touch the scheduler core.
- **B · OSINT hard floor (#6).** Promote `osint_enum:1` into
  `target_intel.json.min_invocations` (as hard as DNS/subdomain). Trade-off:
  ENScan_GO is China-corp-focused; not every engagement wants OSINT forced.
  *Recommendation:* keep the prompt nudge for now; only add the floor if you want
  OSINT always-on for every engagement.

### Validation (this session)
- `just precommit` after all edits: `fmt` + `check-fe` + `test-fe` + `lint-rust`
  (clippy `--workspace -D warnings`) all PASS; `test-rust-all` (workspace nextest)
  was aborted mid-run by the user (`nextest -p golish-agent-kit -p golish-sub-agents`
  = 706/706 already covers the touched crates).

## 8. Raw-log re-verification (req: conclude from `~/.golish/backend.log`, not the transcript)

Re-checked the run directly in the raw `~/.golish/backend.log` (session
`pentest-chat-1781436545266-1`, 2026-06-14 11:29–11:47; the session's lines begin
at `backend.log:357039`). This **corrects one earlier verdict** that had been read
off the transcript:

- **#4 CONFIRMED.** Only `scoping` reached a gate:
  `harness::hook: gate decision: PASS stage_id=scoping ... claims=2 findings=0`
  @ 11:31:44. The single executed `submit_stage_deliverable`
  (`[tool-dispatch] Executing tool: name=submit_stage_deliverable ... args_len=1159`
  @ 11:31:36) was scoping's. **No `target_intel` gate decision and no `target_intel`
  submit appear** — the run died ~11:47 during recon. Not a bug.
- **#3 CONFIRMED.** Heavy repeated `dig` (328 `dig` log lines in the window) + amass
  (50). The efficiency red lines existed; the model ignored them.
- **#6 CORRECTED — OSINT *did* run.** The recon sub-agent executed the OSINT
  provider repeatedly during `target_intel`:
  `golish_recon_app::asset_intel::runtime::cli: running asset_intel cli_json provider
  provider=enscan-go` @ 11:32:55 (completed 11:33:22), then `provider=enscan-go-enrichment`
  @ 11:33:25 (completed 11:34:08) and again @ 11:45:04 (completed 11:45:48). The
  earlier "OSINT never ran" reading was a **transcript-visibility artifact**: ENScan
  runs via the `recon_enrich_assets` provider path (`asset_intel` runtime), which
  logs to backend.log but is NOT surfaced as a visible `pentest_run` tool call in the
  chat transcript. So #6 is a *visibility* gap, not a *not-called* gap. The §7 prompt
  change (OSINT = required coverage) is still valid defensive guidance, but the real
  run DID exercise OSINT.
- Also seen: 3 `stage_guard ... BLOCKED` (curl / python3 / check_job) — the model
  probing disallowed tools; the guard worked as designed.

**Implication.** The dominant failure was run inefficiency / stall during recon
(#3 + deepseek-v4-flash reliability), NOT missing OSINT. #2 (context/output bump) +
#3 (resume + once-per-root) + #5 (no garbled tail) target the real failure.

### Decisions taken on the two open items (req: decide what you can)
- **B (OSINT hard floor) — DECLINED.** OSINT is already exercised via the provider
  path (raw-log proof above); forcing `osint_enum` would add friction for
  non-China engagements without fixing the real issue. Kept the prompt nudge only.
- **A1 (org-level resume oracle) — left as documented default-off follow-up, not
  rushed.** `org_stage_has_truth` does not yet exist (only doc-comment placeholders
  in `engagement_truth.rs` + `fleet.rs`); building it is a from-scratch DB-truth
  query + scheduler wiring with real "what counts as done" semantics. The per-target
  resume already shipped (#3, `manage_targets` status lifecycle) satisfies the
  literal ask ("先看每个目标的状态，做过别重做"), so A1 is an additive optimization,
  not a fix the 6 issues require.
