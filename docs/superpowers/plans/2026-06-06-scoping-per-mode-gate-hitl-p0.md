# Scoping 按 Task 模式分流 + 人工确认硬门禁 · P0 实现计划

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任务实现此计划。每个 Task 独立 commit，TDD 优先（先写失败测试）。
>
> **设计文档：** `docs/design/2026-06-06-scoping-per-mode-gate-hitl.md`（已含 2026-06-06 用户决议：除 smoke 外都启硬门禁；**pentest 必须建/选 organization**，`subject_kind=organization`、`write_organizations=true`，与前端 `NewEngagementDialog` org-first 一致）。

**目标：** 让 harness 的 scoping 阶段按 profile（task 模式）走差异化流程——确认主体 / 列资产或单位候选给人确认编辑 / 人确认通过才进 target_intel（gate 硬门禁）。
**架构：** profile JSON 新增 `scoping_policy` 块作为唯一差异源；prompt 构造与 gate hook 都读它分流；HITL 复用 `ask_human` 通道加结构化 `input_type`；AI 经增强后的 `manage_targets` / 新 `manage_organizations` 写库。
**技术栈：** Rust（serde / sqlx / 既有 harness gate DSL）、React + TS（ts-rs 同步类型）、cargo nextest / vitest。

---

## 文件结构（先锁分解）

| 文件 | 职责 | 动作 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/harness/profile.rs` | `ScopingPolicy` / `SubjectKind` / `AssetConfirmation` 类型 + `Profile.scoping_policy` 字段 | 改 |
| `resources/harness/profiles/{pentest,red_team,assessment,bug_bounty,cloud_assessment,smoke}.json` | 各模式 `scoping_policy` 配置 | 改 |
| `backend/crates/golish-agent-kit/src/harness/feature_flags.rs`（或现有 flag 模块） | `scoping_human_gate_enabled()` | 改/查 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | gate hook 按 policy 注入硬门禁规则；`synthesize_stage_subtask` 接 profile | 改 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs` | `stage_charter` scoping 段按 policy 分流 | 改 |
| `backend/crates/golish-pentest-app/src/pentest_bridge/manage_targets.rs` | `add` 支持 scope/org_id + `set_scope` action | 改 |
| `backend/crates/golish-pentest-app/src/pentest_bridge/manage_organizations.rs` | 新 agent 工具（list/propose_candidates/create/update_profile） | 新建 |
| `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs` | 导出/注册新工具 | 改 |
| `backend/crates/golish-agent-runtime/src/execution_mode/modes/task.rs` + `tool_list.rs` | 把新工具放进 Task specialist 工具集 | 改 |
| `frontend/components/AIChatPanel/AskHumanInline.tsx`（+ 新 `ScopeReviewTable.tsx`） | `scope_review`/`unit_review` 可编辑表渲染 + 回传 | 改/新建 |
| `frontend/lib/generated/*` | ts-rs 同步（若新增跨 IPC 类型） | 生成 |

> 依赖顺序：Task 1（类型）→ Task 2（配置）→ Task 3（gate）/Task 4（prompt）/Task 5,6（工具）可并行 → Task 7（前端）→ Task 8（收口）。

---

## Task 1 · `ScopingPolicy` 数据模型

**文件：** `backend/crates/golish-agent-kit/src/harness/profile.rs`

**步骤 1.1** 先读现状确认 `Profile` 结构与 derive：

```bash
sed -n '1,90p' backend/crates/golish-agent-kit/src/harness/profile.rs   # 用 Read 工具读，确认 Profile 字段顺序 / derive(Deserialize) / approval_policy 的写法做参照
```

**步骤 1.2** 新增类型（放在 `Profile` 定义之前）：

```rust
/// scoping 阶段的 per-profile 行为策略（设计 2026-06-06-scoping-per-mode-gate-hitl §3.2）。
/// 全字段 serde default：旧 profile JSON 无此块时取保守安全默认（要求人工确认）。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ScopingPolicy {
    /// 是否必须确认主体。
    pub require_subject: bool,
    /// 主体形态。
    pub subject_kind: SubjectKind,
    /// 红队专用：先产出「单位名称候选」交人判断。
    pub require_unit_candidates: bool,
    /// 资产确认方式。
    pub asset_confirmation: AssetConfirmation,
    /// 硬门禁开关：true 时 scoping 通过前必须有人工确认 claim。
    pub require_human_scope_approval: bool,
    /// scoping 是否落组织（红队 true）。
    pub write_organizations: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    #[default]
    None,
    Freetext,
    Organization,
    OrganizationOrFreetext,
    CloudTenant,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssetConfirmation {
    None,
    Auto,
    #[default]
    Interactive,
}

impl Default for ScopingPolicy {
    /// 保守安全默认（无 scoping_policy 的旧 profile）：要求人工确认 scope。
    fn default() -> Self {
        Self {
            require_subject: false,
            subject_kind: SubjectKind::Freetext,
            require_unit_candidates: false,
            asset_confirmation: AssetConfirmation::Interactive,
            require_human_scope_approval: true,
            write_organizations: false,
        }
    }
}
```

**步骤 1.3** 给 `Profile` 加字段（在结构体内，紧跟 `approval_policy` 后）：

```rust
    /// scoping 阶段 per-profile 策略（设计 2026-06-06）。缺省 = ScopingPolicy::default()。
    #[serde(default)]
    pub scoping_policy: ScopingPolicy,
```

**步骤 1.4（测试，先写后实现亦可——本 Task 类型新增，测试紧随）** 在 `profile.rs` 的 `#[cfg(test)] mod tests` 加：

```rust
#[test]
fn scoping_policy_defaults_when_absent() {
    // 旧 profile JSON 无 scoping_policy → 取安全默认（require_human_scope_approval=true）。
    let json = r#"{"id":"x","display_name":"X","max_authorization":"active_recon",
        "allowed_stage_kinds":["scoping"],"forbidden_stage_kinds":[],
        "approval_policy":{"before_active_scan":true,"before_scope_expansion":true},
        "cleanup_required":false,"evidence_required":true}"#;
    let p: Profile = serde_json::from_str(json).unwrap();
    assert!(p.scoping_policy.require_human_scope_approval);
    assert_eq!(p.scoping_policy.asset_confirmation, AssetConfirmation::Interactive);
}

#[test]
fn scoping_policy_parses_explicit_block() {
    let json = r#"{"id":"red_team","display_name":"Red Team","max_authorization":"post_exploit_red_team",
        "allowed_stage_kinds":["scoping"],"forbidden_stage_kinds":[],
        "approval_policy":{"before_active_scan":true,"before_scope_expansion":true},
        "cleanup_required":true,"evidence_required":true,
        "scoping_policy":{"require_subject":true,"subject_kind":"organization",
            "require_unit_candidates":true,"asset_confirmation":"interactive",
            "require_human_scope_approval":true,"write_organizations":true}}"#;
    let p: Profile = serde_json::from_str(json).unwrap();
    assert_eq!(p.scoping_policy.subject_kind, SubjectKind::Organization);
    assert!(p.scoping_policy.require_unit_candidates);
    assert!(p.scoping_policy.write_organizations);
}
```

> 若 `Profile` 还有其它必填字段，按步骤 1.1 读到的实际字段补进上面两个 JSON，使其能反序列化。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit scoping_policy --status-level fail
# 预期：2 passed, 0 skipped
cargo clippy -p golish-agent-kit --all-targets -- -D warnings   # 预期 exit 0
```
**提交：** `feat(harness): add ScopingPolicy per-profile scoping config`

---

## Task 2 · 6 个 profile JSON 加 `scoping_policy`

**文件：** `resources/harness/profiles/*.json`

**步骤 2.1** 按设计 §3.2 表，给每个文件加 `scoping_policy` 块（在 `evidence_required` 同级）。示例 —— `pentest.json`：

```json
  "cleanup_required": false,
  "evidence_required": true,
  "scoping_policy": {
    "require_subject": true,
    "subject_kind": "organization",
    "require_unit_candidates": false,
    "asset_confirmation": "interactive",
    "require_human_scope_approval": true,
    "write_organizations": true
  }
```

`red_team.json`：
```json
  "scoping_policy": {
    "require_subject": true,
    "subject_kind": "organization",
    "require_unit_candidates": true,
    "asset_confirmation": "interactive",
    "require_human_scope_approval": true,
    "write_organizations": true
  }
```

`assessment.json` / `bug_bounty.json` / `cloud_assessment.json`（cloud 的 `subject_kind` 用 `cloud_tenant`、`require_subject:true`；assessment/bug_bounty 用 `require_subject:false`、`subject_kind:"freetext"`）：均 `require_human_scope_approval: true`、`asset_confirmation: "interactive"`、`require_unit_candidates: false`、`write_organizations: false`。

`smoke.json`（**唯一豁免**）：
```json
  "scoping_policy": {
    "require_subject": false,
    "subject_kind": "none",
    "require_unit_candidates": false,
    "asset_confirmation": "none",
    "require_human_scope_approval": false,
    "write_organizations": false
  }
```

**步骤 2.2** 确认嵌入加载测试仍过（项目有 `all_*_profiles_load` / `EMBEDDED_PROFILE_IDS` 之类）：先 `Grep "EMBEDDED_PROFILE_IDS\|profiles_load" backend/crates/golish-agent-kit/src/harness/` 找到测试，跑它。

**验证：**
```bash
for f in resources/harness/profiles/*.json; do python3 -m json.tool "$f" >/dev/null && echo "$f OK"; done
cd backend && cargo nextest run -p golish-agent-kit profile --status-level fail   # 嵌入 profile 加载测试全过
```
**提交：** `feat(harness): configure per-mode scoping_policy in 6 profiles`

---

## Task 3 · gate 硬门禁注入（按 policy）

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`（gate hook）；feature flag 模块。

**步骤 3.1** 读 `apply_harness_gate_hook`（约 L1405-1618）确认：① 它在哪里拿到 `StageSpec`；② 调 `validate_stage_gate_with_context` 的位置；③ 已加载的 `Profile`（`exec_ctx.harness_profile_id` → `load_embedded_profile`）。确认 `StageSpec` derive `Clone`、`gate_rules: Vec<GateRule>` 字段可变（`Grep "pub gate_rules" backend/crates/golish-agent-kit/src/harness/stage_spec.rs`）。

**步骤 3.2** feature flag：`Grep "fn sprint_skeleton_enforcement_enabled" backend/crates/golish-agent-kit/src` 找到现有 flag 写法，照搬新增：

```rust
/// scoping 人工确认硬门禁灰度开关（设计 2026-06-06 §6 R1）。默认开启。
pub fn scoping_human_gate_enabled() -> bool {
    std::env::var("GOLISH_SCOPING_HUMAN_GATE")
        .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}
```

**步骤 3.3** 在 hook 内、`validate_stage_gate_with_context` 调用前，按 policy 注入规则（用 spec 的可变副本）：

```rust
use crate::harness::gate::rule_engine::{Collection, GateRule, ItemField, OnFail, Pred};

// scoping 人工确认硬门禁（设计 2026-06-06 §3.4）：除 smoke 外（policy.require_human_scope_approval）
// 要求 deliverable 至少 1 条 kind="scope_human_approved" 的 claim，否则不许离开 scoping。
let mut effective_spec = spec.clone();
if matches!(stage_kind, crate::harness::types::StageKind::Scoping)
    && crate::harness::feature_flags::scoping_human_gate_enabled()
    && profile
        .as_ref()
        .map(|p| p.scoping_policy.require_human_scope_approval)
        .unwrap_or(false)
{
    effective_spec.gate_rules.push(GateRule::CountAtLeast {
        over: Collection::Claims,
        filter: Some(Pred::Eq {
            field: ItemField::Kind,
            value: "scope_human_approved".to_string(),
        }),
        min: 1,
        on_fail: OnFail {
            reason: "scope must be human-confirmed before leaving scoping".to_string(),
            hints: vec![
                "call ask_human(input_type=\"scope_review\") and let the user confirm/edit the target list".to_string(),
                "after the user approves, add a claim {kind:\"scope_human_approved\", subject:<engagement subject>} that cites the ask_human request_id".to_string(),
            ],
            repair_tool_calls: vec!["ask_human".to_string()],
            missing_evidence_kinds: vec![],
        },
    });
}
// 用 effective_spec 替换原 spec 传入 validate_stage_gate_with_context(...)
```

> 按步骤 3.1 实际变量名调整（`stage_kind` / `profile` / `spec` 可能叫别的名字）。`profile` 若 hook 内尚未加载，用 `load_embedded_profile(exec_ctx.harness_profile_id)` 取。

**步骤 3.4（测试）** 在 `harness/gate/mod.rs` tests（或 execute 的 gate 测试模块）加直接针对规则的单测（不依赖 hook 私有路径，直接验证规则语义）：

```rust
#[test]
fn scoping_human_gate_blocks_without_approval_claim_and_passes_with() {
    use super::super::stage_spec::load_stage_spec_from_json;
    use super::super::types::StageClaim;
    use super::rule_engine::{Collection, GateRule, ItemField, OnFail, Pred};
    use golish_pentest::evidence_ledger::EvidenceAuditId;

    let mut spec = load_stage_spec_from_json(include_str!(
        "../../../../../../resources/harness/stages/scoping.json"
    )).unwrap();
    spec.gate_rules.push(GateRule::CountAtLeast {
        over: Collection::Claims,
        filter: Some(Pred::Eq { field: ItemField::Kind, value: "scope_human_approved".into() }),
        min: 1,
        on_fail: OnFail { reason: "scope must be human-confirmed".into(), hints: vec![], repair_tool_calls: vec![], missing_evidence_kinds: vec![] },
    });

    // 仅 scope_confirmed（无 human_approved）→ BLOCK。
    let mut d = StageDeliverable {
        stage_id: "scoping".into(), stage_run_id: Uuid::new_v4(),
        claims: vec![StageClaim { kind: "scope_confirmed".into(), subject: "example.com".into(), summary: "x".into(), evidence_ids: vec![] }],
        evidence_refs: vec![], skipped_checks: vec![], findings: vec![], required_checks_done: vec![], coverage: vec![],
    };
    assert!(!validate_stage_gate(&d, &spec, None).allowed);

    // 加一条 scope_human_approved → PASS。
    d.claims.push(StageClaim { kind: "scope_human_approved".into(), subject: "example.com".into(), summary: "user approved 3 targets".into(), evidence_ids: vec![] });
    assert!(validate_stage_gate(&d, &spec, None).allowed);
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit scoping_human_gate --status-level fail   # 1 passed
cargo nextest run -p golish-agent-kit -p golish-agent-app --status-level fail                 # 全绿回归
cargo clippy -p golish-agent-kit --all-targets -- -D warnings
```
**提交：** `feat(harness): inject human-scope-approval gate for scoping per profile`

---

## Task 4 · scoping prompt 按 policy 分流

**文件：** `task_orchestrator/prompts/mod.rs`（`stage_charter`）；`task_orchestrator/subtask_phases/execute.rs`（`synthesize_stage_subtask`）。

**步骤 4.1** 读 `stage_charter`（约 L48-172，scoping 段 L118-122）与 `synthesize_stage_subtask`（K::Scoping 约 L1820-1827），确认调用方能传 `&Profile`（调用点已有 `exec_ctx.harness_profile_id`，加载后传入）。

**步骤 4.2** 给 `synthesize_stage_subtask` 增加 `policy: &ScopingPolicy` 参数，scoping 分支按 policy 生成指令：

```rust
K::Scoping => {
    let mut steps = String::new();
    if policy.require_subject {
        steps.push_str(match policy.subject_kind {
            SubjectKind::Organization | SubjectKind::OrganizationOrFreetext => {
                if policy.write_organizations && !policy.require_unit_candidates {
                    // pentest: org-first, no candidate discovery — create/select the single subject org so targets can link to it.
                    "1) Identify the engagement subject organization; create or select it via manage_organizations(action=\"create\"/\"list\") and CONFIRM it with the user (org-first: every target must be linked to this organization_id). "
                } else {
                    "1) Identify and CONFIRM the engagement subject (the target organization). "
                }
            }
            SubjectKind::CloudTenant =>
                "1) Identify and CONFIRM the cloud tenant/account that is the engagement subject. ",
            _ => "1) State the engagement subject. ",
        });
    }
    if policy.require_unit_candidates {
        steps.push_str("2) Call manage_organizations(action=\"propose_candidates\") to list candidate unit/organization names (subsidiaries, aliases), then ask_human(input_type=\"unit_review\") so the user can judge/edit them; create confirmed orgs with manage_organizations(action=\"create\"). ");
    }
    if matches!(policy.asset_confirmation, AssetConfirmation::Interactive) {
        steps.push_str("3) Parse the user input into a candidate target list (mark in/out of scope), call ask_human(input_type=\"scope_review\") so the user can add/remove/edit, and ONLY AFTER approval write them via manage_targets(action=\"add\", with scope/organization_id). ");
    }
    if policy.require_human_scope_approval {
        steps.push_str("4) After human approval, record a claim {kind:\"scope_human_approved\", subject:<engagement subject>} citing the ask_human request_id, then submit_stage_deliverable. ");
    }
    steps.push_str("Do NOT perform any active scanning in this stage.");
    (
        "Scope & Authorization Confirmation",
        format!("Confirm and document the engagement scope for `{target}`. {steps}"),
        "pentester",
    )
}
```

**步骤 4.3** `stage_charter` 接 `&Profile`：scoping 段在「empty evidence OK」之外，追加一行随 policy 变化的提醒（require_human_scope_approval 时强调「submit 前必须有人确认 claim」）。调用方 `stage_charter(&spec)` 全部改 `stage_charter(&spec, &profile)`（用 `Grep "stage_charter(" backend/crates/golish-agent-kit/src` 找全调用点同步改）。

**步骤 4.4（测试）** 在 `execute.rs`（或 prompts）tests 断言文案随 policy 变化：

```rust
#[test]
fn scoping_subtask_prompt_varies_by_policy() {
    use crate::harness::profile::{AssetConfirmation, ScopingPolicy, SubjectKind};
    let red = ScopingPolicy { require_subject: true, subject_kind: SubjectKind::Organization,
        require_unit_candidates: true, asset_confirmation: AssetConfirmation::Interactive,
        require_human_scope_approval: true, write_organizations: true };
    let s = synthesize_stage_subtask(StageKind::Scoping, "acme corp", &red);
    assert!(s.description.contains("unit_review"));
    assert!(s.description.contains("scope_human_approved"));

    let smoke = ScopingPolicy { require_human_scope_approval: false,
        asset_confirmation: AssetConfirmation::None, ..ScopingPolicy::default() };
    let s2 = synthesize_stage_subtask(StageKind::Scoping, "x", &smoke);
    assert!(!s2.description.contains("scope_human_approved"));
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit scoping_subtask_prompt --status-level fail
cargo nextest run -p golish-agent-kit -p golish-agent-app --status-level fail   # 回归
cargo clippy -p golish-agent-kit --all-targets -- -D warnings
```
**提交：** `feat(harness): branch scoping prompt by scoping_policy`

---

## Task 5 · `manage_targets` 支持 scope / organization_id + `set_scope`

**文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/manage_targets.rs`

**步骤 5.1** `parameters()` 的 add item 增加 `scope` 与 `organization_id`，并加 `set_scope` 到 action enum：

```rust
"action": { "type": "string", "enum": ["add", "list", "update_status", "update_recon", "set_scope"], ... },
// 在 targets[].items.properties 内追加：
"scope": { "type": "string", "enum": ["in", "out"], "description": "Scope (default in)" },
"organization_id": { "type": "string", "description": "Owning organization UUID (optional)" }
```

**步骤 5.2** `add` 分支：解析 item 的 `scope`/`organization_id`，透传给 `target_add`（现把对应位置传 `None`，L156-169）。先 `Grep "fn target_add" backend/crates/golish-app-core/src/ports/recon/targets.rs` 确认参数顺序，再传：

```rust
let scope = item.get("scope").and_then(|v| v.as_str()); // "in"/"out"
let org_id = item.get("organization_id").and_then(|v| v.as_str());
// target_add(name, value, tt, scope, /*grp*/None, /*owner*/None, /*tw*/None, None, org_id, Some(&project_path), "discovered", parent_id)
```
> 按实际 `ReconTargetsPort::target_add` 签名对齐（其 `scope`/`organization_id` 形参类型——`Option<&str>` 或枚举——以读到的为准）。

**步骤 5.3** 新增 `set_scope` 分支（`target_id` + `scope` → `target_update` 仅改 scope）。复用 `golish-recon-app` 的 `target_update` 或 port 等价方法；必须带 `project_path` 做 IDOR 校验（参照 `target_update` 的 `ensure_scoped_found`）。

**步骤 5.4（测试）** 加单测（若该 crate 有 sqlx 测试夹具则用之；否则至少测 `parameters()` schema 含新字段 + add 解析 scope/org_id 不 panic）：

```rust
#[test]
fn manage_targets_schema_exposes_scope_and_set_scope() {
    let tool = ManageTargetsTool::new(/* mock pool or skip if needs DB */);
    let p = tool.parameters();
    let actions = p["properties"]["action"]["enum"].as_array().unwrap();
    assert!(actions.iter().any(|a| a == "set_scope"));
    assert!(p["properties"]["targets"]["items"]["properties"].get("scope").is_some());
    assert!(p["properties"]["targets"]["items"]["properties"].get("organization_id").is_some());
}
```
> 若 `ManageTargetsTool::new` 需真实 `PgPool`，把该断言改为对 `parameters()` 的纯函数测试（将 schema 抽成独立 `fn manage_targets_parameters() -> Value` 再测），避免 DB 依赖。

**验证：**
```bash
cd backend && cargo nextest run -p golish-pentest-app manage_targets --status-level fail
cargo clippy -p golish-pentest-app --all-targets -- -D warnings
```
**提交：** `feat(pentest): manage_targets supports scope/organization_id + set_scope`

---

## Task 6 · 新 `manage_organizations` agent 工具

**文件：** 新建 `backend/crates/golish-pentest-app/src/pentest_bridge/manage_organizations.rs`；改 `pentest_bridge/mod.rs` 导出。

**步骤 6.1** 先读既有后端签名：`organization_create` / `organization_list` / `organization_update_profile` / `organization_candidates_upsert`（`organizations/mod.rs:97-221`）与其 repo 层（`golish_db::repo::organizations`），确认能在 pentest-app 直接调 repo（参照 `manage_targets` 用 `PgReconTargetsAdapter` 的方式：找 organizations 的等价 adapter/port，`Grep "organizations" backend/crates/golish-app-core/src/ports`；若无 port，则按 `manage_targets` 模式直接用 `golish_db::repo::organizations`）。

**步骤 6.2** 实现工具（actions：`list` / `propose_candidates` / `create` / `update_profile`）：

```rust
//! `ManageOrganizationsTool` — AI 在 scoping（尤其红队）列/建甲方单位。
use std::path::Path; use std::sync::Arc;
use anyhow::Result; use serde_json::{json, Value}; use sqlx::PgPool; use uuid::Uuid;
use golish_core::Tool;

pub struct ManageOrganizationsTool { pool: Arc<PgPool> }
impl ManageOrganizationsTool { pub fn new(pool: Arc<PgPool>) -> Self { Self { pool } } }

#[async_trait::async_trait]
impl Tool for ManageOrganizationsTool {
    fn name(&self) -> &'static str { "manage_organizations" }
    fn description(&self) -> &'static str {
        "Manage target organizations/units during scoping. 'list' existing orgs; 'propose_candidates' to record candidate unit names for human review; 'create' to add a confirmed org; 'update_profile' to attach domains/ip_ranges/asns/scope_rules."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","propose_candidates","create","update_profile"]},
            "name":{"type":"string"},
            "parent_id":{"type":"string"},
            "organization_id":{"type":"string"},
            "candidates":{"type":"array","items":{"type":"object","properties":{
                "name":{"type":"string"},"reason":{"type":"string"}}}},
            "profile":{"type":"object","description":"domains/ip_ranges/asns/email_domains/scope_rules patch"}
        },"required":["action"]})
    }
    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let pp = workspace.to_string_lossy().to_string();
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("list") {
            "list" => { /* repo::organizations::list(pool, &pp) → json 数组 {id,name,...} */ Ok(json!({"action":"list"})) }
            "create" => { /* repo::organizations::create(pool,&pp,name,parent_id,"","") */ Ok(json!({"action":"create"})) }
            "propose_candidates" => { /* upsert_organization_candidates_for_org(pool, org_id, candidates) */ Ok(json!({"action":"propose_candidates"})) }
            "update_profile" => { /* repo::organizations::update_profile(pool, org_id, patch) */ Ok(json!({"action":"update_profile"})) }
            a => Ok(json!({"error": format!("unknown action {a}")})),
        }
    }
}
```
> 把每个分支的注释替换为按步骤 6.1 读到的真实 repo 调用 + 审计日志（`PentestAudit::started`，参照 `manage_targets`）。IDOR：所有写操作绑定 `pp`（project_path）。

**步骤 6.3** `mod.rs` 导出 `ManageOrganizationsTool`。

**步骤 6.4（测试）** schema 纯函数测试（同 Task 5 思路，避免 DB）：断言 `parameters()` 含 4 个 action 与 `candidates` 字段。

**验证：**
```bash
cd backend && cargo nextest run -p golish-pentest-app manage_organizations --status-level fail
cargo clippy -p golish-pentest-app --all-targets -- -D warnings
```
**提交：** `feat(pentest): add manage_organizations agent tool`

---

## Task 7 · 注册新工具到 Task specialist 工具集

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/modes/task.rs`、`tool_list.rs`。

**步骤 7.1** `Grep "ManageTargetsTool\|manage_targets" backend/crates/golish-agent-runtime backend/crates/golish-pentest-app/src/pentest_bridge` 找到工具被装配进 Task specialist（depth>0）的位置，照同一处把 `ManageOrganizationsTool::new(pool)` 也加进去。

**步骤 7.2（测试）** 若 `tool_list.rs` 有「task specialist 工具集包含 manage_targets」的断言测试，追加断言含 `manage_organizations`。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-runtime -p golish-agent-app tool_list --status-level fail
cargo clippy -p golish-agent-runtime --all-targets -- -D warnings
```
**提交：** `feat(runtime): expose manage_organizations to task specialists`

---

## Task 8 · 前端 `scope_review` / `unit_review` 可编辑确认表

**文件：** `frontend/components/AIChatPanel/AskHumanInline.tsx`（分支）；新建 `frontend/components/AIChatPanel/ScopeReviewTable.tsx`。

**步骤 8.1** 读 `AskHumanInline.tsx` 确认它如何拿到 `input_type` / `context` 以及如何回传（找发回 `ApprovalDecision`/`reason` 的调用，`Grep "input_type\|AskHuman" frontend/components/AIChatPanel`）。

**步骤 8.2** 新建 `ScopeReviewTable.tsx`：props `{ kind: "scope_review" | "unit_review"; initial: Row[]; onConfirm(rows): void; onSkip(): void }`，渲染可增删改表格：
- `scope_review` 列：value、type(domain/ip/cidr/url/wildcard)、scope(in/out)；
- `unit_review` 列：name、aliases、domains。
「确认」→ `onConfirm(rows)`；「跳过」→ `onSkip()`。

```tsx
// 关键回传：把编辑后的 rows 作为 JSON 字符串放进 ask_human 的 reason，approved=true
const handleConfirm = () => respondAskHuman(requestId, { approved: true, reason: JSON.stringify(rows) });
const handleSkip = () => respondAskHuman(requestId, { approved: false, reason: "" });
```
> `respondAskHuman` 用 `AskHumanInline` 现有的回传函数名（步骤 8.1 读到的实际 API）。

**步骤 8.3** `AskHumanInline.tsx`：当 `input_type ∈ {"scope_review","unit_review"}` 时，解析 `context`（AI 提议的 JSON）为 `initial` 并渲染 `<ScopeReviewTable/>`，否则走原有 freetext/options 渲染。

**步骤 8.4（测试）** `frontend/components/AIChatPanel/ScopeReviewTable.test.tsx`（Vitest）：渲染初始行 → 增/删/改一行 → 点确认 → 断言 `onConfirm` 收到编辑后的数组（且能 `JSON.parse`）。

**验证：**
```bash
just check-fe        # biome + tsc，exit 0
just test-fe         # 含新 ScopeReviewTable.test，全过
```
**提交：** `feat(ui): editable scope_review/unit_review confirmation table`

---

## Task 9 · 收口（集成 + 全量验证 + 进度）

**步骤 9.1** 若 Task 1/8 引入跨 IPC 新类型（policy/HITL row），按 `docs/development.md` 跑 ts-rs 同步，确认 `frontend/lib/generated/` 更新、无手改。

**步骤 9.2** 全量验证：
```bash
just precommit       # = just check + just test，必须全绿
```
**步骤 9.3** 更新 `agent-progress.md`（本轮目标/已跑命令+输出/证据/风险/下一步）与 `feature_list.json`（本 feature 由 in_progress → passing 并填 evidence；若 precommit 未全绿则留 in_progress 或 blocked 写明）。
**步骤 9.4** 人工 smoke：分别用 pentest / red_team / smoke 三个模式跑一次 scoping，确认：pentest 弹 scope_review、red_team 先弹 unit_review、smoke 不弹且直接进 target_intel；未确认时 gate BLOCK 停在 scoping。
**提交：** `chore: wire scoping per-mode HITL end-to-end + progress`

---

## 自检（writing-plans）

**1. 规格覆盖度（对照设计 §3）：** §3.2 policy 模型→T1/T2；§3.3 prompt 分流→T4；§3.4 硬门禁→T3；§3.5 写入工具→T5/T6/T7；§3.6 HITL→T8；§7 影响面全部有对应 Task；§8 不变量（I2 IDOR→T5/T6 校验、I5 ts-rs→T9.1）。无遗漏。
**2. 占位符扫描：** 各 Task 给出真实代码块；标注「读 X 确认签名」是执行期的改前定位动作（global-enforcement §7），非 TODO；工具分支注释处明确「替换为读到的真实 repo 调用」。
**3. 类型一致性：** `ScopingPolicy`/`SubjectKind`/`AssetConfirmation`（T1 定义）在 T3/T4 引用一致；`scope_human_approved` claim kind 在 T3 gate 与 T4 prompt 一致；`input_type` 值 `scope_review`/`unit_review` 在 T4 prompt 与 T8 前端一致；`manage_organizations` 工具名在 T6 定义、T7 注册一致。

> P1（设计 §10）：防伪造交叉验证（`scope_human_approved` claim 关联真实 AskHumanResponse）、assessment/bug_bounty/cloud 细化（scope_rules 编辑卡、云资产采集）、scoping HITL/gate 进 trace 面板——本 P0 不含。
