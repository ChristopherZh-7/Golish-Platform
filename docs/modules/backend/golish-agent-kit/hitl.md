# golish-agent-kit / hitl

> **一句话职责**：Human-in-the-Loop 工具审批——`ApprovalRecorder` 跟踪每工具审批模式（`ApprovalPattern` 统计），高审批率工具自动放行（pattern learning）。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/hitl/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改工具审批记录、自动放行阈值（`HITL_AUTO_APPROVE_*`）、审批模式学习时
- 改 `ApprovalDecision` / `RiskLevel` / `ToolApprovalConfig` 行为时

## 职责

`ApprovalRecorder` 记录每工具的审批历史并算 `ApprovalPattern`，对超过阈值（次数 + 比率）的工具建议自动放行。核心类型来自 `golish-core::hitl`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ApprovalRecorder` / `ApprovalRequest` | 审批记录器 + 请求 |
| `ApprovalDecision` / `ApprovalPattern` / `RiskLevel` / `ToolApprovalConfig`（re-export 自 core） | 决策/统计/风险/配置 |
| `HITL_AUTO_APPROVE_MIN_APPROVALS` / `HITL_AUTO_APPROVE_THRESHOLD` | 自动放行阈值 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export core hitl 类型 |
| `approval_recorder.rs` | `ApprovalRecorder` 实现 |

## 依赖

- `golish-core::hitl`（核心类型）

## 注意事项 / 坑

- 自动放行是安全敏感：阈值（`MIN_APPROVALS` + `THRESHOLD`）放太松会绕过人审；改要谨慎。
- 高风险工具（删文件/发布等）即使高审批率也应慎重自动放行——遵守 AGENTS.md §2.7。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit hitl
```
