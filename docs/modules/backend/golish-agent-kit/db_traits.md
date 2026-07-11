# golish-agent-kit / db_traits

> **一句话职责**：DB 操作的 trait 抽象 + 本地模型类型——把 agent 层与 `golish-db`/`sqlx` 完全解耦：`DbRepoProvider`（CRUD）/ `DbTrackingBackend`（记录+记忆）/ `DbReadinessGate`（PG 就绪门）/ `TextEmbedder`（语义记忆嵌入），由 application 层注入实现。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/db_traits/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 agent 层与 DB 的边界 trait（repo/tracking/readiness/embedder）或本地 DTO 时
- 排查 agent 层为何不直接依赖 golish-db/sqlx 时

## 职责

定义 agent 层访问 DB 所需的 trait + 本地模型，使本 crate **不依赖 golish-db/sqlx**（依赖倒置）。application 层（golish-agent-app 的 db_bridge）提供具体实现。

## 公开接口

| 符号 | 说明 |
|---|---|
| `DbRepoProvider` | 仓库操作（tasks/subtasks/plans/wiki… CRUD） |
| `DbTrackingBackend` | fire-and-forget 记录 + memory 存/搜 |
| `DbReadinessGate` | PG 启动就绪门 |
| `TextEmbedder` | 语义记忆文本嵌入 |
| `types` / `memory` / `repo` / `tracking`（本地 DTO） | trait + 本地模型；`StageAssetWaveView` 携带 durable wave 的对齐 `target_ids + asset_values`；`TechniqueOutcomeFact` 保留 asset/technique/outcome/evidence_id **及 source**，让 submit/final gate 能验证 trusted producer |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `DbReadinessGate` + re-export |
| `repo.rs` / `tracking.rs` / `memory.rs` / `types.rs` | 各 trait + 本地 DTO |

## 依赖

- `async-trait`、`uuid`；**无 golish-db/sqlx**（这是本模块的全部意义）

## 注意事项 / 坑

- **依赖倒置核心**：本 crate 定义 trait，golish-agent-app 注入实现——别在此引 golish-db/sqlx（会破坏 agent 栈的可测/解耦）。
- 本地 DTO 与 golish-db 的 row 类型是两套，由 bridge 转换。
- harness gate 相关读写也走 `DbRepoProvider` seam：`technique_outcome_facts` 必须返回保留 `source` 的 `TechniqueOutcomeFact`，不能退回丢 provenance 的四元组；Enumeration `blocked` 的 submit/final gate 据此只接受 preflight→四轴、route recovery→DIR、browser recovery→JS/JSAPI/PARAM 的 source/axis 组合，并要求匹配 current-target guarded evidence。audit `kind` 的精确校验由 app bridge 在投影前完成，不能指望 kit trait 自行查询 DB。`source_query_facts` 投影 `source_query_log` terminal rows，但只证明 source 尝试、不证明 found。`mark_target_intel_dns_empty_outcomes` 是 target_intel DNS negative fact 的 app-side hook：runtime 拿到真实 evidence id 后调用，trait 默认 no-op，生产实现写 `technique_outcomes(GOLISH-INTEL-DNS, empty)`。
- EAS gate 的 ledger seam 是 `eas_evidence_facts_for_session_org_fresh(session, org, since)`：默认空且绝不 fallback 到 session-wide facts；app 实现负责 producer org、current target owner/project/scope、freshness 与 asset/technique raw witness 校验。
- wave-aware stage 的 durable batch 也走 `DbRepoProvider` seam：`stage_asset_wave_current_or_create_initial` / `stage_asset_wave_create_next` / `stage_asset_wave_complete` 默认 no-op/None，app bridge 才接到 `golish-db::repo::stage_asset_waves`。coverage snapshot seam 同时接 current wave ids/values，不能只传 value；present-invalid wave 与 `None` 必须保持可区分。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit db_traits
```
