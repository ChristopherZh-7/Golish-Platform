# Operation Harness 全量实现计划（depth-first 铺完运行时）

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
>
> **执行约束（用户 2026-06-01 明确指示，覆盖 AGENTS.md §2.6「每次改动即验证」铁律 · 仅本计划有效）：**
> 1. **不要每改一处就跑全流程编译**（`cargo check`/`just check` 很慢）。按 Phase 把代码全部铺完，**Phase 末尾才集中编译一次**改错。
> 2. **测试滞后**：主干铺完后再一点点测、一点点修。本计划只在每个 Phase 末放一个「集中编译 gate」，不在每个 task 后放 `cargo` 命令。
> 3. **Harness Lab 自反馈线（设计 §7：trace→bench→失败分析→自动改 prompt/gate/profile）整体滞后**，不在本计划范围（见 Phase D）。
> 4. commit / push 仍按 AGENTS.md §2.7：未经用户授权不 commit、不 push。

**目标：** 把 Operation Harness 从「仅 assessment + external_attack_surface 单 stage 能跑」铺成「所有 Profile × 所有 Stage 都有声明数据 + 通用运行时（多 stage 加载 / 通用 gate / 工具面授权 / charter 注入 / gate→repair 回灌 / approval 闸 / handoff）能跑通主干」。

**架构：** DAG=边界 / Profile=权限与意图 / Stage loop=自适应执行 / Gate=确定性裁判 / Evidence=唯一真相。复用现有 `task_orchestrator` 内循环 + harness gate hook；把当前硬编码 `assessment + ExternalAttackSurface` 的接缝**泛化**为「按 operation 的 profile_id + 当前 stage_kind 从嵌入资源加载 spec/profile，跑通用 gate，按 DAG 推进 operation_state 游标」。

**技术栈：** Rust 2021（`golish-agent-kit` harness 模块 + `golish-agent-app` db bridge）、`serde_json`、嵌入式 Postgres（`operation_state` 游标）、`include_str!` 嵌入 `resources/harness/**`。Feature flag `GOLISH_HARNESS_STAGE_MODE`（默认 OFF）。

---

## 0. 现状盘点（2026-06-01 磁盘 + git 实证）

| 组件 | 文件 | 状态 |
|---|---|---|
| Profile DTO + loader + L0–L5 rank | `harness/profile.rs` | ✅ 完整；磁盘仅 `resources/harness/profiles/assessment.json`(+`assessment.sprint_skeleton.json`) |
| Base DAG + 投影 + next_stages | `harness/operation_graph.rs` + `resources/harness/graph/operation_graph.json`(12 节点) | ✅ 完整 |
| StageSpec DTO + loader | `harness/stage_spec.rs` | ✅ 字段齐全；磁盘仅 `resources/harness/stages/external_attack_surface.json` |
| StageHarness::for_stage | `harness/stage_harness.rs:40` | ⚠ **硬锁** `ExternalAttackSurface`，其它 stage 返 Err |
| Gate（6 check）+ 聚合 | `harness/gate/mod.rs` + `gate/*.rs` | ⚠ 只有 `validate_external_attack_surface_gate` 一个入口；check 本身通用 |
| Deliverable 类型 | `harness/types.rs:143` `ExternalAttackSurfaceDeliverable` | ⚠ 仅 1 个（实际字段已通用：claims/findings/evidence_refs/skipped_checks） |
| PreActionAuthorizer | `harness/pre_action_authorizer.rs` | ⚠ 写好+单测，**0 调用方**（未接 dispatch） |
| stage_transition + operation_state 游标 | `harness/stage_transition.rs` + `subtask_phases/execute.rs:279` + `golish-agent-app db_bridge` | ✅ 接好（编译+单测），**未活体跑** |
| gate hook | `subtask_phases/execute.rs:358` `apply_harness_gate_hook` | ⚠ 硬编码 assessment + ExternalAttackSurface + content 整体当 JSON |
| 内循环（子任务/refine/reflector） | `subtask_phases/execute.rs:24` `execute_single_subtask` | ✅ 复用 |
| Evidence Ledger（六态） | `golish-pentest/evidence_ledger` | ✅ Phase 1a 已建 |
| Harness Lab 自反馈 | — | ⏸ 未建（本计划滞后） |

**核心洞察：** 大量组件已是「纯逻辑 + 单测」的孤岛。全量实现 ≈ ①补数据层（profiles + stages JSON）②把 external 这套**泛化**成「按 kind 加载 + 通用 gate dispatch」③把孤岛（authorizer / charter / recovery→refiner / approval）接进运行时闭环。**不是从零造轮子。**

---

## 1. 三个总体设计决策（执行者必须照此实现，保证 task 间类型一致）

### D-a · Deliverable：单一通用 `StageDeliverable`（不做 N 个 per-stage struct）
`ExternalAttackSurfaceDeliverable` 字段已通用。改名为 `StageDeliverable` + 加 `stage_kind: StageKind`，并保留 `pub type ExternalAttackSurfaceDeliverable = StageDeliverable;` 别名，使现有 hook/gate/单测零改动编译通过。YAGNI：某 stage 真出现独有字段时再加 `#[serde(flatten)] extra`，本计划不做。

### D-b · Gate：单一通用 `validate_stage_gate(deliverable, spec, contract)`，按 `spec.required_checks` 选语义 check
- **结构性 check 永远跑**：`schema_check` / `contract_check` / `vacuous_check` / `freshness_check`（与 deliverable 形状/契约/时效有关，与 stage 语义无关）。
- **语义 check 按 `spec.required_checks` 命名选跑**：建一个 `check-name → fn` 注册表。映射（现有 → required_checks 名）：
  | required_checks 名 | check fn |
  |---|---|
  | `scope_status_present` / `out_of_scope_targets_excluded` | `scope_check::run` |
  | `surface_workbench_coverage` | `surface_coverage_check::run` |
  | `min_tool_invocations_per_check` | 新 `min_invocations_check::run`（读 `spec.min_invocations` vs deliverable 的 `required_checks_done`/tool 痕迹；MVP 先按 `required_checks_done` 名单近似）|
  | `evidence_non_empty` / `unchecked_distinct_from_checked_empty` | 已在 `schema_check`/`vacuous_check` 覆盖，命名映射到既有 check（不重复跑）|
- 保留 `validate_external_attack_surface_gate` 为**薄包装**：`validate_stage_gate(d, external_spec, contract)`，旧调用方与 e2e 单测不破。

### D-c · 资源加载：嵌入式 registry（`include_str!` map）
新增 `harness/resources.rs`：`stage_spec_json(kind) -> Option<&'static str>` + `profile_json(profile_id) -> Option<&'static str>` + `load_embedded_stage_spec(kind)` + `load_embedded_profile(id)`。所有 `resources/harness/**` 经此一处加载，消灭散落的 `include_str!`（`execute.rs:344-348` / 各模块单测）。

---

## 2. 文件结构（创建/修改一览）

### Phase A · 数据层（纯 JSON，零 Rust 行为变更）
- **创建** `resources/harness/stages/scoping.json`
- **创建** `resources/harness/stages/target_intel.json`
- **创建** `resources/harness/stages/enumeration.json`
- **创建** `resources/harness/stages/reporting.json`
- **创建** `resources/harness/stages/vuln_triage.json`
- **创建** `resources/harness/stages/verification.json`
- **创建** `resources/harness/stages/access_validation.json`
- **创建** `resources/harness/stages/internal_discovery.json`
- **创建** `resources/harness/stages/objective_pathing.json`
- **创建** `resources/harness/stages/objective_simulation.json`
- **创建** `resources/harness/stages/cleanup.json`
- **创建** `resources/harness/profiles/pentest.json`
- **创建** `resources/harness/profiles/red_team.json`
- **创建** `resources/harness/profiles/bug_bounty.json`
- **创建** `resources/harness/profiles/cloud_assessment.json`

### Phase B · 多 stage 骨架（Rust 泛化）
- **修改** `harness/types.rs`（`StageDeliverable` 改名 + `stage_kind` 字段 + 别名）
- **创建** `harness/resources.rs`（嵌入 registry）
- **创建** `harness/gate/min_invocations_check.rs`
- **修改** `harness/gate/mod.rs`（`validate_stage_gate` 通用入口 + check 注册表 + 薄包装）
- **修改** `harness/stage_harness.rs`（解锁 `for_stage`：按 kind 从 registry 载 spec）
- **修改** `harness/mod.rs`（re-export 新符号）

### Phase C · 接线成闭环（运行时集成）
- **修改** `task_orchestrator/subtask_phases/execute.rs`（hook 泛化 + recovery→refiner 回灌 + drive_stage_transition 通用 profile）
- **修改** `task_orchestrator/prompts/mod.rs`（charter 注入 system context）
- **修改** `task_orchestrator/tool_dispatch`（接 `PreActionAuthorizer::check`，文件待 Phase C0 定位）
- **修改** `harness/stage_transition.rs` 或新增 approval 判定（approval_policy + has_user_approval）
- **修改** `golish-agent-app db_bridge`（若 operation_state 需带 profile_id，扩 `operation_state_insert`）

### Phase D · 滞后（不在本计划做）
- Harness Lab（trace→bench→失败分析→自动改）；逐项测试补全。

---

## Phase A · 数据层（先铺完，解锁后面一切）

> 每个 stage JSON 严格照 `resources/harness/stages/external_attack_surface.json` 的字段集；`kind` 取值必须 ∈ `StageKind`（见 `harness/types.rs:16`，snake_case）；`gate_validator` 统一填 `"validate_stage_gate"`（Phase B 通用入口）；`deliverable_schema` 统一填 `"StageDeliverable"`。`requires_stages`/`allowed_next_stages` 必须与 `resources/harness/graph/operation_graph.json` 的边一致。

### Task A1 · `resources/harness/stages/scoping.json`
**文件：** 创建 `resources/harness/stages/scoping.json`
**步骤：** 写入：
```json
{
  "$comment": "Stage spec: scoping / ROE. DAG entry. L0-L1 only, no probing.",
  "id": "scoping",
  "kind": "scoping",
  "risk_level": "low",
  "requires_stages": [],
  "allowed_next_stages": ["target_intel"],
  "allowed_tools": [
    "query_target_data",
    "log_operation",
    "submit_stage_deliverable"
  ],
  "forbidden_tools": ["dns_resolve", "http_probe", "subdomain_enum_passive", "metasploit", "sqlmap", "destructive_action"],
  "deliverable_schema": "StageDeliverable",
  "gate_validator": "validate_stage_gate",
  "required_checks": ["scope_status_present"],
  "min_invocations": {},
  "max_other_skips": 2,
  "human_approval": { "required_before": ["scope_expansion"] },
  "agent_continuity": "single_session",
  "inherits_evidence_from": []
}
```

### Task A2 · `resources/harness/stages/target_intel.json`
**文件：** 创建 `resources/harness/stages/target_intel.json`
**步骤：** 写入：
```json
{
  "$comment": "Stage spec: target_intel. Passive intel collection (DNS/ASN/whois).",
  "id": "target_intel",
  "kind": "target_intel",
  "risk_level": "low",
  "requires_stages": ["scoping"],
  "allowed_next_stages": ["external_attack_surface"],
  "allowed_tools": [
    "query_target_data",
    "dns_resolve",
    "subdomain_enum_passive",
    "shodan_query",
    "log_operation",
    "log_scan_result",
    "submit_stage_deliverable"
  ],
  "forbidden_tools": ["http_probe", "metasploit", "sqlmap", "credential_attack", "destructive_action", "exploit_validation"],
  "deliverable_schema": "StageDeliverable",
  "gate_validator": "validate_stage_gate",
  "required_checks": ["scope_status_present", "evidence_non_empty", "out_of_scope_targets_excluded"],
  "min_invocations": { "dns_resolve": 1 },
  "max_other_skips": 2,
  "human_approval": { "required_before": ["active_scan"] },
  "agent_continuity": "single_session",
  "inherits_evidence_from": [{ "stage_kind": "scoping", "evidence_kinds": ["scope_rule"] }]
}
```

### Task A3 · `resources/harness/stages/enumeration.json`
**文件：** 创建 `resources/harness/stages/enumeration.json`
**步骤：** 写入：
```json
{
  "$comment": "Stage spec: enumeration. Active recon (L2): port/service/dir enum.",
  "id": "enumeration",
  "kind": "enumeration",
  "risk_level": "medium",
  "requires_stages": ["external_attack_surface"],
  "allowed_next_stages": ["vuln_triage", "reporting"],
  "allowed_tools": [
    "query_target_data",
    "http_probe",
    "fingerprint_target",
    "port_scan",
    "dir_enum",
    "log_operation",
    "log_scan_result",
    "submit_stage_deliverable"
  ],
  "forbidden_tools": ["metasploit", "sqlmap", "credential_attack", "destructive_action", "exploit_validation"],
  "deliverable_schema": "StageDeliverable",
  "gate_validator": "validate_stage_gate",
  "required_checks": ["scope_status_present", "evidence_non_empty", "unchecked_distinct_from_checked_empty", "out_of_scope_targets_excluded", "min_tool_invocations_per_check"],
  "min_invocations": { "http_probe": 1 },
  "max_other_skips": 2,
  "human_approval": { "required_before": ["active_scan"] },
  "agent_continuity": "single_session",
  "inherits_evidence_from": [{ "stage_kind": "external_attack_surface", "evidence_kinds": ["http_service", "fingerprint", "subdomain"] }]
}
```

### Task A4 · `resources/harness/stages/reporting.json`
**文件：** 创建 `resources/harness/stages/reporting.json`
**步骤：** 写入：
```json
{
  "$comment": "Stage spec: reporting. Terminal stage. Synthesize report from evidence.",
  "id": "reporting",
  "kind": "reporting",
  "risk_level": "low",
  "requires_stages": [],
  "allowed_next_stages": [],
  "allowed_tools": ["query_target_data", "log_operation", "submit_stage_deliverable"],
  "forbidden_tools": ["dns_resolve", "http_probe", "metasploit", "sqlmap", "destructive_action"],
  "deliverable_schema": "StageDeliverable",
  "gate_validator": "validate_stage_gate",
  "required_checks": ["evidence_non_empty"],
  "min_invocations": {},
  "max_other_skips": 5,
  "human_approval": { "required_before": [] },
  "agent_continuity": "single_session",
  "inherits_evidence_from": [
    { "stage_kind": "external_attack_surface", "evidence_kinds": ["http_service", "subdomain", "fingerprint"] },
    { "stage_kind": "enumeration", "evidence_kinds": ["open_port", "dir_entry"] }
  ]
}
```

### Task A5 · pentest 专属 stage（`vuln_triage.json` + `verification.json`）
**文件：** 创建 `resources/harness/stages/vuln_triage.json` 和 `resources/harness/stages/verification.json`
**步骤：** `vuln_triage.json`：
```json
{
  "$comment": "Stage spec: vuln_triage. L3 non-destructive vuln validation.",
  "id": "vuln_triage",
  "kind": "vuln_triage",
  "risk_level": "high",
  "requires_stages": ["enumeration"],
  "allowed_next_stages": ["verification", "reporting"],
  "allowed_tools": ["query_target_data", "nuclei_scan", "vuln_match", "http_probe", "log_operation", "log_scan_result", "submit_stage_deliverable"],
  "forbidden_tools": ["metasploit", "sqlmap", "credential_attack", "destructive_action"],
  "deliverable_schema": "StageDeliverable",
  "gate_validator": "validate_stage_gate",
  "required_checks": ["scope_status_present", "evidence_non_empty", "unchecked_distinct_from_checked_empty", "out_of_scope_targets_excluded"],
  "min_invocations": {},
  "max_other_skips": 2,
  "human_approval": { "required_before": ["active_scan", "exploit_validation"] },
  "agent_continuity": "single_session",
  "inherits_evidence_from": [{ "stage_kind": "enumeration", "evidence_kinds": ["open_port", "http_service", "fingerprint"] }]
}
```
`verification.json`：
```json
{
  "$comment": "Stage spec: verification. L4 controlled exploit validation. Approval-gated.",
  "id": "verification",
  "kind": "verification",
  "risk_level": "critical",
  "requires_stages": ["vuln_triage"],
  "allowed_next_stages": ["reporting"],
  "allowed_tools": ["query_target_data", "exploit_validation", "log_operation", "log_scan_result", "submit_stage_deliverable"],
  "forbidden_tools": ["credential_attack", "destructive_action", "persistence"],
  "deliverable_schema": "StageDeliverable",
  "gate_validator": "validate_stage_gate",
  "required_checks": ["scope_status_present", "evidence_non_empty", "out_of_scope_targets_excluded"],
  "min_invocations": {},
  "max_other_skips": 1,
  "human_approval": { "required_before": ["exploit_validation"] },
  "agent_continuity": "single_session",
  "inherits_evidence_from": [{ "stage_kind": "vuln_triage", "evidence_kinds": ["vuln_finding"] }]
}
```

### Task A6 · red_team 专属 stage（5 个）
**文件：** 创建 `access_validation.json` / `internal_discovery.json` / `objective_pathing.json` / `objective_simulation.json` / `cleanup.json`，均照下模板，仅改 `id`/`kind`/`requires_stages`/`allowed_next_stages`/`risk_level`/`allowed_tools`，对照 `operation_graph.json` 的边链 `verification→access_validation→…→cleanup→reporting`：
```json
{
  "$comment": "Stage spec: <KIND>. L5 red-team. Approval-gated, cleanup required.",
  "id": "<KIND>",
  "kind": "<KIND>",
  "risk_level": "critical",
  "requires_stages": ["<PREV_KIND>"],
  "allowed_next_stages": ["<NEXT_KIND>"],
  "allowed_tools": ["query_target_data", "log_operation", "log_scan_result", "submit_stage_deliverable"],
  "forbidden_tools": ["destructive_action"],
  "deliverable_schema": "StageDeliverable",
  "gate_validator": "validate_stage_gate",
  "required_checks": ["scope_status_present", "evidence_non_empty", "out_of_scope_targets_excluded"],
  "min_invocations": {},
  "max_other_skips": 1,
  "human_approval": { "required_before": ["post_exploit", "exploit_validation"] },
  "agent_continuity": "multi_session_relay",
  "inherits_evidence_from": []
}
```
逐文件取值：
- `access_validation`：prev=`verification`，next=`internal_discovery`
- `internal_discovery`：prev=`access_validation`，next=`objective_pathing`
- `objective_pathing`：prev=`internal_discovery`，next=`objective_simulation`
- `objective_simulation`：prev=`objective_pathing`，next=`cleanup`
- `cleanup`：prev=`objective_simulation`，next=`reporting`，`risk_level`=`medium`

### Task A7 · 其余 Profile JSON
**文件：** 创建 `pentest.json` / `red_team.json` / `bug_bounty.json` / `cloud_assessment.json`，照 `resources/harness/profiles/assessment.json` 字段集。`max_authorization` ∈ `AuthorizationLevel`（snake_case：`observe_only`/`passive_intel`/`active_recon`/`vuln_validation`/`controlled_exploit`/`post_exploit_red_team`）。
`pentest.json`：
```json
{
  "$comment": "Profile: Pentest. L4 controlled exploit.",
  "id": "pentest",
  "display_name": "Pentest",
  "max_authorization": "controlled_exploit",
  "allowed_stage_kinds": ["scoping", "target_intel", "external_attack_surface", "enumeration", "vuln_triage", "verification", "reporting"],
  "forbidden_stage_kinds": ["access_validation", "internal_discovery", "objective_pathing", "objective_simulation", "cleanup"],
  "approval_policy": { "before_active_scan": true, "before_scope_expansion": true },
  "cleanup_required": false,
  "evidence_required": true
}
```
`red_team.json`：
```json
{
  "$comment": "Profile: Red Team. L5 post-exploit. Cleanup required.",
  "id": "red_team",
  "display_name": "Red Team",
  "max_authorization": "post_exploit_red_team",
  "allowed_stage_kinds": ["scoping", "target_intel", "external_attack_surface", "enumeration", "vuln_triage", "verification", "access_validation", "internal_discovery", "objective_pathing", "objective_simulation", "cleanup", "reporting"],
  "forbidden_stage_kinds": [],
  "approval_policy": { "before_active_scan": true, "before_scope_expansion": true },
  "cleanup_required": true,
  "evidence_required": true
}
```
`bug_bounty.json`：max_authorization=`vuln_validation`，allowed=`["scoping","target_intel","external_attack_surface","enumeration","vuln_triage","reporting"]`，forbidden=其余，cleanup_required=false。
`cloud_assessment.json`：同 bug_bounty 的 max_authorization=`vuln_validation`，allowed 同 assessment + `vuln_triage`。

### Phase A 集中编译 gate（本 Phase 末跑一次）
- `python3 -m json.tool` 对每个新 JSON → exit 0（语法）。
- 暂不接 Rust（registry 在 Phase B），故 Phase A 不触发 cargo。**预期：无 Rust 改动 → 不编译。**

---

## Phase B · 多 stage 骨架（Rust 泛化）

### Task B1 · `StageDeliverable` 改名 + 通用化
**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/types.rs`
**步骤：** 把 `ExternalAttackSurfaceDeliverable`（:143）改名为 `StageDeliverable`，加 `stage_kind`，保留别名：
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageDeliverable {
    #[serde(default = "default_stage_kind")]
    pub stage_kind: StageKind,
    pub stage_id: String,
    pub stage_run_id: Uuid,
    pub claims: Vec<StageClaim>,
    pub evidence_refs: Vec<EvidenceAuditId>,
    #[serde(default)]
    pub skipped_checks: Vec<SkippedCheckRecord>,
    pub findings: Vec<HarnessFinding>,
    #[serde(default)]
    pub required_checks_done: Vec<String>,
}

fn default_stage_kind() -> StageKind { StageKind::ExternalAttackSurface }

/// 向后兼容别名：旧 hook / gate / e2e 单测无需改。
pub type ExternalAttackSurfaceDeliverable = StageDeliverable;
```
**说明：** `#[serde(default)]` 的 `stage_kind` 保证旧 JSON（无该字段）仍解析为 ExternalAttackSurface，零破坏。

### Task B2 · 嵌入资源 registry
**文件：** 创建 `backend/crates/golish-agent-kit/src/harness/resources.rs`
**步骤：**
```rust
//! 嵌入式 harness 资源 registry（Doc 3 §4 / §2）。所有 resources/harness/** 经此加载。
use super::profile::{load_profile_from_json, Profile, ProfileLoadError};
use super::stage_spec::{load_stage_spec_from_json, StageSpec, StageSpecLoadError};
use super::types::StageKind;

macro_rules! stage_json { ($p:literal) => { include_str!(concat!("../../../../../resources/harness/stages/", $p)) }; }
macro_rules! profile_json { ($p:literal) => { include_str!(concat!("../../../../../resources/harness/profiles/", $p)) }; }

pub fn stage_spec_json(kind: StageKind) -> Option<&'static str> {
    Some(match kind {
        StageKind::Scoping => stage_json!("scoping.json"),
        StageKind::TargetIntel => stage_json!("target_intel.json"),
        StageKind::ExternalAttackSurface => stage_json!("external_attack_surface.json"),
        StageKind::Enumeration => stage_json!("enumeration.json"),
        StageKind::VulnTriage => stage_json!("vuln_triage.json"),
        StageKind::Verification => stage_json!("verification.json"),
        StageKind::AccessValidation => stage_json!("access_validation.json"),
        StageKind::InternalDiscovery => stage_json!("internal_discovery.json"),
        StageKind::ObjectivePathing => stage_json!("objective_pathing.json"),
        StageKind::ObjectiveSimulation => stage_json!("objective_simulation.json"),
        StageKind::Cleanup => stage_json!("cleanup.json"),
        StageKind::Reporting => stage_json!("reporting.json"),
    })
}

pub fn profile_json(id: &str) -> Option<&'static str> {
    Some(match id {
        "assessment" => profile_json!("assessment.json"),
        "pentest" => profile_json!("pentest.json"),
        "red_team" => profile_json!("red_team.json"),
        "bug_bounty" => profile_json!("bug_bounty.json"),
        "cloud_assessment" => profile_json!("cloud_assessment.json"),
        _ => return None,
    })
}

pub fn load_embedded_stage_spec(kind: StageKind) -> Result<StageSpec, StageSpecLoadError> {
    load_stage_spec_from_json(stage_spec_json(kind).expect("stage json present"))
}

pub fn load_embedded_profile(id: &str) -> Result<Option<Profile>, ProfileLoadError> {
    match profile_json(id) { Some(raw) => Ok(Some(load_profile_from_json(raw)?)), None => Ok(None) }
}
```
**注意：** `include_str!` 相对路径以本文件（`harness/resources.rs`）为基准；按 `external_attack_surface.json` 在 `stage_spec.rs` 里的 `../../../../../resources/...` 同深度校准（同 `src/harness/` 层级）。

### Task B3 · `min_invocations_check`
**文件：** 创建 `backend/crates/golish-agent-kit/src/harness/gate/min_invocations_check.rs`
**步骤：**
```rust
//! min_tool_invocations_per_check：spec.min_invocations 里的工具是否在 deliverable
//! 的 required_checks_done 中体现（MVP 近似：名单匹配；Phase 2 接真实 tool 痕迹）。
use super::super::stage_spec::StageSpec;
use super::super::types::{HarnessRecoveryActions, StageDeliverable};
use super::GateCheckOutcome;

pub fn run(deliverable: &StageDeliverable, spec: &StageSpec) -> GateCheckOutcome {
    let mut missing = Vec::new();
    for tool in spec.min_invocations.keys() {
        if !deliverable.required_checks_done.iter().any(|c| c.contains(tool)) {
            missing.push(tool.clone());
        }
    }
    if missing.is_empty() { GateCheckOutcome::Pass } else {
        GateCheckOutcome::Block {
            reasons: missing.iter().map(|t| format!("min invocations not met for tool '{t}'")).collect(),
            recovery: HarnessRecoveryActions { repair_tool_calls: missing, ..Default::default() },
        }
    }
}
```

### Task B4 · 通用 gate 入口 `validate_stage_gate`
**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/gate/mod.rs`
**步骤：** 加 `pub mod min_invocations_check;`；新增通用入口，结构性 check 永跑 + 语义 check 按 `spec.required_checks` 选；旧入口改薄包装：
```rust
pub fn validate_stage_gate(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    contract: Option<&SprintContract>,
) -> GateResult {
    let mut outcomes = vec![
        schema_check::run(deliverable, spec),
        contract_check::run(deliverable, contract),
        vacuous_check::run(deliverable, spec),
        freshness_check::run(deliverable, spec),
    ];
    for name in &spec.required_checks {
        match name.as_str() {
            "scope_status_present" | "out_of_scope_targets_excluded" => outcomes.push(scope_check::run(deliverable)),
            "surface_workbench_coverage" => outcomes.push(surface_coverage_check::run(deliverable)),
            "min_tool_invocations_per_check" => outcomes.push(min_invocations_check::run(deliverable, spec)),
            _ => {} // evidence_non_empty / unchecked_distinct_* 已由 schema/vacuous 覆盖
        }
    }
    aggregate(outcomes)
}

fn aggregate(outcomes: Vec<GateCheckOutcome>) -> GateResult {
    let mut reasons = Vec::new();
    let mut recovery = HarnessRecoveryActions::default();
    for o in outcomes {
        if let GateCheckOutcome::Block { reasons: r, recovery: rec } = o {
            reasons.extend(r);
            recovery.hints.extend(rec.hints);
            recovery.repair_tool_calls.extend(rec.repair_tool_calls);
            recovery.missing_evidence_kinds.extend(rec.missing_evidence_kinds);
        }
    }
    if reasons.is_empty() { GateResult::pass() } else { GateResult::block(reasons, recovery) }
}

/// 薄包装，保留旧调用方与 e2e 单测。
pub fn validate_external_attack_surface_gate(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    contract: Option<&SprintContract>,
) -> GateResult { validate_stage_gate(deliverable, spec, contract) }
```
（把原 `validate_external_attack_surface_gate` 内联的 6-check 循环抽进 `aggregate`，避免重复。）

### Task B5 · 解锁 `StageHarness::for_stage`
**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/stage_harness.rs`
**步骤：** 去掉 :40 的单 stage 硬锁；`validate_gate` 改调通用 gate；新增按 kind 自动载 spec 的便捷构造：
```rust
pub fn for_stage(stage_kind: StageKind, profile: Profile, stage_spec: StageSpec) -> Result<Self> {
    if stage_spec.kind != stage_kind {
        return Err(anyhow!("StageSpec.kind ({:?}) != requested ({:?})", stage_spec.kind, stage_kind));
    }
    Ok(Self::new(profile, stage_spec))
}

/// 从嵌入 registry 自动载 spec（Phase B）。
pub fn for_stage_embedded(stage_kind: StageKind, profile: Profile) -> Result<Self> {
    let spec = super::resources::load_embedded_stage_spec(stage_kind)?;
    Self::for_stage(stage_kind, profile, spec)
}
```
`validate_gate` 内部把 `validate_external_attack_surface_gate(...)` 换成 `validate_stage_gate(deliverable, &self.stage_spec, sprint_contract)`。

### Task B6 · `harness/mod.rs` re-export
**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/mod.rs`
**步骤：** 加 `pub mod resources;`；`pub use gate::{validate_stage_gate, ...}`；`pub use resources::{load_embedded_profile, load_embedded_stage_spec, profile_json, stage_spec_json};`；`pub use types::StageDeliverable;`（别名 `ExternalAttackSurfaceDeliverable` 已随 types 导出）。

### Phase B 集中编译 gate（本 Phase 末跑一次）
```
cargo check -p golish-agent-kit
```
预期 exit 0。若别名/字段不齐导致 e2e_tests / 各 check 编译错，集中修。**本 Phase 末才跑，不在每个 task 后跑。**

---

## Phase C · 接线成闭环（运行时集成）

### Task C0 · 定位接缝（只读，先读后改）
**步骤：** 读 `task_orchestrator/tool_dispatch*`（找 tool call dispatch 的单点）、`task_orchestrator/prompts/mod.rs`（system prompt 组装）、`task_orchestrator/types.rs`（`PlannedSubtask` / `ExecutionContext` 是否有 profile_id / has_user_approval）。记录：①工具 dispatch 真正发生的函数 ②system prompt 拼接点 ③ctx 是否能拿到 profile + approval 状态。

### Task C1 · hook 泛化（去 ExternalAttackSurface 硬锁）
**文件：** 修改 `task_orchestrator/subtask_phases/execute.rs`
**步骤：** `apply_harness_gate_hook`（:358）：
- 删 :373 的 `stage_kind != ExternalAttackSurface` 早退。
- spec 改 `crate::harness::load_embedded_stage_spec(stage_hint.stage_kind)`；profile 改按 operation 的 profile_id 载（MVP：从 `planned`/`exec_ctx` 取，缺省 `"assessment"`，用 `load_embedded_profile`）。
- `StageHarness::for_stage_embedded(stage_hint.stage_kind, profile)`。
- `parse_deliverable_from_content` 返回 `StageDeliverable`（别名兼容；并加 JSON code-fence 抽取：先找 ```json fence，找不到再 trim 整体）。
- `drive_stage_transition`（:279）的 `ASSESSMENT_PROFILE_JSON` 改成按 operation profile_id 载（同上缺省 assessment）。

### Task C2 · charter 注入 system context
**文件：** 修改 `task_orchestrator/prompts/mod.rs`（按 C0 定位的拼接点）+ `execute.rs` 的 `augmented_description` 组装（:74）
**步骤：** 新增 `fn stage_charter(spec: &StageSpec) -> String`：渲染「你在 `<stage>` 阶段，允许工具 = allowed_tools，禁止 = forbidden_tools，必须产出 StageDeliverable 并 submit_stage_deliverable，gate 检查 = required_checks」。stage_mode 开 + `planned.harness_stage` 有值时，把 charter 前置进 `augmented_description`。

### Task C3 · 接 PreActionAuthorizer 进 dispatch
**文件：** 修改 C0 定位的 tool dispatch 函数
**步骤：** 每个 tool call dispatch 前：载当前 stage spec + profile，`PreActionAuthorizer::check(tool_name, &spec, &profile, intent)`；intent 由 IntentClassifier（已存在 `harness::IntentClassifier`）对 tool/参数判定。`Err(AuthorizationError::...)` → 不执行该 tool，回注 deny 文案给 agent（让其改走 allowed 工具）。stage_mode OFF 时跳过（零影响旧路径）。

### Task C4 · gate→repair 回灌 refiner
**文件：** 修改 `task_orchestrator/subtask_phases/execute.rs`
**步骤：** 当前 BLOCK 只把 decision JSON 追加进 content（:467）。改为：gate BLOCK 且 stage_mode 开时，把 `decision.recovery_actions`（hints/repair_tool_calls/missing_evidence_kinds）拼成 correction 文本，走现有 reflector 重试路径（:113 `executor.reflect` 同款），即把 recovery 当作 `## IMPORTANT CORRECTION` 注入再 `execute_subtask` 一次（受 `MAX_REFLECTOR_RETRIES` 约束）。

### Task C5 · approval 闸接 transition
**文件：** 修改 `harness/stage_transition.rs`（加 approval 判定）+ `execute.rs` 的 `drive_stage_transition`
**步骤：** 新增 `fn requires_approval(next_spec: &StageSpec, profile: &Profile) -> bool`：`next_spec.human_approval.required_before` 非空 且 `profile.approval_policy.before_active_scan`（或对应项）为 true。`drive_stage_transition` 在 `advance_stage` 前判定：需审批则不直接推进，emit `AiEvent::SubtaskWaitingForInput`（复用现有 :181 机制）请求用户确认，确认后再 `advance_stage`。`has_user_approval` 状态：MVP 从 `user_input_rx` 收到肯定回复即视为 approved。

### Task C6 · handoff + 跨 stage evidence 可见性
**文件：** 修改 `execute.rs`（transition 成功后）+ 复用 `StageSpec.inherits_evidence_from`
**步骤：** stage gate PASS 后产 handoff（MVP：把本 stage deliverable 的 evidence_refs 落 operation_state 或 enrich 上下文）。下一 stage 的 `enrich_subtask`（:35）读 `next_spec.inherits_evidence_from` 指定的 evidence_kinds，注入 SUPPLEMENTARY CONTEXT。

### Phase C 集中编译 gate（本 Phase 末跑一次）
```
cargo check -p golish-agent-kit -p golish-agent-app
```
预期 exit 0；集中修编译错。仍**不**跑全量 `just check`/test（按用户指示滞后）。

---

## Phase D · 滞后项（本计划不做，仅登记）

1. **Harness Lab 自反馈线**（设计 §7 / §13.6）：trace 收集 → Golish Bench → 失败分类 → 自动改 prompt/tool/gate/profile → rerun benchmark 比对 pass/fail flips。这是「自动化检测/修改 harness 的逻辑」，用户明确滞后。
2. **逐项测试**：主干铺完后，再按 stage / check / 闭环一点点补单测 + 活体 E2E（含 `db_bridge/mod.rs` 已挂的 opt-in `operation_state` 集成测试，需 `GOLISH_TEST_DATABASE_URL`）。
3. **活体端到端验证**：`GOLISH_HARNESS_STAGE_MODE=true` + `just dev` + 真 agent 跑多 stage，看 operation_state 游标真推进、approval 真弹窗、gate BLOCK 真回灌。

---

## 自检（写完计划后对照）

**1. 规格覆盖度：**
- 设计 §5.1 Profile → Phase A Task A7（pentest/red_team/bug_bounty/cloud_assessment）✅
- §5.2 Base DAG → 已存在（operation_graph.json）✅
- §5.3 StageSpec → Phase A Task A1–A6（11 个新 stage）✅
- §5.4 Stage Harness（charter/工具面限制/normalize/deliverable/gate/repair/边界）→ Phase B(B5) + Phase C(C2 charter / C3 工具面 / C4 repair / C5 边界)✅
- §5.5 Evidence Ledger 六态 → 已存在；跨 stage 可见性 Phase C(C6)✅
- §5.6 Gate Validator → Phase B(B4 通用 gate)✅
- §6.1 Operation Startup（profile→DAG 投影→plan→首 stage）→ 已存在（orchestrator + operation_graph），profile 多选 Phase C(C1)✅
- §6.2 Stage Execution（载 spec/charter/限工具/内循环/submit/gate/repair/handoff/advance）→ Phase B + C 全覆盖 ✅
- §6.3 Inner Loop → 复用现有 execute_single_subtask ✅
- §7 Harness Lab → Phase D（滞后，明确登记）✅

**2. 占位符扫描：** Phase A 全为完整 JSON；Phase B 全为完整 Rust；Phase C 因依赖 C0 只读定位结果，标注「按 C0 定位点修改」并给出代表性代码 + 精确函数名（`apply_harness_gate_hook`/`drive_stage_transition`/`execute_single_subtask`/`PreActionAuthorizer::check`/`decide_transition`），非 TODO 占位。

**3. 类型一致性：** `StageDeliverable`（B1）= gate（B4）入参 = hook（C1）解析目标，统一；别名 `ExternalAttackSurfaceDeliverable` 全程兼容；`validate_stage_gate` 签名 `(deliverable, spec, contract)` 在 B4/B5 一致；`load_embedded_stage_spec(kind)` / `load_embedded_profile(id)` 在 B2/B5/C1 一致。

---

## 执行顺序总结

A（数据层，零风险，解锁全部）→ A 末 JSON 校验 → B（Rust 泛化）→ B 末 `cargo check -p golish-agent-kit` → C（闭环接线，先 C0 只读定位）→ C 末 `cargo check -p golish-agent-kit -p golish-agent-app` → 报告用户 → 用户决定 commit / 跑 E2E。Phase D 永远滞后。
