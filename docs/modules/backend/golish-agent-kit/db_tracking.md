# golish-agent-kit / db_tracking

> **一句话职责**：agent 活动的后台 DB 跟踪——`DbTracker` 把工具调用/token 用量/终端输出/web 搜索/审计 fire-and-forget 写 PG，不阻塞 agent loop；写入 gated on `DbReadinessGate`（PG 没就绪就静默等待，失败只 warn 不 panic）。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/db_tracking/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 agent 后台记录（工具调用/token/终端/搜索/审计 写 PG）、记忆存取时
- 排查记录未落库（PG 就绪门）、`ToolCallGuard` 生命周期时

## 职责

`DbTracker` 是穿过 agent loop 的轻量句柄，所有方法 spawn fire-and-forget 任务，**永不阻塞** agentic loop。查询经 `DbReadinessGate` 门控（PG 未就绪则短超时静默等待，避免撞 pool acquire_timeout）。失败只 log warn，不 panic。

## 公开接口

| 符号 | 说明 |
|---|---|
| `DbTracker` | fire-and-forget 后台记录句柄 |
| `ToolCallGuard` | 工具调用 guard（RAII 记录） |
| `MemoryHit` / `ScoredMemoryHit` / `BriefingPlan` | 记忆命中 / 评分 / briefing |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `DbTracker` 句柄 |
| `recording.rs` / `memory.rs` / `helpers.rs` / `types.rs` | 记录 / 记忆 / helper / DTO |

## 依赖

- crate 内 `db_traits`（`DbTrackingBackend`/`DbReadinessGate`/`TextEmbedder`）、`uuid`、`tokio`

## 注意事项 / 坑

- **绝不阻塞 loop**：所有写都 fire-and-forget；别改成 await 阻塞（会卡 agent）。
- 经 trait（db_traits）写，不直接依赖 golish-db；就绪门防 PG 启动期撞超时。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit db_tracking
```
