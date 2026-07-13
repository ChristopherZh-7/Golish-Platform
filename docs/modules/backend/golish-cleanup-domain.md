# golish-cleanup-domain

> **一句话职责**：Cleanup P7a 的纯领域契约——side-effect 与 obligation 一一绑定、retry-safe Attempt 状态机、独立 absence proof 与 residual risk。

- **类型**：crate（Layer 1 · pure domain）
- **路径**：`backend/crates/golish-cleanup-domain/`
- **状态**：✅ C5/P7a 已实现

## 何时该读这张卡

- 修改 cleanup obligation/attempt/absence/waiver 状态机时
- 增加 Post-Exploit side effect 或 Cleanup Gate 语义时
- 判断“清理失败”“校验不确定”和“已验证不存在”的区别时

## 职责

- `PendingSideEffectAction` 必须匹配一个 exact `NewCleanupObligation`；只读 action 不能伪造 obligation。
- live Attempt 仅有 `claimed`、`executing`、`cleaned_pending_verification`。
- absence `inconclusive` / `still_present` 关闭当前 Attempt 为 `verification_failed`，但 obligation 回到 `open`，允许下一 ordinal；任何 Attempt 一旦进入 `verified_absent|verification_failed|execution_failed`，该 retained terminal row 不再原地更新或删除，重试只能创建新 ordinal。
- `verified_absent` 才是无 residual 的成功终态；waiver 必须保存 residual risk，并携带完整 frozen operation/project/snapshot/org identity 与 row-version CAS。
- `TrustedOperatorPrincipal` 不实现 serde，调用方不能从 request/model DTO 构造 actor。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `CleanupObligation` / `NewCleanupObligation` | frozen scope、source action、resource snapshot、strategy 与 proof contract |
| `CleanupAttemptStatus` / `apply_absence_result` | retry-safe attempt/obligation 联合迁移 |
| `PendingSideEffectAction` / `validate_action_obligation_pair` | action + obligation exact identity 校验 |
| `TrustedOperatorPrincipal` / `WaiverRequest` | server-owned waiver actor 与 CAS 请求 |
| `ResidualRisk` | waiver/reporting 使用的结构化残余风险 |

## 依赖

- **内部**：`golish-post-exploit-domain`（typed Action/SideEffectClass）
- **外部**：`chrono`、`serde`、`serde_json`、`thiserror`、`uuid`

## 注意事项 / 坑

- `verification_failed` 不是 live 状态，也不是 obligation terminal；不得阻止下一 ordinal。
- cleanup evidence 与 absence evidence 必须独立，不能复用同一 evidence id 冒充二次验证。
- opaque principal 的 server constructor 只允许 trusted adapter 使用，不能加 serde/公开字段；`WaiverRequest` 的 exact scope 字段是资源授权，不得退化成只传 obligation id。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-cleanup-domain --status-level fail
```
