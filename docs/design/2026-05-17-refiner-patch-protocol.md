# P0-2 · Refiner Patch 协议 实现计划

> **状态**：分两阶段交付。
> - **阶段 1（本计划范围）**：拓展 `PlanStep` 数据模型加入 `failure_kind` 元数据 + apply patch ops 的 `PlanManager` 内部方法 + 单测。
> - **阶段 2（P1 计划）**：新增 `update_plan_patch` 工具暴露给 LLM、引导系统 prompt、ChatPanel UI 加失败分类徽章。
>
> 把 P0-2 拆两阶段是为了不在一晚上动 system prompt（容易回归），但同时把 patch-able 的内核搭好，下一阶段只需暴露 schema。

**目标**：让 `PlanManager` 内部具备「按 patch 操作演化 plan」的能力，并把 PentAGI 的 4 类失败分类（Technical / Environmental / Conceptual / External）作为可选元数据嵌入 `PlanStep`。

**架构**：

```
PlanPatchOp ::= Add { after_id?, title, status? }
             | Remove { id }
             | Modify { id, title?, status?, failure_kind? }
             | Reorder { id, after_id? }

PlanManager::apply_patch_ops(ops, failure_kind?) -> Result<TaskPlan, PlanError>
```

`apply_patch_ops` 是 `update_plan` 的兄弟方法，**不**通过工具暴露给 LLM（避免改 system prompt），但可被未来的 `update_plan_patch` 工具复用。

**技术栈**：Rust (`golish-core::plan` + `golish-agent-kit::planner`).

---

## 0. TL;DR

- 加 `FailureKind` 枚举（Technical/Environmental/Conceptual/External）
- 给 `PlanStep` 加 `failure_kind: Option<FailureKind>` 字段
- 加 `PlanManager::apply_patch_ops(ops: Vec<PlanPatchOp>)` 方法
- 单测 cover 4 类 op + 失败分类
- 暂不暴露给 LLM（P1 工作），不动现有 update_plan 工具

## 1. 数据模型变化

### 1.1 `golish-core::plan::FailureKind`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Different command / tool / parameter would solve it.
    Technical,
    /// Missing dependency / wrong config.
    Environmental,
    /// Approach itself is wrong, needs replanning.
    Conceptual,
    /// Out of system control (rate limit, target offline).
    External,
}
```

### 1.2 `PlanStep` 加 `failure_kind`

```rust
pub struct PlanStep {
    pub id: Option<String>,
    pub step: String,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<FailureKind>,  // ← new
}
```

`#[serde(default)]` 保证向后兼容：DB 里旧 plan 反序列化时 `failure_kind = None`。前端不读这个字段也不出错。

### 1.3 `PlanPatchOp`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PlanPatchOp {
    Add {
        after_id: Option<String>,
        title: String,
        status: Option<StepStatus>,
    },
    Remove { id: String },
    Modify {
        id: String,
        title: Option<String>,
        status: Option<StepStatus>,
        failure_kind: Option<FailureKind>,
    },
    Reorder {
        id: String,
        after_id: Option<String>,
    },
}
```

## 2. 实现清单

| 层 | 文件 | 改动 |
|---|---|---|
| 核心类型 | `backend/crates/golish-core/src/plan.rs` | `FailureKind` enum、`PlanStep.failure_kind` field |
| Patch 类型 | `backend/crates/golish-agent-kit/src/planner/mod.rs` | `PlanPatchOp` enum |
| Manager | `backend/crates/golish-agent-kit/src/planner/manager.rs` | `apply_patch_ops(ops)` |
| 单测 | `backend/crates/golish-agent-kit/src/planner/tests/patch_tests.rs` (new) | 8 个测试 |
| 测试 mod | `planner/tests.rs` | `mod patch_tests;` |

## 3. 任务拆分

### Task 1: 加 FailureKind + PlanStep 字段

`golish-core/src/plan.rs`：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Technical,
    Environmental,
    Conceptual,
    External,
}

impl std::fmt::Display for FailureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            FailureKind::Technical => "technical",
            FailureKind::Environmental => "environmental",
            FailureKind::Conceptual => "conceptual",
            FailureKind::External => "external",
        };
        write!(f, "{}", s)
    }
}

pub struct PlanStep {
    pub id: Option<String>,
    pub step: String,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<FailureKind>,
}
```

**验证**：`cargo check -p golish-core` 通过；现有反序列化测试通过（向后兼容）。

**提交**：`feat(core/plan): add FailureKind enum + PlanStep.failure_kind`

### Task 2: PlanPatchOp + apply_patch_ops

在 `planner/mod.rs` 加 `PlanPatchOp` 枚举。

在 `planner/manager.rs` 加 `apply_patch_ops`：

```rust
pub async fn apply_patch_ops(
    &self,
    ops: Vec<super::PlanPatchOp>,
) -> Result<TaskPlan, PlanError> {
    let mut plan = self.plan.write().await;
    let mut steps = plan.steps.clone();

    for op in ops {
        match op {
            super::PlanPatchOp::Add { after_id, title, status } => {
                let new_step = PlanStep {
                    id: Some(uuid::Uuid::new_v4().to_string()),
                    step: title,
                    status: status.unwrap_or(StepStatus::Pending),
                    failure_kind: None,
                };
                let idx = after_id
                    .as_ref()
                    .and_then(|aid| steps.iter().position(|s| s.id.as_deref() == Some(aid.as_str())))
                    .map(|i| i + 1)
                    .unwrap_or(0);
                steps.insert(idx, new_step);
            }
            super::PlanPatchOp::Remove { id } => {
                steps.retain(|s| s.id.as_deref() != Some(id.as_str()));
            }
            super::PlanPatchOp::Modify { id, title, status, failure_kind } => {
                if let Some(step) = steps.iter_mut().find(|s| s.id.as_deref() == Some(id.as_str())) {
                    if let Some(t) = title { step.step = t; }
                    if let Some(s) = status { step.status = s; }
                    if failure_kind.is_some() { step.failure_kind = failure_kind; }
                }
            }
            super::PlanPatchOp::Reorder { id, after_id } => {
                if let Some(pos) = steps.iter().position(|s| s.id.as_deref() == Some(id.as_str())) {
                    let step = steps.remove(pos);
                    let new_idx = after_id
                        .as_ref()
                        .and_then(|aid| steps.iter().position(|s| s.id.as_deref() == Some(aid.as_str())))
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    steps.insert(new_idx.min(steps.len()), step);
                }
            }
        }
    }

    // Enforce MAX_PLAN_STEPS
    if steps.len() > MAX_PLAN_STEPS {
        return Err(PlanError::InvalidStepCount(steps.len()));
    }
    // Enforce at most 1 in_progress
    let in_prog = steps.iter().filter(|s| s.status == StepStatus::InProgress).count();
    if in_prog > 1 {
        return Err(PlanError::MultipleInProgress(in_prog));
    }

    plan.steps = steps;
    plan.summary = PlanSummary::from_steps(&plan.steps);
    plan.version += 1;
    plan.updated_at = chrono::Utc::now();

    Ok(plan.clone())
}
```

**验证**：`cargo check -p golish-agent-kit`。

**提交**：`feat(planner): apply_patch_ops + PlanPatchOp variants`

### Task 3: 单测覆盖 patch ops

`planner/tests/patch_tests.rs` 8 个测试：
- add at beginning（after_id=None）
- add after existing step
- remove existing step
- remove nonexistent step（no-op）
- modify title / status / failure_kind 三组合
- reorder to end / to middle
- 超过 MAX_PLAN_STEPS 触发 `InvalidStepCount`
- 复合 ops 一次性应用（remove+add+modify）

**提交**：`test(planner): cover apply_patch_ops + FailureKind`

## 4. 风险

- **向后兼容**：`failure_kind` 用 `#[serde(default, skip_serializing_if = "Option::is_none")]`，旧 DB 反序列化不报错，新写入不在 JSON 中出现 `null`
- **emit 事件**：本阶段 `apply_patch_ops` **不 emit** PlanUpdated（避免与未来的工具 emit 冲突）；阶段 2 工具 wrapper 负责 emit
- **DB 落库**：本阶段不落库；阶段 2 工具 wrapper 复用 `update_plan` 的落库路径
- **AiEvent::PlanUpdated 的 PlanStep**：现有事件序列化已经走 `golish-core::plan::PlanStep`，加字段后 frontend 收到的 JSON 多一个可选字段，前端 `interface PlanStep` 用 `failure_kind?: string` 即可（P1 一并更新）

## 5. 不做的事

- 新增 `update_plan_patch` 工具 → P1
- 系统 prompt 引导 → P1
- 前端 UI 失败分类徽章 → P1
- emit `PlanUpdated` from `apply_patch_ops` → P1

---

**文档版本**：v1.0 · 2026-05-17 · author: fullstack_dev
