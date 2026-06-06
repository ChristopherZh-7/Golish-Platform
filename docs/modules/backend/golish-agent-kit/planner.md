# golish-agent-kit / planner

> **一句话职责**：agent 任务计划管理——`PlanManager` 线程安全访问 `TaskPlan`（校验 + 可选 PG 持久化 + prompt 注入格式化）；核心 plan 类型来自 `golish-core::plan`。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/planner/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 agent 多步计划的创建/更新/校验（`update_plan` 工具背后的状态）时
- 改计划 PG 持久化或 prompt 注入格式时

## 职责

`PlanManager` 提供对 `TaskPlan` 的线程安全访问：步骤校验（步数上下限）、可选 PostgreSQL 持久化、把当前计划格式化注入 prompt。核心类型（`PlanStep`/`StepStatus`/`TaskPlan`）在 `golish-core::plan`，此处 re-export。

## 公开接口

| 符号 | 说明 |
|---|---|
| `PlanManager` | 计划运行时（校验 + 持久化 + prompt 格式化） |
| `TaskPlan` / `PlanStep` / `StepStatus` / `PlanSummary` / `FailureKind`（re-export 自 core） | 计划类型 |
| `MAX_PLAN_STEPS` / `MIN_PLAN_STEPS` | 步数约束 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export core plan 类型 |
| `manager.rs` | `PlanManager` 运行时 |

## 依赖

- `golish-core::plan`（核心类型）；可选 PG（经 `db_traits`）

## 注意事项 / 坑

- 计划类型的单一事实源在 `golish-core::plan`；别在此另定义重复类型。
- 步数受 `MIN/MAX_PLAN_STEPS` 约束，校验失败应返回错误而非静默截断。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit planner
```
