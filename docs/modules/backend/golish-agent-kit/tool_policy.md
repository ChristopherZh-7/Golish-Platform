# golish-agent-kit / tool_policy

> **一句话职责**：AI 工具策略访问控制——`ToolPolicy`（allow/prompt/deny）+ `ToolConstraints`（per-tool 限制）+ `ToolPolicyManager`（加载 `.golish/tool-policy.json`、两级 global/project 配置、评估、预批、full-auto 状态）。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/tool_policy/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改工具策略评估（allow/prompt/deny）、glob 匹配、per-tool 约束、full-auto 时
- 改 `.golish/tool-policy.json` 加载/保存或 global/project 两级合并时

## 职责

策略式工具访问控制：`types`（`ToolPolicy`/`ToolConstraints` + glob 匹配 + `PolicyConstraintResult`）；`defaults`（默认工具目录 typed+dynamic、`is_known_tool`、`ToolPolicyConfig` + Default）；`manager`（运行时评估、两级 global/project 配置、持久化、预批、full-auto）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ToolPolicyManager` | 策略加载/评估/持久化/预批/full-auto |
| `ToolPolicy`（allow/prompt/deny） / `ToolConstraints` / `PolicyConstraintResult` | 策略/约束/结果 |
| `ToolPolicyConfig` / `is_known_tool` | 配置 / 已知工具判定 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `manager.rs` | `ToolPolicyManager`（评估/两级配置/持久化） |
| `types.rs` | `ToolPolicy`/`ToolConstraints` + glob 匹配 |
| `defaults.rs` | 默认目录 + `ToolPolicyConfig` |

## 依赖

- crate 内；`.golish/tool-policy.json`（serde）

## 注意事项 / 坑

- 模块标 `#![allow(dead_code)]`（部分为未来集成）。
- 两级配置：project 覆盖 global；full-auto 是放开所有 prompt——安全敏感，别误开。
- 与 `hitl/`（审批学习）互补：policy 是声明式，hitl 是行为学习。
- `list_in_scope_targets` 是 server-scoped、无网络和无写入的 harness census，属于 dynamic 默认 `Allow` 且包含在 planning allowlist；它不代表目标授权，也不能扩大 operation/org scope。shell、probe、scope mutation 仍使用各自 Prompt/Deny policy。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit tool_policy
```
