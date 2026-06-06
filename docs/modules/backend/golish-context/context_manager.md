# golish-context / context_manager

> **一句话职责**：上下文窗口管理编排——协调 token 预算、上下文压缩（compaction）、截断策略，含 compaction 状态机与决策结果类型。

- **类型**：目录模块（属于 crate [`golish-context`](../golish-context.md)）
- **路径**：`backend/crates/golish-context/src/context_manager/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `ContextManager` 的压缩触发、trim 策略、enforcement 步骤时
- 改 compaction 状态机（`CompactionState` / `CompactionCheck`）或告警/效率指标事件时

## 职责

`ContextManager` 编排上下文窗口管理：何时压缩、如何截断、token 预算检查与 enforcement。owns compaction 状态机 + 决策结果，以及 alert/efficiency/summary/warning 等上下文事件。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ContextManager` | 上下文管理主体 + chars 估算 helper |
| `ContextManagerConfig` / `ContextTrimConfig` | trim 策略 + 高层设置 facade |
| `CompactionState` / `CompactionCheck` | 压缩状态机 + 决策结果 |
| `ContextEvent` / `ContextWarningInfo` / `ContextSummary` / `ContextEfficiency` / `ContextEnforcementResult` | 上下文告警/警告/摘要/效率/enforcement 结果 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `manager.rs` | `ContextManager` 本体 + chars 估算 |
| `state.rs` | compaction 状态机 + 决策结果 |
| `config.rs` | trim 策略 + 设置 facade |
| `events.rs` | 告警/效率/摘要/enforcement 事件 |

## 依赖

- crate 内 `token_budget`（预算）；`serde`

## 注意事项 / 坑

- 模块标了 `#![allow(dead_code)]`——部分公开 API 为未来用，当前未全调用，删"死"方法前确认无下游计划。
- 压缩与 token 预算（`token_budget/`）是配对的：改压缩阈值要对齐预算限制。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-context context_manager
```
