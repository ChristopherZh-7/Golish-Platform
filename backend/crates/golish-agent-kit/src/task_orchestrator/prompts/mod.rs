//! System prompts for Task mode agents.
//!
//! Each agent in the Task mode pipeline has a specialized prompt
//! that matches PentAGI's template structure.

mod pipeline;
pub use pipeline::*;

use crate::harness::profile::ScopingPolicy;
use crate::harness::stage_spec::StageSpec;
use crate::harness::types::StageKind;

/// Intent classifier prompt — determines whether a user message in Task mode
/// is an actionable task or just casual conversation (greeting, question, etc.).
///
/// The LLM responds with a single word: "TASK" or "CHAT".
pub fn intent_classifier_prompt() -> &'static str {
    r#"You are an intent classifier. Given a user message, determine whether it is:

- **TASK**: An actionable request that requires planning, tool execution, or multi-step work.
  Examples: "Scan example.com for vulnerabilities", "Write a script to enumerate subdomains",
  "Analyze the auth module for security issues", "Set up a reverse proxy"

- **CHAT**: A greeting, casual remark, simple question, or anything that does NOT require
  planning or tool execution.
  Examples: "Hello", "你好", "What can you do?", "How are you?", "Thanks",
  "What is SQL injection?", "Explain XSS to me"

Respond with ONLY one word: TASK or CHAT. Nothing else."#
}

/// Truncate a string slice to at most `max` bytes without splitting a multi-byte char.
pub(crate) fn safe_truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &s[..end]
    }
}

/// C2 · Operation Harness stage charter — 注入到 subtask 描述顶部.
///
/// 告诉执行 agent 当前 stage 的允许/禁止工具面、必须提交的结构化 deliverable、
/// 以及确定性 gate 会检查哪些项. 仅在 subtask 有 `harness_stage` 时由
/// `execute_single_subtask` 拼到描述前.
///
/// `scoping_policy` (设计 2026-06-06 §3.3/§3.4): scoping 段按 profile 策略分流——
/// `require_human_scope_approval` 时追加「submit 前必须有 scope_human_approved
/// claim」的硬门禁提醒, 与 gate hook 注入的规则呼应. 非 scoping stage 不读它.
pub fn stage_charter(spec: &StageSpec, scoping_policy: &ScopingPolicy) -> String {
    let allowed = if spec.allowed_tool_types.is_empty() {
        "(none — this stage runs no scan tools)".to_string()
    } else {
        spec.allowed_tool_types.join(", ")
    };
    // gate-rules-migration (2026-06-05): pass-criteria moved from `required_checks`
    // (deleted) to `gate_rules`; surface each rule's short summary to the agent.
    let checks = if spec.gate_rules.is_empty() {
        "(none)".to_string()
    } else {
        spec.gate_rules
            .iter()
            .map(|r| r.summary())
            .collect::<Vec<_>>()
            .join(", ")
    };
    // Coverage matrix (设计 2026-06-05): when the stage declares expected
    // techniques, the `coverage_complete` gate op requires every in-scope asset
    // to take each technique to a terminal state. Surface that contract so the
    // agent fills the `coverage` field instead of leaving cells not_attempted.
    //
    // Phase C slim deliverable (设计 2026-06-22): when the stage adjudicates
    // coverage from DB truth (`facts_from_db_truth`), the completeness matrix is
    // filled by the DB-truth projection, not self-report. Telling the agent to
    // fill the matrix cell-by-cell + tag claims is then counter-productive (drives
    // the "fill the table" busywork loop). Instead instruct it to just run the
    // collection tools (data lands in DB) and only self-report blocked/not_applicable.
    let coverage_line = if spec.expected_techniques.is_empty() {
        String::new()
    } else if spec.facts_from_db_truth {
        let db_truth_action = if spec.kind == StageKind::ExternalAttackSurface {
            "Run the EAS active mapping tools so their data LANDS in the database \
             (stage_run -> prober -> list_attack_surface_seeds/list_in_scope_targets -> \
             batch-first wrapper calls: eas_discover_ports first for concrete IP/CIDR hosts, \
             eas_fingerprint_services/nmap -sV for every confirmed open port, \
             eas_probe_http_liveness/httpx for domain/URL/web-origin liveness, and \
             eas_fingerprint_web_stack/whatweb for each confirmed HTTP(S) web origin -> \
             manage_targets/targets.ports/fingerprints/technique_outcomes)."
        } else {
            "Run the target_intel registry tools so their data LANDS in the database \
             (e.g. recon_map_assets / recon_lookup_whois -> target_assets / dns_records where \
             provider data exists / organizations.*)."
        };
        let terminal_hint = if spec.kind == StageKind::ExternalAttackSurface {
            "ONLY add a `coverage` cell when the DB cannot derive the terminal state: \
             for active negatives use checked_empty when a scan/probe really ran and found nothing, \
             or blocked/not_applicable+note when the technique cannot apply or was blocked. \
             If no ports are open, mark SERVICE-FINGERPRINT not_applicable with a note; do NOT invent \
             a found service from HTTP liveness alone. If a web origin is confirmed, run WhatWeb once \
             for that origin; if multiple domains share one IP:port, each confirmed origin still needs \
             WEB-FINGERPRINT because Host/SNI can change the stack."
        } else {
            "ONLY add a `coverage` cell when a technique genuinely has no data source or is blocked \
             for an asset: mark it `blocked` or `not_applicable` with a `note` naming the failed/absent source."
        };
        format!(
            "\n- **Coverage (auto-adjudicated from the DATABASE)** — for these techniques: {}. \
             {db_truth_action} The deterministic gate reads \
             the DB directly to score per-(asset × technique) completeness. You do NOT need to fill the \
             `coverage` matrix cell-by-cell, and you do NOT need to tag claims/findings with `technique` \
             — leave found cells to the DB-truth projection. {terminal_hint} Before submit, call \
             `check_stage_asset_coverage`; if `ready_to_submit=false`, close the reported `gap_examples` \
             instead of submitting. A blocked/not_applicable cell is \
             TERMINAL and clears that gap — do NOT resubmit the same matrix expecting it to fill \
             (\"checked-empty\" is NOT \"unchecked\").",
            spec.expected_techniques.join(", ")
        )
    } else if spec.kind == StageKind::ExternalAttackSurface {
        format!(
            "\n- **Coverage (per in-scope asset)** — EAS asset/port contract: give EACH applicable \
             technique a terminal status for EVERY asset via the `coverage` field: {}. \
             domain/URL assets require LIVENESS and WEB-FINGERPRINT once HTTP(S) is confirmed; \
             IP/CIDR-discovered IP assets require PORT first, then LIVENESS + SERVICE-FINGERPRINT \
             and WEB-FINGERPRINT if the open service is HTTP(S). Host-level PORT/SERVICE belongs to the \
             concrete IP target, never to an unresolved domain string. Per cell: found / checked_empty / \
             blocked|not_applicable+note. A missing (asset × technique) = not_attempted = gate \
             BLOCK (\"checked-empty\" is NOT \"unchecked\").\n\
             - **EAS denominator** — if you explicitly submit found/checked_empty coverage, set \
             `tested_units` and `total_units`. LIVENESS uses 1/1 for the checked host or URL. \
             PORT uses the scanned port-set denominator for that host/IP. SERVICE-FINGERPRINT uses \
             `tested_units = open ports fingerprinted` and `total_units = open ports discovered`. \
             If no ports are open, mark SERVICE-FINGERPRINT `not_applicable` with a note; do NOT \
             submit checked_empty with total_units=0. HTTP liveness alone is never PORT or \
             SERVICE-FINGERPRINT coverage. If a later port discovery expands the open-port set, \
             fingerprint the newly discovered ports before submit. Before submit, call \
             `check_stage_asset_coverage`; if \
             `ready_to_submit=false`, close the reported `gap_examples` instead of submitting.",
            spec.expected_techniques.join(", ")
        )
    } else {
        format!(
            "\n- **Coverage (per in-scope asset)** — give EACH of these techniques a terminal status \
             for EVERY asset via the `coverage` field: {}. Per cell: found / \
             checked_empty / blocked|not_applicable+note. A missing (asset × technique) \
             = not_attempted = gate BLOCK (\"checked-empty\" is NOT \"unchecked\").\n\
             - **Coverage is measured against a DENOMINATOR** — for each found/checked_empty cell set \
             `tested_units` and `total_units` (M = the enumerated endpoints/params/services for that \
             asset×technique, inherited from enumeration). The default requires `tested_units == \
             total_units` (full coverage). To legitimately sample a huge surface you MUST set \
             `sampling_rationale` AND meet the coverage ratio; otherwise the cell counts as partial \
             (not finished) and the gate BLOCKS. Testing 3/5000 endpoints then claiming checked_empty \
             is false coverage. (blocked / not_applicable cells are exempt from the denominator.)\n\
             - **Tag claims/findings with `technique`** — set each claim's/finding's `technique` field to \
             the matching expected technique id above, using the SAME `subject` string as the cell's \
             `asset`. Technique-tagged items corroborate your 'found' coverage cells (a 'found' cell with \
             NO matching tagged claim/finding on the same asset is rejected) and can auto-derive cells you \
             did the work for but forgot to declare. Before submit, call `check_stage_asset_coverage`; if \
             `ready_to_submit=false`, close the reported `gap_examples` instead of submitting.",
            spec.expected_techniques.join(", ")
        )
    };
    // Spec-derived minimum tool invocations: the gate's vacuous_check +
    // min_invocations_check are deterministic, so surface the exact requirement
    // to the agent (sorted for stable prompts across HashMap iteration order).
    let min_inv = if spec.min_invocations.is_empty() {
        "(no per-tool minimum)".to_string()
    } else {
        let mut items: Vec<String> = spec
            .min_invocations
            .iter()
            .map(|(tool, n)| format!("{tool} ≥ {n}"))
            .collect();
        items.sort();
        items.join(", ")
    };
    let min_inv_keys_json = {
        let mut keys: Vec<String> = spec
            .min_invocations
            .keys()
            .map(|k| format!("\"{k}\""))
            .collect();
        keys.sort();
        format!("[{}]", keys.join(", "))
    };
    // C2c · deterministic submission channel: the agent CALLS the
    // `submit_stage_deliverable` tool (typed args), validated by the gate.
    let submission_instr = "### REQUIRED — submit via the `submit_stage_deliverable` tool\n\nWhen this stage's required tools have actually run, CALL the `submit_stage_deliverable` tool with the fields below as STRUCTURED ARGUMENTS — do NOT print or describe the JSON in prose, and do NOT just delegate \"write the JSON\" to another agent. The runtime validates your submission with the deterministic gate. The deliverable shape:";
    // Per-stage note for scoping. The "do not probe" boundary is now enforced
    // deterministically by the runtime (the stage tool guard resolves
    // pentest_run/shell to their real capability and blocks forbidden ones), so
    // this prose no longer enumerates tool names — it only states intent + the
    // (relaxed) evidence contract that stops the fabricated-evidence retry loop.
    let stage_specific = if spec.kind == crate::harness::StageKind::Scoping {
        let mut s = String::from(
            "\n### SCOPING — authorization confirmation ONLY\n\
- This stage CONFIRMS the target is authorized; it does NOT probe. The runtime enforces the stage's tool boundary, so reconnaissance attempts are blocked here — that work belongs to the next stage.\n\
- You do NOT need tool evidence here: confirm the authorized scope from the task context (target, exclusions, black-box vs credentialed, objective) and submit. Omit `evidence_refs`, claim `evidence_ids`, `findings`, `coverage`, `skipped_checks`, and `required_checks_done` unless this run actually produced those records — the submit tool canonicalizes omitted empty fields.\n\
- Emit ONE claim with kind \"scope_confirmed\" summarizing the authorized scope. If you created or confirmed an engagement organization via manage_organizations, set this claim's `subject` to that organization_id (the UUID) so the whole engagement binds to it (downstream stages then scope to that org's subtree only); otherwise use the engagement subject name. Then CALL `submit_stage_deliverable`.\n",
        );
        // scoping 人工确认硬门禁 (设计 2026-06-06 §3.4): policy 要求时, gate 额外要求一条
        // scope_human_approved claim — 提前在 charter 里说清「submit 前必须有人确认」,
        // 与 apply_harness_gate_hook 注入的 scoping_human_gate_rule 呼应.
        if scoping_policy.require_human_scope_approval {
            s.push_str(
                "- HARD GATE — human scope approval REQUIRED: first inspect the trusted pre-stage target snapshot. When it is NON-EMPTY, call `ask_human(input_type=\"scope_review\")` EXACTLY ONCE with those exact rows and have the human confirm the unchanged snapshot. The editable table may only PROPOSE changes; an edited response does not mutate or authorize targets, so stop and ask the user to update the trusted UI/CLI target intake before a fresh Scoping attempt. Never open a second scope_review to replace the first decision. When the trusted snapshot is EMPTY (company/organization-only input), do NOT manufacture an empty target-table review: the applicable organization review/confirmation is the approval. After that applicable human decision, emit a SECOND claim with kind \"scope_human_approved\" (subject = the engagement subject's organization_id) citing its ask_human request_id when one exists. The deterministic gate compares every persisted non-empty target review to DB truth and BLOCKS a missing, repeated, stale, or edited list.\n",
            );
        }
        // red_team (require_unit_candidates): the gate cross-verifies the REAL
        // unit-candidate flow against this run's tool calls — a claim alone will
        // NOT pass. State it plainly so the model performs the steps.
        if scoping_policy.require_unit_candidates {
            s.push_str(
                "- HARD GATE — RED-TEAM subsidiary scope is VERIFIED against actual tool calls and the persisted human choice, not claims. First call `ask_human(input_type=\"choice\", context=\"{\\\"decision\\\":\\\"subsidiary_scope\\\",\\\"organization_id\\\":\\\"<root-id>\\\"}\")`. If the human explicitly chooses parent/root-only scope, that decision completes this branch: do NOT run discovery, `propose_candidates`, or an empty `unit_review`. If subsidiaries may be included, you MUST call `manage_organizations(action=\"propose_candidates\")`, then `ask_human(input_type=\"unit_review\")` so the user judges candidate units. If the root org/tree already exists (REUSE mode), DO NOT call `create`/`create_batch` just to satisfy the gate; only create a missing root or units the user explicitly added/confirmed. A claim alone cannot satisfy either branch.\n",
            );
        }
        s
    } else {
        String::new()
    };
    format!(
        r#"## OPERATION HARNESS — STAGE CHARTER

You are operating inside the **{stage}** stage of an authorized operation. Stay within this stage's boundary:

- **Allowed tool types** (scan tools — use ONLY these tool types): {allowed}
- **Minimum tool invocations** (you MUST actually run these): {min_inv}
- A deterministic gate will check: {checks}. Unverified natural-language claims do NOT pass the gate.{coverage_line}
- Do not escalate authorization or jump to another stage — the runtime advances stages for you once the gate passes.
{stage_specific}
{submission_instr}

```json
{{
  "stage_id": "{stage}",
  "claims": [
    {{"kind": "http_service_observed", "subject": "<host>", "summary": "<what was observed>", "technique": "<registered technique id backing this claim — omit if none applies>"}}
  ],
  "evidence_refs": [],
  "findings": [],
  "skipped_checks": [],
  "required_checks_done": {min_inv_keys_json}
}}
```

The submit tool assigns `stage_run_id`; do not pass it. Empty arrays are optional: omit `evidence_refs`, `findings`, `coverage`, `skipped_checks`, and `required_checks_done` when the stage has none.

IMPORTANT — evidence ids are internal ledger references, not fields you must fill. Do NOT hunt for ids in raw tool output and NEVER invent small guessed integers (1, 2, 3, …). Scans are recorded to the ledger automatically and the deterministic gate resolves DB/ledger truth. If you already have a real ledger id you may include it, but omission is preferred over guessing; fabricated ids FAIL.

Gate rules your deliverable MUST satisfy (otherwise it is rejected and you redo the stage):
- `stage_id` MUST equal "{stage}"; `stage_run_id` MUST be a valid, non-nil UUID v4.
- Do not write evidence ids unless they are real ledger ids explicitly known to you; the backend resolves evidence from DB/ledger truth.
- `evidence_refs` is optional. If present, every id must exist in the ledger.
- `required_checks_done` MUST name every tool you were required to run: {min_inv}.
- If you deliberately skip a required check, record it in `skipped_checks` with a reason — "checked-empty" is NOT the same as "unchecked". Do not use `skipped_checks` for normal scope exclusions; record those in the scope claim summary.

"#,
        stage = spec.id,
        allowed = allowed,
        checks = checks,
        coverage_line = coverage_line,
        min_inv = min_inv,
        min_inv_keys_json = min_inv_keys_json,
        submission_instr = submission_instr,
        stage_specific = stage_specific,
    )
}

/// C2b · Final stage-discipline directive, appended to the very END of the
/// assembled subtask prompt (after the base orchestrator prompt + charter +
/// description) so it is the most recent — and therefore highest-salience —
/// instruction the model sees. Only appended when the subtask belongs to a
/// harness stage (`ExecutionContext::harness_stage.is_some()`).
///
/// Targets two observed live failures:
///  1. The deterministic gate was skipped because the agent ended with a prose
///     summary and never emitted the `StageDeliverable` JSON. The charter
///     already asks for it, but that requirement is injected mid-prompt (inside
///     the subtask description) and is out-ranked by recency + the base
///     orchestrator prompt's prose-oriented "COMPLETION REQUIREMENTS". Restating
///     it LAST makes the agent actually emit the parseable block.
///  2. Agents rabbit-holed when a tool was unavailable — spawning more
///     sub-agents / installing / retrying a blocked tool. This tells them to
///     stop and report instead, and to respect the stage's tool boundary (which
///     the runtime already enforces, so retries are wasted turns).
pub fn stage_discipline_reminder() -> String {
    let boundary = r#"## STAGE DISCIPLINE — READ THIS LAST (overrides any earlier output-format instructions)

- Stay inside this stage's tool boundary: use ONLY the stage's allowed tools and NEVER call a forbidden tool. The runtime blocks forbidden calls anyway — do not waste turns attempting them.
- If a required tool is unavailable, errors, or returns nothing on two attempts: STOP and record it in `skipped_checks` with the reason ("checked-empty" is NOT the same as "unchecked"). Do NOT install tools, spawn additional sub-agents, or retry the same tool to work around an unavailable capability.
- The runtime advances stages for you once the deterministic gate passes — do not jump ahead to a later stage."#;
    // C2c · the deliverable-submission directive: always CALL the submit tool.
    let submit = "\n\n### Submit the StageDeliverable by CALLING `submit_stage_deliverable`\nThe stage completes ONLY when you call the `submit_stage_deliverable` tool with structured fields (`stage_id`, `claims`, plus optional `coverage`/`findings`/notes). Do NOT just print or describe the JSON, and do NOT only delegate \"write the JSON\" to a sub-agent — if you delegated to a reporter, take its result and call `submit_stage_deliverable` yourself. A prose-only ending leaves the gate with nothing to validate and forces you to redo this entire stage.";
    format!("{boundary}{submit}")
}

/// Agent-driven stage execution directive (设计 2026-06-04 · D1=B / 阶段内 todo).
///
/// 注入到阶段级 agent loop 的描述里：主 agent 在一个 harness 阶段内**自己决定**要不要
/// 拆 todo、按需派 `sub_agent_*` 完成每一项、最后提交 StageDeliverable，替代旧的
/// generator 产 JSON 子任务 + 固定子任务循环。与 [`stage_charter`]（工具面 / gate
/// 要求）和 [`stage_discipline_reminder`]（提交纪律）配套使用。
pub fn stage_execution_prompt(stage_id: &str) -> String {
    format!(
        r#"## STAGE EXECUTION — you own this stage end-to-end

You are now working the `{stage_id}` stage of a structured operation. Decide how much process this stage needs, then drive it to a gated deliverable:

1. **Assess scope first.** If this stage is a quick confirmation or a single obvious action, do it directly — do NOT manufacture busywork or a multi-step plan you don't need.
2. **Plan only when it helps — and ONLY for this stage.** For a stage with real, multi-part work, call `update_plan` to lay out 2-5 concrete todos scoped to `{stage_id}` ONLY. Do NOT list other stages (e.g. recon / enumeration / vulnerability triage / exploitation / reporting) as todos and do NOT pre-list the whole engagement — the harness drives stage-to-stage transitions for you, and your plan here renders as `{stage_id}`'s own card. Keep exactly one `in_progress` at a time and mark each `completed` as you finish it. Skip `update_plan` entirely for trivial stages.
3. **Delegate the actual work.** For each todo that needs tool execution, dispatch the right `sub_agent_*` specialist (e.g. `sub_agent_pentester` for recon/scanning). Stay within this stage's allowed tool boundary (see the stage charter above) — the runtime blocks forbidden tools.
4. **Close the stage.** When the stage's objective is met (or you've recorded honest `skipped_checks` for anything unavailable), submit the StageDeliverable. The deterministic gate validates it and the runtime advances to the next stage for you — do NOT jump ahead.

Only plan and act for `{stage_id}`. Do not perform later stages."#
    )
}

/// Stage methodology playbook (设计 2026-06-11 · stage-level skill 注入).
///
/// 与 [`stage_charter`]（边界/否定式约束 + gate 要求）互补：charter 告诉 agent
/// 「不能用什么 + gate 查什么」，本函数注入「这个阶段**怎么高效做**」的正向方法论
/// —— 推荐工具序列、效率红线（如 target_intel 禁逐条 dig）、何时收口。内容来自
/// `resources/harness/stages/<stage>/methodology.md`（[`crate::harness::resources::stage_methodology_md`]），
/// 改 markdown 即改指导、0 Rust 改动。没写 playbook 的阶段返回空串（不追加段落）。
///
/// 这是**指导**而非硬门禁：确定性 gate 仍是唯一过关裁判，playbook 只为减少模型空转
/// / 越界尝试 / 低效循环（headless MiMo 实测：逐条 dig 22min+、target_intel 误试 nmap）。
pub fn stage_methodology(spec: &StageSpec) -> String {
    match crate::harness::resources::stage_methodology_md(spec.kind) {
        Some(md) if !md.trim().is_empty() => format!(
            "## STAGE PLAYBOOK — how to do `{stage}` efficiently (methodology)\n\n\
             {body}\n\n\
             This playbook is GUIDANCE, not a gate: the deterministic gate still decides \
             pass/fail. Follow the recommended sequence and stop conditions so you don't waste \
             turns or stray outside this stage.\n\n",
            stage = spec.id,
            body = md.trim(),
        ),
        _ => String::new(),
    }
}

/// Slim orchestration note for a stage whose work is delegated to a per-org
/// `specialist` via `stage_run` (设计 2026-06-15).
///
/// 与 [`stage_methodology`] 互斥：方法论 playbook（这个阶段「怎么做」的脏活）属于
/// **干活的 worker**（由 `stage_run` 的 `build_org_objective` 注入到 specialist 子
/// agent），主 agent 不再重复携带。主 agent 只需要这份编排提示：用 `stage_run` 按
/// org 扇出 → 有界 gap closure → 全过后收口，或预算耗尽后结束本请求 BLOCKED；新的
/// 用户 continuation 才能取得下一份有界预算。无 `specialist` 的阶段返回空串（主 agent
/// 自己干，仍走 [`stage_methodology`]）。
pub fn stage_specialist_orchestration(spec: &StageSpec) -> String {
    let Some(specialist) = spec
        .specialist
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return String::new();
    };
    format!(
        "## DELEGATE `{stage}` — fan out with `stage_run`\n\n\
         This stage has a per-org specialist (`{specialist}`). Do NOT collect the \
         intelligence yourself: call `stage_run` with your full in-scope organization \
         tree (the parent + subsidiaries you built in scoping). It runs `{specialist}` \
         once per org — each isolated and gated on its own evidence — and returns \
         `{{ passed, gaps[], retry_budget_exhausted }}`. If `passed` is false and \
         `retry_budget_exhausted=false`, call `stage_run` again with `orgs` set to ONLY \
         the blocked org(s). If `retry_budget_exhausted=true`, do not call `stage_run` \
         again in this request: stop and report the stage BLOCKED. A separate user \
         continuation receives a fresh bounded retry budget and resumes the saved worker \
         chain. Once every org passes, submit the `{stage}` StageDeliverable to close — coverage is read from \
         the DB the specialists populated, so you do not re-collect or hand-build it.\n\n",
        stage = spec.id,
        specialist = specialist,
    )
}

/// C6 · cross-stage evidence handoff context (Doc 3 §6.2 handoff).
///
/// Renders the stage's `inherits_evidence_from` so the executing agent knows
/// which evidence kinds upstream stages should have produced and can build on
/// them instead of re-collecting. Empty `inherits_evidence_from` → empty string
/// (no section emitted). Prepended to the subtask description right after the
/// stage charter.
pub fn stage_inherited_evidence(spec: &StageSpec) -> String {
    if spec.inherits_evidence_from.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## INHERITED EVIDENCE (handoff from prior stages)\n\n\
         This stage builds on evidence already collected upstream. Query existing \
         evidence first and reuse it; do not blindly re-run upstream tools:\n\n",
    );
    for inh in &spec.inherits_evidence_from {
        let kinds = if inh.evidence_kinds.is_empty() {
            "(all kinds)".to_string()
        } else {
            inh.evidence_kinds.join(", ")
        };
        s.push_str(&format!(
            "- from **{}**: {}\n",
            inh.stage_kind.as_str(),
            kinds
        ));
    }
    s.push('\n');
    s
}

/// Phase 2 ①③ seam (agent-facing): inject the authoritative in-scope asset set
/// (recon-populated `targets.scope='in'`) into the stage description so the
/// executing agent works through the real assets instead of guessing. Empty
/// `assets` → empty string (no recon data yet ⇒ no section, no behavior change).
/// Capped to keep the prompt bounded; the full set is always queryable via the
/// `list_in_scope_targets` tool.
pub fn render_in_scope_assets(assets: &[String]) -> String {
    if assets.is_empty() {
        return String::new();
    }
    const MAX_SHOWN: usize = 50;
    let total = assets.len();
    let mut s = String::from(
        "## IN-SCOPE ASSETS (from reconnaissance)\n\n\
         These in-scope assets were collected by recon and are authoritative for \
         coverage. Work through them; use `list_in_scope_targets` for their ids and \
         `query_target_data(target_id)` for per-asset detail:\n\n",
    );
    for a in assets.iter().take(MAX_SHOWN) {
        s.push_str(&format!("- {a}\n"));
    }
    if total > MAX_SHOWN {
        s.push_str(&format!(
            "\n(showing first {MAX_SHOWN} of {total} — call `list_in_scope_targets` for the full set)\n"
        ));
    }
    s.push('\n');
    s
}

/// Stage-aware wrapper over [`render_in_scope_assets`] (设计 2026-06-13).
///
/// scoping 是 ORG 层（判定「这个集团 org 建了没 / 子公司树完不完整」），不是
/// ASSET 层。把 recon 收集的 `targets.scope='in'` 资产清单注入 scoping prompt，
/// 会让上一轮或别的 org 的残留资产冒充「本次权威资产」污染纠名——scoping 时
/// `harness_org_id=None`，org 过滤关闭，整个工作区 + 历史 `project_path=''` 的
/// in-scope 资产会被全捞出来。所以 scoping 阶段一律不注入资产；其余阶段透传
/// 权威清单，行为零变更。
pub fn render_in_scope_assets_for_stage(stage_kind: StageKind, assets: &[String]) -> String {
    if stage_kind == StageKind::Scoping {
        return String::new();
    }
    render_in_scope_assets(assets)
}

/// C6 · real cross-stage evidence handoff. Unlike [`stage_inherited_evidence`]
/// (which only lists the *kinds* a stage declares it inherits), this injects the
/// **actual** gate-passed deliverable summaries produced by upstream stages this
/// run, looked up by `inherits_evidence_from`. `recorded` is keyed by
/// `StageKind::as_str()`. Returns an empty string when none of the inherited
/// stages have a recorded summary yet (e.g. they ran in a prior process, or the
/// DAG took a shortcut), in which case only the static kind hint is emitted.
pub fn render_inherited_handoff(
    spec: &StageSpec,
    recorded: &std::collections::HashMap<String, String>,
) -> String {
    if spec.inherits_evidence_from.is_empty() || recorded.is_empty() {
        return String::new();
    }
    let mut sections = String::new();
    for inh in &spec.inherits_evidence_from {
        if let Some(summary) = recorded.get(inh.stage_kind.as_str()) {
            sections.push_str(&format!(
                "### from {}\n{}\n\n",
                inh.stage_kind.as_str(),
                summary
            ));
        }
    }
    if sections.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## INHERITED EVIDENCE — ACTUAL UPSTREAM RESULTS\n\n\
         These concrete results were produced by upstream stages earlier in this \
         operation. Reuse them; do not re-collect what is already below:\n\n",
    );
    s.push_str(&sections);
    s
}

/// Generator prompt — decomposes a user task into ordered subtasks.
///
/// Equivalent to PentAGI's `generator.tmpl`.
pub fn generator_prompt() -> &'static str {
    r#"You are a task planning specialist for penetration testing and security engineering.

## YOUR ROLE

Given a user's task description, decompose it into a sequence of concrete, actionable subtasks.
Each subtask should be independently executable by a specialist agent.

## RULES

1. Create between 2 and 10 subtasks. More complex tasks need more subtasks.
2. Order subtasks logically — each subtask can depend on results from earlier ones.
3. Assign an appropriate specialist for each subtask:
   - "pentester" — scanning, exploitation, security testing
   - "coder" — writing code, scripts, exploits
   - "researcher" — information gathering, OSINT, documentation lookup
   - "analyzer" — code review, architecture analysis
   - "memorist" — knowledge retrieval and storage
   - "explorer" — codebase navigation
   - null — let the primary agent decide
4. Be specific in descriptions. Include expected inputs and outputs.
5. Consider the full workflow: reconnaissance → analysis → testing → reporting.

## PENETRATION TESTING METHODOLOGY

When the task involves testing a target, follow this standard methodology:

### Phase 1: Information Gathering
- Provider-backed target intelligence (asset/subdomain/DNS-adjacent facts,
  ASN/CT/OSINT, WHOIS) — ONLY for applicable targets
- For IP targets, skip domain-only subdomain/CT expectations entirely
- Port discovery (naabu first, masscan for larger ranges, nmap fallback/verification) to identify open services
- **CRITICAL**: Always verify what service is actually running on each confirmed open port using nmap -sV. Use httpx for HTTP liveness/metadata and whatweb only after the endpoint is confirmed HTTP(S). NEVER assume a service based on port number alone (e.g., port 8080 is NOT necessarily Tomcat).

### Phase 2: Service Enumeration
- HTTP service probing (httpx) for web services
- HTTP technology fingerprinting (whatweb, wappalyzer) only for confirmed HTTP(S) services; if several domains/vhosts share one IP:port, fingerprint each confirmed web origin separately because Host/SNI can change the stack
- Web crawling (katana) for content discovery
- JavaScript collection and analysis for SPAs

### Phase 3: Vulnerability Assessment
- Based on identified technologies, select appropriate test vectors
- Automated scanning (nuclei) with relevant templates
- Manual testing for logic vulnerabilities

### Phase 4: Reporting
- Summarize all findings with severity ratings
- Provide remediation recommendations

## IMPORTANT CONSTRAINTS

- Each subtask description MUST specify what tools to use and what NOT to assume
- Subtask descriptions should include verification steps (e.g., "verify the service type before proceeding")
- If a previous subtask found no results (e.g., no open ports), subsequent subtasks should handle that case

## HARNESS STAGE ASSIGNMENT (Phase 1 MVP — Operation Harness)

When a subtask falls into a known **harness stage**, attach a `harness_stage` field so
the runtime can validate the deliverable against deterministic gate checks. Tag each subtask
with the ONE harness stage it belongs to (the full operation DAG is supported).

**Harness stages** (pick the single best match; omit `harness_stage` entirely if none fit):

- `scoping` — define scope / rules of engagement / authorization boundary (no probing).
- `target_intel` — passive intel (zero-touch, no target contact): provider-backed asset/subdomain/DNS-adjacent/ASN/CT/OSINT survey via recon_map_assets, plus RDAP WHOIS via recon_lookup_whois; no scan-tool fallback. (情报收集)
- `external_attack_surface` — active recon that DEFINES the attack surface of approved hosts: DNS resolution, port scanning, service/version fingerprinting, HTTP probing, screenshots (host x port x service x live-web). Subdomains inherited from `target_intel` (do not re-enumerate). (资产测绘 / 攻击面 / 端口扫描)
- `enumeration` — content enumeration on the services mapped by EAS: JS collection + API endpoint extraction, directory/path discovery, parameter discovery. Do NOT re-port-scan (already done in EAS). (目录扫描 / JS-API / 参数发现)
- `vuln_triage` — FORMULAIC vulnerability scan: batch-run tool+dictionary/template techniques with an objective found/checked-empty/blocked observation. It writes evidence-backed observation seeds only; it can never create Candidate/Finding authority. (公式化漏洞扫描)
- `attack_candidate` — decide every server-seeded work item as `candidate` or evidence-backed `no_candidate`. The server derives immutable Candidate ids/plans/hashes/risk only after final Gate PASS; this reasoning stage runs no scan tools. (攻击候选合成)
- `verification` — controlled exploit validation / PoC confirmation of APPROVED candidates, approval-gated; each approved candidate reaches a terminal disposition (verified/refuted/blocked). (真打验证)
- `reporting` — synthesize the final report + attack/kill chain from collected evidence. (报告生成 / 修复建议)

(Red-team stages access_validation / internal_discovery / objective_pathing / objective_simulation / cleanup also exist; tag only when explicitly in scope.)

Add it like (replace the value with the matching stage):

```
"harness_stage": { "stage_kind": "external_attack_surface" }
```

If you are not sure, omit the field — the runtime will fall back to a deterministic
keyword-based backfill (so over-tagging is worse than under-tagging).

## OUTPUT FORMAT

Respond with ONLY a JSON object (no markdown fences, no explanation):

{
  "subtasks": [
    {
      "title": "Short descriptive title",
      "description": "Detailed description of what to do, expected inputs, and desired outputs",
      "agent": "pentester",
      "harness_stage": { "stage_kind": "external_attack_surface" }
    }
  ]
}

The `harness_stage` field is OPTIONAL — omit it for subtasks that don't match a known
stage (most subtasks today).
"#
}

/// Primary agent prompt for Task mode — executes a single subtask.
///
/// This wraps the subtask context around the main agent's capabilities.
/// Equivalent to PentAGI's `primary_agent.tmpl`.
#[allow(dead_code)]
pub fn primary_agent_subtask_prompt(
    subtask_title: &str,
    subtask_description: &str,
    execution_context: &str,
) -> String {
    primary_agent_subtask_prompt_with_agent(
        subtask_title,
        subtask_description,
        execution_context,
        None,
    )
}

/// Primary agent prompt with optional agent-type hint.
///
/// The Primary agent acts as a pure orchestrator (PentAGI-style): it delegates
/// work to specialist sub-agents via `sub_agent_*` tools and synthesizes their
/// results. The `agent_type` from Generator is a hint, not a hard constraint.
pub fn primary_agent_subtask_prompt_with_agent(
    subtask_title: &str,
    subtask_description: &str,
    execution_context: &str,
    agent_type: Option<&str>,
) -> String {
    let specialist_hint = match agent_type {
        Some("primary") | None => String::new(),
        Some(at) => format!(
            "\n**Suggested specialist**: `sub_agent_{at}` — prioritize calling this agent, \
             but use your judgment if a different specialist would be more effective.\n"
        ),
    };

    format!(
        r#"## TASK MODE — SUBTASK EXECUTION

You are the **Primary orchestrator** executing a subtask as part of a larger automated task.

### YOUR ROLE

You are a COORDINATOR. You delegate work to specialist sub-agents and synthesize their results.
You have access to sub_agent_* tools to invoke specialists — use them.
{specialist_hint}
### Current Subtask: {title}

{description}

### Previous Results

{context}

### AVAILABLE SPECIALISTS

Call these via their `sub_agent_*` tools:
- `sub_agent_pentester` — security scanning, exploitation, vulnerability assessment
- `sub_agent_coder` — code editing, script generation, diff application
- `sub_agent_researcher` — web research, documentation lookup, CVE investigation
- `sub_agent_memorist` — store/retrieve findings from long-term memory
- `sub_agent_installer` — install and configure penetration testing tools
- `sub_agent_adviser` — expert security consulting and risk assessment
- `sub_agent_explorer` — fast file search and codebase navigation
- `sub_agent_analyzer` — deep code analysis and architecture review
- `sub_agent_reporter` — generate structured security reports

### WORKFLOW

1. Analyze the subtask requirements
2. Delegate to the appropriate specialist(s) — you may call multiple agents sequentially
3. After each agent returns, decide if more work is needed
4. Synthesize results into a coherent summary

### RULES

1. **DELEGATE** — always use sub_agent_* tools. Do not try to run shell commands or edit files directly.
2. **MULTI-AGENT** — you may call multiple agents for one subtask (e.g., pentester → memorist to store findings).
3. **FOCUS** — complete only this specific subtask, not the entire parent task.
4. **EVIDENCE** — include concrete findings and evidence in your summary.

### OUTPUT FORMAT

After all specialists complete, provide:

**Actions Taken**: Which agents you called and why
**Findings**: Key results with evidence
**Next Steps**: Recommendations for subsequent subtasks
"#,
        specialist_hint = specialist_hint,
        title = subtask_title,
        description = subtask_description,
        context = if execution_context.is_empty() {
            "No previous subtasks completed yet.".to_string()
        } else {
            execution_context.to_string()
        },
    )
}

/// Refiner prompt — evaluates progress and adjusts the remaining plan.
///
/// Equivalent to PentAGI's `refiner.tmpl`.
pub fn refiner_prompt(execution_context: &str, remaining_subtasks_json: &str) -> String {
    format!(
        r#"You are a task plan refiner for penetration testing operations.

## YOUR ROLE

After each subtask completes, you evaluate the progress and decide whether the remaining plan needs adjustment.

## COMPLETED WORK

{context}

## REMAINING SUBTASKS

```json
{remaining}
```

## INSTRUCTIONS

Based on the completed results, decide:
1. Are any remaining subtasks now unnecessary? (e.g., already covered, or blocked)
2. Are new subtasks needed based on discoveries? (e.g., new attack surface found)
3. Is the overall task already complete?

## OUTPUT FORMAT

Respond with ONLY a JSON object (no markdown fences, no explanation):

{{
  "add": [
    {{
      "title": "New subtask title",
      "description": "What to do",
      "agent": "pentester"
    }}
  ],
  "remove": [0, 2],
  "modify": [
    {{
      "index": 1,
      "title": "Updated title",
      "description": "Updated description based on new findings"
    }}
  ],
  "reorder": [2, 0, 1],
  "complete": false
}}

- "add": new subtasks to append to the queue (empty array if none)
- "remove": 0-based indices of remaining subtasks to remove (empty array if none)
- "modify": changes to existing subtasks — only include fields that changed (empty array if none)
- "reorder": new ordering of remaining subtasks by their current indices (omit if no reorder needed)
- "complete": true if the task is fully done and remaining subtasks can be skipped

Operations are applied in order: reorder → modify → remove → add.
Prefer surgical modifications over removing+re-adding subtasks.
"#,
        context = execution_context,
        remaining = remaining_subtasks_json,
    )
}

/// Reporter prompt — generates the final task report.
///
/// Equivalent to PentAGI's `reporter.tmpl`.
pub fn reporter_prompt(execution_context: &str) -> String {
    format!(
        r#"You are a security assessment reporter.

## YOUR ROLE

Generate a comprehensive final report for a completed penetration testing task.

## COMPLETED SUBTASKS AND RESULTS

{context}

## REPORT FORMAT

Write a clear, structured report with:

1. **Executive Summary** — 2-3 sentence overview of what was done and key findings
2. **Scope** — what was tested
3. **Findings** — each finding with severity, description, evidence, and remediation
4. **Recommendations** — prioritized list of actions to improve security
5. **Conclusion** — overall assessment

Use markdown formatting. Be factual and precise. Reference specific evidence from the subtask results.
"#,
        context = execution_context,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 设计 2026-06-13：scoping 是 ORG 层、不是 ASSET 层。把 recon 资产清单注入
    /// scoping prompt 会让上一轮/别 org 的脏资产（如残留的 `*.moresec.cn`）冒充
    /// 「本次权威资产」污染纠名。`render_in_scope_assets_for_stage` 在 scoping
    /// 阶段必须返回空串；其余阶段照旧注入权威清单（行为零变更）。
    #[test]
    fn scoping_stage_never_injects_in_scope_assets() {
        use crate::harness::types::StageKind;
        let assets = vec!["moresec.cn".to_string(), "ai.moresec.cn".to_string()];
        // Scoping: ORG 层，绝不注入资产清单（防跨 engagement 污染）。
        assert_eq!(
            render_in_scope_assets_for_stage(StageKind::Scoping, &assets),
            ""
        );
        // 下游阶段仍拿到权威清单。
        let eas = render_in_scope_assets_for_stage(StageKind::ExternalAttackSurface, &assets);
        assert!(
            eas.contains("IN-SCOPE ASSETS"),
            "non-scoping stages must keep the authoritative in-scope list"
        );
        assert!(eas.contains("moresec.cn"));
    }

    /// The final stage-discipline directive must (1) force a parseable
    /// StageDeliverable JSON ending so the deterministic gate stops being
    /// skipped, and (2) tell the agent to stop + report instead of
    /// rabbit-holing on unavailable tools. Locks both intents in place.
    #[test]
    fn stage_discipline_reminder_forces_deliverable_and_stops_rabbit_holing() {
        let r = stage_discipline_reminder();
        // Deliverable forcing function (flag-robust: tool path or text path).
        assert!(r.contains("StageDeliverable"));
        assert!(
            r.contains("submit_stage_deliverable") || r.contains("```json"),
            "must instruct deliverable submission via the tool or a fenced json block"
        );
        assert!(r.contains("redo this entire stage"));
        // Boundary + stop-on-unavailable, no rabbit-holing (always present).
        assert!(r.contains("forbidden tool"));
        assert!(r.contains("STOP and record"));
        assert!(
            r.contains("spawn additional sub-agents"),
            "must forbid spawning more sub-agents to work around unavailable tools"
        );
    }

    /// Recency contract: the reminder is appended last, so it must explicitly
    /// state it overrides earlier output-format instructions (the base
    /// orchestrator prompt's prose "COMPLETION REQUIREMENTS").
    #[test]
    fn stage_discipline_reminder_announces_override() {
        let r = stage_discipline_reminder();
        assert!(r.to_lowercase().contains("overrides"));
        assert!(r.to_uppercase().contains("READ THIS LAST"));
    }

    #[test]
    fn specialist_orchestration_stops_after_stage_run_budget_exhaustion() {
        let spec = crate::harness::resources::load_embedded_stage_spec(
            crate::harness::types::StageKind::Enumeration,
        )
        .expect("load enumeration spec");
        let prompt = stage_specialist_orchestration(&spec);

        assert!(prompt.contains("retry_budget_exhausted=true"));
        assert!(prompt.contains("do not call `stage_run` again in this request"));
        assert!(prompt.contains("separate user continuation"));
    }

    #[test]
    fn mentor_prompt_renders_stage_boundary_context() {
        let context = MentorPromptContext {
            stage: Some("external_attack_surface".to_string()),
            agent_role: Some("prober (Recon Prober)".to_string()),
            allowed_tools: vec![
                "pentest_list_tools".to_string(),
                "pentest_run".to_string(),
                "wait_for_background_jobs".to_string(),
                "submit_stage_deliverable".to_string(),
            ],
            allowed_tool_types: vec![
                "recon/port-scan".to_string(),
                "recon/http".to_string(),
                "recon/visual".to_string(),
            ],
        };

        let prompt = mentor_user_prompt_with_context(
            "Close the current organization's EAS coverage gaps.",
            "pentest_run",
            3,
            "pentest_run({\"tool_name\":\"httpx\"})",
            &context,
        );

        assert!(prompt.contains("Current stage: external_attack_surface"));
        assert!(prompt.contains("Agent role: prober (Recon Prober)"));
        assert!(prompt.contains("Allowed visible tools: pentest_list_tools"));
        assert!(prompt.contains("Allowed scan categories: recon/port-scan"));
        assert!(prompt.contains("Close EAS coverage"));
        assert!(prompt.contains("Do NOT recommend exploitation"));
        assert!(prompt.contains("while staying inside the execution boundary"));
    }

    #[test]
    fn mentor_hard_guard_prioritizes_stage_boundary_over_model_advice() {
        let context = MentorPromptContext {
            stage: Some("external_attack_surface".to_string()),
            agent_role: Some("prober".to_string()),
            allowed_tools: vec![
                "pentest_run".to_string(),
                "wait_for_background_jobs".to_string(),
            ],
            allowed_tool_types: vec!["recon/http".to_string()],
        };

        let guard = mentor_hard_stage_guard(&context);

        assert!(guard.contains("Current stage: external_attack_surface"));
        assert!(guard.contains("Objective now: Close EAS coverage"));
        assert!(guard.contains("If the model advice below conflicts"));
        assert!(guard.contains("Do NOT recommend or follow exploitation"));
    }

    /// Coverage matrix (设计 2026-06-05): the charter must surface the stage's
    /// expected techniques + the per-cell coverage contract when set, and stay
    /// silent (no Coverage bullet) when the stage declares none.
    #[test]
    fn stage_charter_lists_expected_techniques_when_set() {
        use crate::harness::stage_spec::load_stage_spec_from_json;

        let with = load_stage_spec_from_json(
            r#"{"id":"vuln_triage","kind":"vuln_triage","risk_level":"high",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "expected_techniques":["WSTG-INPV-05","WSTG-ATHZ-04"]}"#,
        )
        .unwrap();
        let charter = stage_charter(&with, &ScopingPolicy::default());
        assert!(charter.contains("Coverage (per in-scope asset)"));
        assert!(charter.contains("WSTG-INPV-05"));
        assert!(charter.contains("WSTG-ATHZ-04"));
        // 分母覆盖契约（设计 2026-06-05-vuln-triage-technique-matrix §5）。
        assert!(charter.contains("tested_units"));
        assert!(charter.contains("total_units"));
        assert!(charter.contains("sampling_rationale"));

        let without = load_stage_spec_from_json(
            r#"{"id":"scoping","kind":"scoping","risk_level":"low",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#,
        )
        .unwrap();
        assert!(!stage_charter(&without, &ScopingPolicy::default())
            .contains("Coverage (per in-scope asset)"));
    }

    /// P5（2026-06-11）：声明 expected_techniques 的 stage，charter 必须教 agent 给
    /// claims/findings 打 technique 标注（派生 + 佐证）；expected_techniques 为空的
    /// stage 不渲染该教学（与 coverage_line 同生命周期）。
    #[test]
    fn stage_charter_mentions_technique_tagging_when_expected() {
        use crate::harness::stage_spec::load_stage_spec_from_json;

        let spec = load_stage_spec_from_json(
            r#"{"id":"target_intel","kind":"target_intel","risk_level":"low",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "expected_techniques":["GOLISH-INTEL-DNS","GOLISH-INTEL-WHOIS"]}"#,
        )
        .expect("spec parses");
        let charter = stage_charter(&spec, &ScopingPolicy::default());
        assert!(
            charter.contains("Tag claims/findings with `technique`"),
            "charter must explain technique tagging when expected_techniques set"
        );

        let without = load_stage_spec_from_json(
            r#"{"id":"scoping","kind":"scoping","risk_level":"low",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#,
        )
        .unwrap();
        assert!(!stage_charter(&without, &ScopingPolicy::default())
            .contains("Tag claims/findings with `technique`"));
    }

    /// Phase C 瘦身交付物（设计 2026-06-22）：facts_from_db_truth 阶段的 charter 用
    /// 「DB 自动裁决」的瘦覆盖指引——不教逐格填矩阵、不教 technique 标注，改教「跑工具
    /// 落库 + 仅对无源/被阻断技术报 blocked/not_applicable」。
    #[test]
    fn stage_charter_slim_coverage_when_facts_from_db_truth() {
        use crate::harness::stage_spec::load_stage_spec_from_json;

        let spec = load_stage_spec_from_json(
            r#"{"id":"target_intel","kind":"target_intel","risk_level":"low",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "facts_from_db_truth":true,
                "expected_techniques":["GOLISH-INTEL-DNS","GOLISH-INTEL-WHOIS"]}"#,
        )
        .expect("spec parses");
        let charter = stage_charter(&spec, &ScopingPolicy::default());
        // slim branch: DB auto-adjudicates; technique list still surfaced.
        assert!(
            charter.contains("auto-adjudicated from the DATABASE"),
            "facts_from_db_truth charter must use the slim coverage instruction"
        );
        assert!(charter.contains("GOLISH-INTEL-DNS"));
        // the cell-by-cell matrix + tagging busywork is NOT rendered for slim stages.
        assert!(!charter.contains("Coverage (per in-scope asset)"));
        assert!(!charter.contains("Tag claims/findings with `technique`"));
    }

    /// 2026-06-25 slim EAS closeout: EAS still surfaces the liveness technique,
    /// but now via the DB-truth slim coverage instruction instead of telling the
    /// agent to hand-fill the matrix.
    #[test]
    fn external_attack_surface_charter_surfaces_liveness_technique() {
        let spec = crate::harness::resources::load_embedded_stage_spec(
            crate::harness::types::StageKind::ExternalAttackSurface,
        )
        .expect("load eas spec");
        let charter = stage_charter(&spec, &ScopingPolicy::default());
        assert!(
            charter.contains("GOLISH-EAS-LIVENESS"),
            "EAS charter must surface the liveness technique to the agent"
        );
        assert!(charter.contains("auto-adjudicated from the DATABASE"));
        assert!(charter.contains("eas_discover_ports first"));
        assert!(charter.contains("nmap -sV for every confirmed open port"));
        assert!(charter.contains("eas_fingerprint_web_stack/whatweb"));
        assert!(charter.contains("WEB-FINGERPRINT"));
        assert!(charter.contains("SERVICE-FINGERPRINT not_applicable"));
        assert!(charter.contains("HTTP liveness alone"));
        assert!(!charter.contains("Coverage (per in-scope asset)"));
    }

    /// 阶段级方法论 playbook (设计 2026-06-11): `stage_methodology` 为有 playbook 的
    /// 阶段渲染「## STAGE PLAYBOOK」段并嵌入 markdown 正文，明确标注是「指导非 gate」；
    /// 没 playbook 的阶段（如 cleanup）返回空串。
    #[test]
    fn stage_methodology_renders_playbook_for_target_intel_and_empty_for_cleanup() {
        let ti = crate::harness::resources::load_embedded_stage_spec(
            crate::harness::types::StageKind::TargetIntel,
        )
        .expect("load target_intel spec");
        let m = stage_methodology(&ti);
        assert!(m.contains("## STAGE PLAYBOOK"));
        assert!(m.contains("target_intel"));
        // The key methodology fix must reach the agent: no scan-tool fallback in
        // target_intel.
        assert!(m.contains("recon_map_assets"));
        assert!(m.contains("recon_lookup_whois"));
        assert!(!m.contains("dig"));
        assert!(!m.contains("subfinder"));
        // Must be clearly framed as guidance, not a hard gate.
        assert!(m.contains("GUIDANCE") || m.contains("not a gate"));

        // A stage without a methodology file → empty string (no section appended).
        let cleanup = crate::harness::resources::load_embedded_stage_spec(
            crate::harness::types::StageKind::Cleanup,
        )
        .expect("load cleanup spec");
        assert!(stage_methodology(&cleanup).is_empty());
    }

    /// scoping 人工确认硬门禁 (设计 2026-06-06 §3.4): charter 的 scoping 段在
    /// `require_human_scope_approval` 时点明硬门禁 + scope_human_approved claim;
    /// 关 (smoke) 时不出现, 与 gate hook 注入条件一致.
    #[test]
    fn stage_charter_scoping_appends_human_gate_when_policy_requires() {
        use crate::harness::stage_spec::load_stage_spec_from_json;

        let scoping = load_stage_spec_from_json(
            r#"{"id":"scoping","kind":"scoping","risk_level":"low",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#,
        )
        .unwrap();
        // Default policy requires human approval → charter spells out the hard gate.
        let gated = ScopingPolicy::default();
        assert!(gated.require_human_scope_approval);
        let c = stage_charter(&scoping, &gated);
        assert!(c.contains("scope_human_approved"));
        assert!(c.contains("ask_human(input_type=\"scope_review\")"));
        assert!(c.contains("EXACTLY ONCE"));
        assert!(c.contains("Never open a second scope_review"));
        assert!(c.contains("NON-EMPTY"));
        assert!(c.contains("do NOT manufacture an empty target-table review"));

        let red_team = ScopingPolicy {
            require_unit_candidates: true,
            ..ScopingPolicy::default()
        };
        let red_team_charter = stage_charter(&scoping, &red_team);
        assert!(red_team_charter.contains("subsidiary_scope"));
        assert!(red_team_charter.contains("parent/root-only"));
        assert!(red_team_charter.contains("propose_candidates"));
        assert!(red_team_charter.contains("unit_review"));

        // Gate off (smoke) → no human-approval line.
        let off = ScopingPolicy {
            require_human_scope_approval: false,
            ..ScopingPolicy::default()
        };
        assert!(!stage_charter(&scoping, &off).contains("scope_human_approved"));
    }
}
