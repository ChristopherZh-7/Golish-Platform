# golish-agent-kit / db_tracking

> **一句话职责**：agent 活动的 DB 跟踪——`DbTracker` 记工具生命周期/token/终端/web/审计；普通记录可后台写，但 tool-call start/finish 是可 await 的有序安全边界，防止 gate 先读到 finish 却没有 start。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/db_tracking/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 agent 后台记录（工具调用/token/终端/搜索/审计 写 PG）、记忆存取时
- 排查记录未落库（PG 就绪门）、`ToolCallGuard` 生命周期时

## 职责

`DbTracker` 是穿过 agent loop 的轻量句柄。token/终端/搜索等普通记录仍可
fire-and-forget；`start_tool_call` / `finish_tool_call` 返回 future，runtime 必须按顺序
await，保证 ask_human `scope_review` 等 gate-sensitive 调用在评分前完整可见。
tracker clones 共享同一个可重绑定 session UUID，TaskMode 在 stage 执行前把它绑定到
durable `sessions.id`。写入经 `DbReadinessGate` 门控；失败记 warn，不 panic。

## 公开接口

| 符号 | 说明 |
|---|---|
| `DbTracker` | 记录句柄；clones 共享 session identity；tool-call start/finish 可 await，其它非 gate-sensitive 写可后台化 |
| `ToolCallGuard` | 工具调用生命周期 guard；固定保存 start 时的 session UUID，finish 不重新读取可变 identity |
| `MemoryHit` / `ScoredMemoryHit` / `BriefingPlan` | 记忆命中 / 评分 / briefing |
| `LegacyToolMemoryContext` / `should_store_legacy_tool_memory` | 自动 tool-result legacy memory 的硬边界；任何 trusted harness operation/stage 一律禁止写自由文本 memory |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `DbTracker` 句柄 |
| `recording.rs` / `memory/{store,policy}.rs` / `helpers.rs` / `types.rs` | 记录 / 记忆 writer + harness cutoff policy / helper / DTO |

## 依赖

- crate 内 `db_traits`（`DbTrackingBackend`/`DbReadinessGate`/`TextEmbedder`）、`uuid`、`tokio`、`parking_lot`

## 注意事项 / 坑

- **工具生命周期不得乱序**：`start_tool_call` 完成后才 dispatch，`finish_tool_call`
  完成后 gate 才可消费该调用；不得恢复为两个互相竞态的 spawned write。
- **session identity 必须共享且成对固定**：Task/stage 开始前 rebind durable UUID，所有
  已克隆 tracker 必须立即读到同一值；每次记录只短暂 snapshot UUID，不能持锁跨
  `await`。`ToolCallGuard` 必须让同一调用的 start/finish 永远使用 start UUID，即使
  bridge identity 随后发生合法 rebind。
- 经 trait（db_traits）写，不直接依赖 golish-db；就绪门防 PG 启动期撞超时。
- `maybe_store_tool_memory` 必须显式接收 `LegacyToolMemoryContext`；`HarnessCustomerFact` 直接返回，canonical customer facts 只能经受约束的 row + outbox projector。显式 general-memory 工具是独立用户授权路径，不属于自动 tool-result writer。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit db_tracking
```
