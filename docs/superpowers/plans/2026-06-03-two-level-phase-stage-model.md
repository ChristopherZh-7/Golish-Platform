# 两级阶段模型（大阶段 Phase × 小阶段 Stage）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
>
> **设计来源（已 Approved 2026-06-03）：** `docs/design/2026-06-03-two-level-phase-stage-model.md`。本计划只把该设计落地，不重开设计决策。
>
> **执行约束：**
> 1. commit / push 按 AGENTS.md §2.7：未经用户授权不 commit、不 push。
> 2. 每个 Phase 末尾跑一次集中编译 + 该 crate 的 nextest（不是每个 task 都跑全量 `just check`）。
> 3. 全部改动挂在 feature flag `GOLISH_HARNESS_TWO_LEVEL`（默认 OFF）后；OFF = 现状线性 DAG + per-stage 行为，零回归。

**目标：** 把扁平的 12 个 stage 重构成「大阶段(phase) + 小阶段(stage)」两级；大阶段=风险/审批边界（锚定授权阶梯 L0–L5），大阶段内小阶段去人为先后、出口共一道放行（成员 gate 全 PASS + 边界审批）。

**架构：** 新增 `phases.json` 拓扑 + `phase.rs` loader（纯数据层）→ `phase_flow.rs` 在现有 `operation_graph`/`stage_transition` 之上加「phase-aware 流转」（成员 stage 全 PASS 才跨 phase；approval 落 phase 边界且 de-dup）→ 运行时游标（`subtask_phases/execute.rs` + `stage_execution.rs` + graph-flow `operation_flow.rs`）在 flag 开时改走 phase-aware 决策 → 前端按 phase 分组展示。**Gate 内核零改动**（D5/甲）。

**技术栈：** Rust 2021（`golish-agent-kit/src/harness`）、`serde`/`serde_json`/`thiserror`、`include_str!` 嵌入 `resources/harness/**`、`cargo nextest`；前端 React/TS（`AIChatPanel`）。Feature flag `GOLISH_HARNESS_TWO_LEVEL`。

---

## 执行实况（2026-06-03 · A–G 进度 + 对计划的偏差）

- **A/B/C ✅ 完成**（kit 纯新增 + 单测）：phases.json / phase.rs / phase_flow.rs / resources loader。C 重审结论=唯一并行候选 `eas∥enumeration` 涉安全语义、本批不删边（见 design §8）。
- **G1 ✅**：`two_level_enabled()`（`GOLISH_HARNESS_TWO_LEVEL`，默认 OFF）。
- **E1 ⤳ 简化（偏差）**：线性 DAG 下「大阶段跑完」隐式于线性遍历，运行时用 `phase_of(current)≠phase_of(next)` 检测跨界即可，**无需** gate_passed 集合。`decide_phase_step`/`pending_phase_approval` 仍保留供未来真并行用。新增运行时实际用的是 `crossing_phase_approval` / `phase_crossing_requires_approval`（phase_flow.rs）。
- **D2/E2 ✅（legacy 路径）**：`drive_stage_transition` 审批条件 flag 切换。
- **E3 ✅（graph-flow 路径，默认 active）**：发现运行时有**两条路径**（legacy 有审批 hold；graph-flow `run_executor_driven` 默认 ON 且原无审批）。新增 `two_level_phase_gate(&mut self)` 在 servicer loop 回引擎前拦截：跨大阶段需审批则阻塞等 `user_input_rx`，未获批→`outcome=blocked`→引擎 Interrupt。
- **F ⏸ 暂缓（偏差 + 理由）**：`subtask_completed`/`task_progress` 事件只带自由文本 `title`、**无 `StageKind`** → 视觉分组 headers 需后端事件加 `stage_kind`（IPC/ts-rs 变更，超出 frontend-only，应独立任务）；`useAiChatEvents.ts` 另有其它 agent 未提交改动（git 隔离风险）。phase **边界已可见**：`two_level_phase_gate` 的「Phase boundary X → Y…」经现有 StageMarker `waiting_approval` detail 展示。
- **验证**：`nextest -p golish-agent-kit` 441/441（off）+ `GOLISH_HARNESS_TWO_LEVEL=1` harness 205/205 + clippy `-D` + rustfmt clean + 下游 `cargo check` exit 0。**G2 全量 precommit / G3 活体 E2E 未做**；零 commit。

---

## 0. 现状盘点（2026-06-03 磁盘实证，附文件:行）

| 组件 | 文件 | 现状 |
|---|---|---|
| 12 StageKind + try_parse/as_str | `harness/types.rs:16` | ✅ 完整 |
| 授权阶梯 L0–L5 + Profile + ApprovalPolicy | `harness/profile.rs:13/45/55` | ✅ 完整 |
| Base DAG（12 节点/15 边）+ 投影 + next_stages | `resources/harness/graph/operation_graph.json` + `harness/operation_graph.rs:151/198` | ✅ 完整；边为线性主干 + 4 bail-to-reporting |
| StageSpec（含 requires_stages/required_checks/min_invocations/max_other_skips/human_approval） | `harness/stage_spec.rs:42` | ✅ 完整；12 个 `resources/harness/stages/*.json` 齐 |
| 流转决策（纯函数 Hold/Complete/Advance/Branch）+ `stage_entry_requires_approval` | `harness/stage_transition.rs:51/84` | ✅ 完整；游标推进由调用方做 |
| Gate（结构 + 语义 + 证据账本交叉） | `harness/gate/` + `subtask_phases/execute.rs` | ✅ 完整（**本计划不动**） |
| 运行时游标推进 + gate hook | `task_orchestrator/subtask_phases/execute.rs`、`task_orchestrator/stage_execution.rs`、`task_orchestrator/harness_backfill.rs` | ✅ per-stage 运行 |
| graph-flow 引擎路径 | `harness/operation_flow.rs` | ✅ flag `GOLISH_HARNESS_GRAPH_FLOW` 时接管顶层循环 |
| 资源嵌入 registry | `harness/resources.rs` | ✅ `load_embedded_stage_spec`/`load_embedded_profile` |
| 前端阶段分隔卡 | `AIChatPanel/StageMarker.tsx` + `hooks/useAiChatEvents.ts`（commit `5fe447d`） | ✅ 已有 stage 边界展示，可扩 phase |

**核心洞察：** phase 是 stage 之上的**编排/拓扑薄层**，不动 gate 内核、不动 stage 数据结构、不动 DB schema。新增数据(phases.json)+loader+phase-flow，运行时用 flag 切换「per-stage 推进 → per-phase 推进」。

---

## 1. 总体设计决策（执行者必须照此，保证 task 间类型一致）

### D-a · Phase id 用字符串常量，不引入新 enum
`Phase` 用 `id: String`（`"prep"`/`"active_recon"`/`"vuln"`/`"post_exploit"`/`"closeout"`），成员是 `Vec<StageKind>`。理由：phase 集合是配置驱动（phases.json），不同 profile 投影后成员不同，用 enum 会和 profile 裁剪打架。

### D-b · 分组（乙，2026-06-03 拍板）
```
prep:         [scoping, target_intel]
active_recon: [external_attack_surface, enumeration]   entry_approval=active_scan
vuln:         [vuln_triage, verification]               entry_approval=exploit_validation
post_exploit: [access_validation, internal_discovery, objective_pathing, objective_simulation]
closeout:     [reporting, cleanup]
```

### D-c · Phase 完整判定（甲）= 成员小阶段 gate 全 PASS
不改 gate。新增 `phase_is_complete(phase, gate_passed_set)` = phase 的 allowed 成员是否都在 `gate_passed_set` 里。

### D-d · 审批落 phase 边界 + de-dup
`active_scan` / `exploit_validation` 在「跨入下一 phase 前」触发一次（即使该 phase 内多个 stage 都声明同 key）。`scope_expansion` 维持现有「动作前」事件触发，不改。复用 `stage_transition::stage_entry_requires_approval` 的判定，但在 phase 层去重。

### D-e · Flag
`GOLISH_HARNESS_TWO_LEVEL`（env，默认 OFF）。仿 `GOLISH_HARNESS_GRAPH_FLOW` 的读取方式（见 §Phase E task）。

---

## 2. 文件结构（创建/修改一览）

**Phase A 数据 + loader（纯新增，零行为变更）**
- 创建 `resources/harness/graph/phases.json`
- 创建 `backend/crates/golish-agent-kit/src/harness/phase.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/mod.rs`（`pub mod phase;`）
- 修改 `backend/crates/golish-agent-kit/src/harness/resources.rs`（嵌入 phases.json）

**Phase B phase-aware 流转（纯新增 + 单测）**
- 创建 `backend/crates/golish-agent-kit/src/harness/phase_flow.rs`
- 修改 `harness/mod.rs`（`pub mod phase_flow;`）

**Phase C 放开阶段内顺序（数据 + 重审）**
- 修改 `resources/harness/graph/operation_graph.json`（删人为边）
- 修改 `resources/harness/stages/{enumeration,target_intel}.json`（按重审调 requires_stages）
- 复用 `docs/design/2026-06-02-stage-spec-worksheet.csv`（填依赖分类）

**Phase D 审批落边界（运行时接线）**
- 修改 `harness/phase_flow.rs`（phase 边界审批解析 + de-dup）
- 修改 `task_orchestrator/subtask_phases/execute.rs`（flag 开时审批走 phase 层）

**Phase E 运行时游标 phase-aware**
- 修改 `task_orchestrator/subtask_phases/execute.rs` + `task_orchestrator/stage_execution.rs`（gate PASS 后用 phase_flow 决定是否跨 phase）
- 修改 `harness/operation_flow.rs`（graph-flow 路径同款接 phase 门）

**Phase F 前端 phase 分组**
- 修改 `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`
- 修改 `frontend/components/AIChatPanel/StageMarker.tsx`

**Phase G flag 接线 + 集成 + precommit**
- 修改读取 flag 的处（§Phase E task 指明）
- 跑 `just precommit`

---

## Phase A · 数据层 + loader

### Task A1 · 创建 phases.json
**文件：** 创建 `resources/harness/graph/phases.json`
**步骤：** 写入（成员顺序＝StageKind 顺序；`entry_approval` 缺省表示无 phase 级入口审批）：
```json
{
  "$comment": "Two-level phase grouping over the 12 StageKinds. Design: docs/design/2026-06-03-two-level-phase-stage-model.md (乙 grouping). Consumed by harness/phase.rs; projected per profile like operation_graph.json.",
  "phases": [
    { "id": "prep",         "stages": ["scoping", "target_intel"] },
    { "id": "active_recon", "stages": ["external_attack_surface", "enumeration"], "entry_approval": "active_scan" },
    { "id": "vuln",         "stages": ["vuln_triage", "verification"], "entry_approval": "exploit_validation" },
    { "id": "post_exploit", "stages": ["access_validation", "internal_discovery", "objective_pathing", "objective_simulation"] },
    { "id": "closeout",     "stages": ["reporting", "cleanup"] }
  ]
}
```
**验证：** `python3 -m json.tool resources/harness/graph/phases.json` → exit 0。
**提交：** `feat(harness): add phases.json two-level grouping (乙)`。

### Task A2 · 写 phase.rs DTO + loader（失败测试先行）
**文件：** 创建 `backend/crates/golish-agent-kit/src/harness/phase.rs`
**步骤 1（写类型 + loader，仿 `operation_graph.rs` 风格）：**
```rust
//! Phase grouping DTO + loader (设计 2026-06-03 两级阶段模型).
//!
//! Phase = 大阶段，是 12 个 StageKind 之上的编排薄层。成员是 StageKind 列表，
//! 每个 phase 可声明跨入它之前的 `entry_approval`（human_approval key）。
//! 与 operation_graph.json 一样：静态 JSON 加载 + 按 profile 投影。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::profile::Profile;
use super::types::StageKind;

/// phases.json `phases[*]` 元素.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    pub id: String,
    pub stages: Vec<StageKind>,
    /// 跨入本 phase 前需要的人工审批动作 key（如 "active_scan"）。None = 无 phase 级入口审批。
    #[serde(default)]
    pub entry_approval: Option<String>,
}

impl Phase {
    pub fn contains(&self, stage: StageKind) -> bool {
        self.stages.contains(&stage)
    }
}

/// phases.json 根.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseMap {
    pub phases: Vec<Phase>,
}

#[derive(Debug, Error)]
pub enum PhaseMapError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("stage {0:?} appears in more than one phase")]
    DuplicateStage(StageKind),
    #[error("stage {0:?} is not assigned to any phase")]
    UnassignedStage(StageKind),
}

const ALL_STAGES: [StageKind; 12] = [
    StageKind::Scoping,
    StageKind::TargetIntel,
    StageKind::ExternalAttackSurface,
    StageKind::Enumeration,
    StageKind::VulnTriage,
    StageKind::Verification,
    StageKind::AccessValidation,
    StageKind::InternalDiscovery,
    StageKind::ObjectivePathing,
    StageKind::ObjectiveSimulation,
    StageKind::Reporting,
    StageKind::Cleanup,
];

pub fn load_phase_map_from_json(raw: &str) -> Result<PhaseMap, PhaseMapError> {
    let map: PhaseMap = serde_json::from_str(raw)?;
    map.validate()?;
    Ok(map)
}

impl PhaseMap {
    /// 校验：每个 StageKind 恰好属于一个 phase（不漏不重）。
    pub fn validate(&self) -> Result<(), PhaseMapError> {
        let mut seen: Vec<StageKind> = Vec::new();
        for p in &self.phases {
            for &s in &p.stages {
                if seen.contains(&s) {
                    return Err(PhaseMapError::DuplicateStage(s));
                }
                seen.push(s);
            }
        }
        for s in ALL_STAGES {
            if !seen.contains(&s) {
                return Err(PhaseMapError::UnassignedStage(s));
            }
        }
        Ok(())
    }

    /// stage 所属 phase（按定义顺序找第一个含它的）。
    pub fn phase_of(&self, stage: StageKind) -> Option<&Phase> {
        self.phases.iter().find(|p| p.contains(stage))
    }

    /// 按 profile 投影：每个 phase 只保留 `allowed` 内的成员；成员清空的 phase 整体剔除。
    /// 复用 `Profile::allowed_stage_set`。
    pub fn project(&self, profile: &Profile) -> PhaseMap {
        let allowed = profile.allowed_stage_set();
        let phases = self
            .phases
            .iter()
            .filter_map(|p| {
                let stages: Vec<StageKind> =
                    p.stages.iter().copied().filter(|s| allowed.contains(s)).collect();
                if stages.is_empty() {
                    None
                } else {
                    Some(Phase {
                        id: p.id.clone(),
                        stages,
                        entry_approval: p.entry_approval.clone(),
                    })
                }
            })
            .collect();
        PhaseMap { phases }
    }
}
```
**步骤 2（在同文件末尾加 `#[cfg(test)]`，先写会失败的断言）：**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::profile::load_profile_from_json;

    const PHASES_JSON: &str =
        include_str!("../../../../../resources/harness/graph/phases.json");
    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");

    fn map() -> PhaseMap {
        load_phase_map_from_json(PHASES_JSON).expect("phases.json parses + validates")
    }

    #[test]
    fn phases_cover_all_12_stages_exactly_once() {
        let m = map();
        assert_eq!(m.phases.len(), 5);
        m.validate().expect("every stage assigned exactly once");
    }

    #[test]
    fn phase_of_known_stages() {
        let m = map();
        assert_eq!(m.phase_of(StageKind::Scoping).unwrap().id, "prep");
        assert_eq!(m.phase_of(StageKind::TargetIntel).unwrap().id, "prep");
        assert_eq!(m.phase_of(StageKind::Enumeration).unwrap().id, "active_recon");
        assert_eq!(m.phase_of(StageKind::Verification).unwrap().id, "vuln");
    }

    #[test]
    fn entry_approvals_on_active_recon_and_vuln() {
        let m = map();
        let ar = m.phases.iter().find(|p| p.id == "active_recon").unwrap();
        assert_eq!(ar.entry_approval.as_deref(), Some("active_scan"));
        let vuln = m.phases.iter().find(|p| p.id == "vuln").unwrap();
        assert_eq!(vuln.entry_approval.as_deref(), Some("exploit_validation"));
        let prep = m.phases.iter().find(|p| p.id == "prep").unwrap();
        assert_eq!(prep.entry_approval, None);
    }

    #[test]
    fn assessment_projection_drops_vuln_and_post_exploit() {
        // assessment forbids vuln_triage/verification/access_validation/cleanup.
        let profile = load_profile_from_json(ASSESSMENT_JSON).expect("profile");
        let projected = map().project(&profile);
        let ids: Vec<&str> = projected.phases.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"prep"));
        assert!(ids.contains(&"active_recon"));
        assert!(ids.contains(&"closeout")); // reporting 在 closeout，assessment 允许
        assert!(!ids.contains(&"vuln"));
        assert!(!ids.contains(&"post_exploit"));
        // closeout 在 assessment 只剩 reporting（cleanup 被 forbidden）
        let closeout = projected.phases.iter().find(|p| p.id == "closeout").unwrap();
        assert_eq!(closeout.stages, vec![StageKind::Reporting]);
    }
}
```
**步骤 3：** 在 `harness/mod.rs` 加 `pub mod phase;`（紧挨现有 `pub mod operation_graph;`，按字母序/现有顺序）。
**验证：** `cargo nextest run -p golish-agent-kit -E 'test(harness::phase::)'` → 4/4 PASS（首跑可能因 include 路径报错，按报错修 `../` 层数到与 stage_spec.rs 的 `include_str!` 同深度）。
**提交：** `feat(harness): phase.rs DTO + loader + projection (unit-tested)`。

### Task A3 · phases.json 进 resources 嵌入
**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/resources.rs`
**步骤：** 仿现有 `load_embedded_profile` 加一个 `load_embedded_phase_map`：
```rust
pub fn load_embedded_phase_map() -> Result<super::phase::PhaseMap, super::phase::PhaseMapError> {
    const PHASES_JSON: &str =
        include_str!("../../../../../resources/harness/graph/phases.json");
    super::phase::load_phase_map_from_json(PHASES_JSON)
}
```
（`include_str!` 的 `../` 层数与该文件现有 `include_str!` 对齐。）
**验证：** `cargo build -p golish-agent-kit` → exit 0。
**提交：** `feat(harness): embed phases.json via resources loader`。

---

## Phase B · phase-aware 流转（纯逻辑 + 单测）

### Task B1 · phase_flow.rs：phase 完整判定 + 跨 phase 决策（TDD）
**文件：** 创建 `backend/crates/golish-agent-kit/src/harness/phase_flow.rs`
**步骤 1（写函数）：**
```rust
//! Phase-aware 流转：在 stage 级 gate 之上判定「大阶段是否跑完」「该跨到哪个大阶段」
//! 「跨 phase 前要不要审批」。纯逻辑，不碰 DB。设计 2026-06-03 §6/§7。

use std::collections::HashSet;

use super::phase::{Phase, PhaseMap};
use super::types::StageKind;

/// 给定一组「已 gate PASS 的 stage」，phase 是否完成 = 它的（投影后）成员全部 PASS。
pub fn phase_is_complete(phase: &Phase, gate_passed: &HashSet<StageKind>) -> bool {
    !phase.stages.is_empty() && phase.stages.iter().all(|s| gate_passed.contains(s))
}

/// 当前 stage 所在 phase 之后的下一个 phase（按投影后 PhaseMap 顺序）。None = 已是最后 phase。
pub fn next_phase<'a>(map: &'a PhaseMap, current: StageKind) -> Option<&'a Phase> {
    let idx = map.phases.iter().position(|p| p.contains(current))?;
    map.phases.get(idx + 1)
}

/// 跨入 `target_phase` 前需要的审批 key（None = 不需要 phase 级审批）。
/// de-dup 天然成立：每个 phase 只有一个 `entry_approval`。
pub fn phase_entry_approval(target_phase: &Phase) -> Option<&str> {
    target_phase.entry_approval.as_deref()
}

/// 综合决策：当前 stage 的 gate 已 PASS 后，结合「已 PASS 集合」判断下一步。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhaseStep {
    /// 当前 phase 还没跑完（还有成员 stage 没 PASS）→ 留在本 phase 继续派成员 stage。
    StayInPhase,
    /// 当前 phase 跑完、有下一 phase → 跨入；附带跨入前需要的审批（None=直接进）。
    EnterPhase { phase_id: String, approval: Option<String> },
    /// 当前 phase 跑完、无下一 phase → operation 完成。
    Complete,
}

pub fn decide_phase_step(
    map: &PhaseMap,
    current: StageKind,
    gate_passed: &HashSet<StageKind>,
) -> PhaseStep {
    let Some(cur_phase) = map.phase_of(current) else {
        return PhaseStep::Complete; // 不在任何 phase（被投影剪掉）→ 防御性收尾
    };
    if !phase_is_complete(cur_phase, gate_passed) {
        return PhaseStep::StayInPhase;
    }
    match next_phase(map, current) {
        None => PhaseStep::Complete,
        Some(next) => PhaseStep::EnterPhase {
            phase_id: next.id.clone(),
            approval: phase_entry_approval(next).map(|s| s.to_string()),
        },
    }
}
```
**步骤 2（同文件 `#[cfg(test)]`，先写失败断言）：**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::resources::load_embedded_phase_map;

    fn passed(stages: &[StageKind]) -> HashSet<StageKind> {
        stages.iter().copied().collect()
    }

    #[test]
    fn stay_in_phase_until_all_members_pass() {
        let map = load_embedded_phase_map().unwrap();
        // prep = [scoping, target_intel]; 只过了 scoping
        let step = decide_phase_step(&map, StageKind::Scoping, &passed(&[StageKind::Scoping]));
        assert_eq!(step, PhaseStep::StayInPhase);
    }

    #[test]
    fn enter_active_recon_requires_active_scan_approval() {
        let map = load_embedded_phase_map().unwrap();
        // prep 全过 → 跨入 active_recon，需 active_scan 审批
        let gp = passed(&[StageKind::Scoping, StageKind::TargetIntel]);
        let step = decide_phase_step(&map, StageKind::TargetIntel, &gp);
        assert_eq!(
            step,
            PhaseStep::EnterPhase {
                phase_id: "active_recon".to_string(),
                approval: Some("active_scan".to_string())
            }
        );
    }

    #[test]
    fn enter_vuln_requires_exploit_validation_approval() {
        let map = load_embedded_phase_map().unwrap();
        let gp = passed(&[
            StageKind::Scoping, StageKind::TargetIntel,
            StageKind::ExternalAttackSurface, StageKind::Enumeration,
        ]);
        let step = decide_phase_step(&map, StageKind::Enumeration, &gp);
        assert_eq!(
            step,
            PhaseStep::EnterPhase {
                phase_id: "vuln".to_string(),
                approval: Some("exploit_validation".to_string())
            }
        );
    }

    #[test]
    fn last_phase_completes() {
        let map = load_embedded_phase_map().unwrap();
        let mut gp = passed(&[StageKind::Reporting, StageKind::Cleanup]);
        // 防御：closeout 成员全过 → Complete
        let step = decide_phase_step(&map, StageKind::Cleanup, &gp);
        assert_eq!(step, PhaseStep::Complete);
        gp.remove(&StageKind::Cleanup);
        // reporting 过但 cleanup 没过（红队场景）→ 仍 StayInPhase
        assert_eq!(
            decide_phase_step(&map, StageKind::Reporting, &gp),
            PhaseStep::StayInPhase
        );
    }
}
```
**步骤 3：** `harness/mod.rs` 加 `pub mod phase_flow;`。
**验证：** `cargo nextest run -p golish-agent-kit -E 'test(harness::phase_flow::)'` → 4/4 PASS。
**提交：** `feat(harness): phase_flow phase-aware transition (unit-tested)`。

---

## Phase C · 放开阶段内顺序（数据 + 依赖重审）

### Task C1 · 逐条重审 requires_stages（填工作表，先决定再改）
**文件：** 修改 `docs/design/2026-06-02-stage-spec-worksheet.csv`（追加一列 `dep_classification`）
**步骤：** 对每条 `requires_stages` 边填「real（真实数据依赖，保留）/ artificial（人为顺序，可解除）」。已知事实（已读 stage JSON）：
- `external_attack_surface.requires_stages = [scoping, target_intel]` → 跨 phase 依赖（target_intel 在 prep），**保留**（real）。
- `enumeration.requires_stages = [external_attack_surface]` → ② 内边；**判定**：端口/目录枚举是否必须先有攻击面测绘结果？默认 **real（保留）**，除非用户/复审改判 artificial。
- `verification.requires_stages = [vuln_triage]` → **real，保留**。
- `target_intel.requires_stages` → 若为 `[scoping]` 则 real（先确认范围再被动情报）。
**验证：** worksheet 每条边都有 `dep_classification`，无空。
**提交：** `docs(harness): classify requires_stages edges (real vs artificial)`。

### Task C2 · 据重审结果删 operation_graph.json 的人为边
**文件：** 修改 `resources/harness/graph/operation_graph.json`
**步骤：** 仅删 C1 判为 artificial 的边。若 C1 把所有现有 requires 都判 real（保守默认），则本 task 只确认无人为边可删、记录决定，不改 JSON（边集保持 15 条）。**任何删边后必须保证图仍无环且每个 allowed phase 至少 1 个成员可达。**
**验证：** `cargo nextest run -p golish-agent-kit -E 'test(harness::operation_graph::)'` 全绿（若改了边，按 `base_graph_has_12_nodes_15_edges`/`project_assessment_yields_5_nodes_5_edges` 等断言更新期望边数，并在 commit message 注明根因）。
**提交：** `refactor(harness): drop artificial intra-phase ordering edges`（无删则跳过本 commit）。

---

## Phase D · 审批落 phase 边界（运行时接线）

> **执行者须先读：** `task_orchestrator/subtask_phases/execute.rs` 里现有 gate hook + transition 调用处（搜 `decide_from_gate` / `stage_entry_requires_approval` / `advance_stage`），以及 `harness/stage_transition.rs:84 stage_entry_requires_approval`（已读，纯函数）。本 Phase 只在 flag 开时把「per-stage 审批判定」替换为「phase 边界审批判定」。

### Task D1 · phase_flow 暴露「跨 phase 是否要审批」给运行时
**文件：** 已在 Task B1 的 `decide_phase_step` 返回 `EnterPhase { approval }` 内提供。本 task 加一个便捷判定供运行时直接用：
```rust
// phase_flow.rs 追加
/// 运行时便捷：当前 stage gate PASS 后，跨 phase 是否需要先审批（返回审批 key）。
/// 仅当 `decide_phase_step` == EnterPhase 且 approval=Some 时返回 Some。
pub fn pending_phase_approval(
    map: &PhaseMap,
    current: StageKind,
    gate_passed: &HashSet<StageKind>,
) -> Option<String> {
    match decide_phase_step(map, current, gate_passed) {
        PhaseStep::EnterPhase { approval, .. } => approval,
        _ => None,
    }
}
```
**步骤 + 验证：** 加单测 `pending_phase_approval` 在 prep 全过时返回 `Some("active_scan")`、在 prep 未过时返回 `None`；`cargo nextest -p golish-agent-kit -E 'test(harness::phase_flow::)'` 全绿。
**提交：** `feat(harness): pending_phase_approval helper`。

### Task D2 · execute.rs 审批判定 flag 切换
**文件：** 修改 `task_orchestrator/subtask_phases/execute.rs`
**步骤：** 在现有「gate PASS 后、推进游标前」调用 `stage_entry_requires_approval(next_spec, profile)` 的位置，包一层 flag 分支：
- `GOLISH_HARNESS_TWO_LEVEL` ON：用 `phase_flow::pending_phase_approval(&projected_phase_map, current_stage, &gate_passed_set)` 决定是否 hold 等审批（`gate_passed_set` 见 Task E1 的累积集合）。
- OFF：保持现有 `stage_entry_requires_approval` 逻辑（零回归）。
（具体插桩点 = 现有 approval hold 分支；执行者按读到的真实代码就地包 `if two_level_enabled() { ... } else { ...现状... }`。）
**验证：** Phase D/E 共用集成测试（Task E2）覆盖；本 task 末跑 `cargo build -p golish-agent-kit` + `-p golish-agent-app` → exit 0。
**提交：** `feat(harness): route approval through phase boundary when two-level on`。

---

## Phase E · 运行时游标 phase-aware

> **执行者须先读：** `task_orchestrator/subtask_phases/execute.rs`（gate hook + `decide_from_gate` + 游标推进 `operation_state::advance_stage` 调用）、`task_orchestrator/stage_execution.rs`、`harness/operation_flow.rs`（graph-flow 路径，节点体 gate→Interrupt/ExecutedWith）。三处共用 `AllowedDag` + `branch_target`，本 Phase 在它们之上叠加「phase 门」：phase 没跑完就别真跨 phase。

### Task E1 · 维护「已 gate PASS 的 stage 集合」
**文件：** 修改 `task_orchestrator/subtask_phases/execute.rs`（或 `stage_execution.rs` 中持有 operation 运行态处）
**步骤：** flag 开时，在 operation 运行态里维护 `gate_passed: HashSet<StageKind>`，每次某 stage gate PASS 就 `insert`。优先从已持久化的 `operation_state`/`stage_runs`（见 `golish-db/src/repo/operation_state.rs`）派生，避免新增持久化字段（遵守 I10）；MVP 可先用进程内集合（与现有 C6 handoff 内存级一致）。
**验证：** `cargo build -p golish-agent-kit` exit 0；Task E2 集成测试覆盖。
**提交：** `feat(harness): track gate-passed stage set for phase gating`。

### Task E2 · gate PASS 后用 phase_flow 决定跨 phase（核心接线）
**文件：** 修改 `task_orchestrator/subtask_phases/execute.rs` + `task_orchestrator/stage_execution.rs`
**步骤：** flag 开时，把「gate PASS → `decide_from_gate` → `advance_stage`」改为两段：
1. 先 `decide_from_gate`（拓扑候选不变）。
2. 再 `phase_flow::decide_phase_step(&projected_phase_map, current, &gate_passed)`：
   - `StayInPhase` → 不跨 phase，按现有逻辑在本 phase 内继续派下一个未完成成员 stage（成员顺序受 §Phase C 重审后的 requires 边约束）。
   - `EnterPhase{approval}` → 若 `approval=Some` 且未获批 → hold 等审批；获批后才 `advance_stage` 到下一 phase 的入口 stage。
   - `Complete` → operation 收尾（同现有 Complete 路径）。
**写集成测试：** 新建 `task_orchestrator/subtask_phases/execute_phase_flow_tests.rs`（仿现有 `execute_harness_loop_tests.rs`），用进程内 stub：模拟 prep 两 stage 依次 PASS → 断言「prep 未全 PASS 时不跨 phase」「prep 全 PASS 后 pending active_scan 审批」「审批后游标到 external_attack_surface」。
**验证：** `cargo nextest run -p golish-agent-kit -E 'test(execute_phase_flow)'` 全绿；`GOLISH_HARNESS_TWO_LEVEL=1 cargo nextest run -p golish-agent-kit` 全绿（flag on 路径健康）；flag off 全量 nextest 不回归。
**提交：** `feat(harness): phase-gated cursor advance in subtask loop`。

### Task E3 · graph-flow 路径同款 phase 门
**文件：** 修改 `harness/operation_flow.rs`
**步骤：** 在节点体「gate PASS → 选下一节点」处，叠加 `phase_flow::decide_phase_step`：phase 未完成则 stay（节点 `Update` 回本 phase 下一成员）；需审批则 `Interrupt`（复用现有 Interrupt/resume 暂停语义）。flag off 时保持现状。
**验证：** `cargo nextest run -p golish-agent-kit -E 'test(harness::operation_flow::)'` 全绿（按需补 1 个 phase-gate 节点测试）。
**提交：** `feat(harness): phase gate in graph-flow engine path`。

---

## Phase F · 前端 phase 分组展示

> **执行者须先读：** `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`（现有 `subtask_completed`/`task_progress` 处理 + `addConversationStageMarker`）、`frontend/components/AIChatPanel/StageMarker.tsx`（commit `5fe447d` 已建的 stage 分隔卡）。

### Task F1 · stage→phase 映射 + 分组分隔卡
**文件：** 修改 `frontend/components/AIChatPanel/StageMarker.tsx`（或新增同目录 `phaseGrouping.ts`）
**步骤：** 加一个前端常量 `STAGE_TO_PHASE`（与 phases.json 一致：scoping/target_intel→prep, eas/enumeration→active_recon, …）+ `PHASE_LABEL`（中文标题）。`StageMarker` 渲染时，若该 stage 是其 phase 的**首个**出现的 stage，额外画一条「▶ 进入大阶段：X」的 phase 头；phase 切换处渲染「✓ 大阶段 X 完成 → 进入 Y（已过审批）」。
**步骤（测试）：** 仿现有 `frontend/components/AIChatPanel/*.test.tsx` 加用例：连续 stage 同 phase 不重复画 phase 头；跨 phase 画一次 phase 边界卡。
**验证：** `just check-fe`（biome + typecheck）exit 0；`just test-fe` 全绿（含新用例）；`ReadLints` 改动文件无错。
**提交：** `feat(ui): group stage markers by phase in chat panel`。

---

## Phase G · flag 接线 + 集成 + 收口

### Task G1 · two-level flag 读取
**文件：** 修改读取 harness flag 的处（搜 `GOLISH_HARNESS_GRAPH_FLOW` 的 env 读取点，同处加 `GOLISH_HARNESS_TWO_LEVEL`）
**步骤：** 加 `pub fn two_level_enabled() -> bool { std::env::var("GOLISH_HARNESS_TWO_LEVEL").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false) }`，放在 graph-flow flag 同模块；§Phase D/E 的分支调它。
**验证：** `cargo build -p golish-agent-kit` exit 0。
**提交：** `feat(harness): GOLISH_HARNESS_TWO_LEVEL flag`。

### Task G2 · 集中编译 + 全量验证
**文件：** 无（验证）
**步骤 + 验证：**
- `cargo nextest run -p golish-agent-kit --no-fail-fast` → 全绿（flag off 基线）
- `GOLISH_HARNESS_TWO_LEVEL=1 cargo nextest run -p golish-agent-kit` → 全绿（flag on 路径）
- `cargo clippy -p golish-agent-kit -p golish-agent-app -- -D warnings` → exit 0
- `cargo check -p golish-agent-app -p golish-agent-runtime -p golish-agent-bridge` → exit 0
- `just check-fe` + `just test-fe` → 全绿
- `just precommit` → 全绿
**提交：** 不单独 commit（验证步）。

### Task G3 · 活体 E2E（用户·需运行时）
**步骤：** `GOLISH_HARNESS_TWO_LEVEL=1 just dev` → 发 example.com 外部攻击面侦察任务 → 盯 `~/.golish/backend.log`：
- ① prep（scoping·target_intel）各自 gate PASS，prep 内不强制等审批；
- 跨入 ② 前弹 `active_scan` 审批一次；批准后 eas/enumeration 推进；
- 跨入 ③ 前弹 `exploit_validation` 审批一次；
- 游标按大阶段前进，gate 仍逐 stage PASS（不再 skip）。
**验证：** 把上述日志片段贴进 `agent-progress.md`「已记录证据」。

---

## 自检（规格覆盖度对照设计 §1–§12）

| 设计章节 | 对应任务 | 覆盖 |
|---|---|---|
| §4 D1 两级模型 | A1/A2 | ✅ |
| §4 D2 / §5 分组（乙） | A1（phases.json）| ✅ |
| §4 D3 丙完整判定 | B1 `phase_is_complete` | ✅ |
| §4 D4 / §5 三审批对齐跃迁 | A1 entry_approval + D1/D2 | ✅ |
| §4 D5 甲 per-stage gate 不动 | 全程不碰 `gate/` | ✅ |
| §4 D6 / §8 去人为顺序 | C1/C2 | ✅ |
| §6.1 phases.json | A1 | ✅ |
| §6.2 DAG 改造 | C2 | ✅ |
| §6.3 phase-aware 遍历 | B1 | ✅ |
| §6.4 审批 de-dup at boundary | D1/D2 | ✅ |
| §6.5 运行时游标 | E1/E2/E3 | ✅ |
| §6.6 gate 不动 | （约束） | ✅ |
| §6.7 前端 | F1 | ✅ |
| §6.8 profile 不动结构 | A2 `project` 复用 profile | ✅ |
| §9 flag/回滚 | G1 | ✅ |
| §12 验证计划 | G2/G3 | ✅ |

**占位符扫描：** 无 TODO/待定；每个新文件/函数都有完整代码；集成 task（D2/E2/E3）明确「先读 X 文件、就地包 flag 分支」，因这些运行时函数体未在本计划逐行内联（执行者须读真实代码再插桩），不是占位符而是接线指引。
**类型一致性：** `PhaseMap`/`Phase`/`PhaseStep`/`phase_is_complete`/`decide_phase_step`/`pending_phase_approval`/`two_level_enabled` 跨 Task A2→B1→D1→E2→G1 命名一致。

---

## 范围与风险

- **范围**：单一连贯子系统（phase 层），但跨 kit + runtime + frontend。若执行者觉得偏大，可按 Phase A/B/C（kit 纯逻辑，可独立 merge 且零行为变更）先交一批，再做 D/E/F/G（运行时+前端接线）第二批——两批都能独立编译+测试。
- **风险**：运行时插桩（D2/E2/E3）依赖真实函数体，务必先读后改；flag off 必须零回归（每 Phase 末验证）。gate 内核全程不动，证据契约（I7/I8）天然保住。
