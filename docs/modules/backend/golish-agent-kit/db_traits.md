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
- harness gate 相关读写也走 `DbRepoProvider` seam：`technique_outcome_facts` 必须返回保留 `source` 的 `TechniqueOutcomeFact`，不能退回丢 provenance 的四元组；Enumeration `blocked` 的 submit/final gate 据此只接受 preflight→四轴、route recovery→DIR、browser recovery→JS/JSAPI/PARAM 的 source/axis 组合，并要求匹配 current-target guarded evidence。audit `kind` 的精确校验由 app bridge 在投影前完成，不能指望 kit trait 自行查询 DB。`source_query_facts` 投影 `source_query_log` terminal rows，但只证明 source 尝试、不证明 found。legacy 名称 `mark_target_intel_dns_empty_outcomes` 是 target_intel DNS attempt 的 app-side hook：runtime 拿到真实 evidence id 后调用，trait 默认 no-op，生产实现分别写 `technique_outcomes(GOLISH-INTEL-DNS, empty|error)`；只有明确 no-record 才 empty，resolver/transport failure 必须是非终态 error。
- Scoping 的 trusted target seam 是 `scoping_target_snapshot(org)`：app 实现只返回
  current-org `scope=in` 且 source 属于 manual/imported/stage-run-seed/seed/cli 的可支持类型。
  `parse_scope_review_tool_result` 必须解外层 ToolResult 及 `response` 内层 JSON array，skip/
  free text/畸形返回均不是批准。orchestrator 按 canonical value + type + scope 精确对齐，
  snapshot 读失败 fail closed；这条 seam 只读，绝不把 review proposal 写成 target。
- `scoping_actions_for_session(session, org, not_before)` 的 `org` 是 gate 已解析出的 trusted root，app/repo 投影不得降级成 session-wide 布尔值。`ScopingActionsSeen` 区分 parent-only exclusion、成功 proposal、成功且在 proposal 后完成的 unit review，以及 target review attempts；error/skip/另一 org/乱序都不能置为成功。
- EAS gate 的 ledger seam 是 `eas_evidence_facts_for_session_org_fresh(session, org, since)`：默认空且绝不 fallback 到 session-wide facts；app 实现负责 producer org、current target owner/project/scope、freshness 与 asset/technique raw witness 校验。
- EAS exact-origin seam 是 `eas_required_web_origins(org, since, current_wave_target_ids)`：返回本轮 fresh、current-owner、project 精确匹配且仍由 target 当前 URL/开放端口/明确 HTTP service 授权的 canonical origins；调用方明确传入空 wave membership 时必须保留 authoritative empty，读取失败必须让 preview/final gate fail closed。
- wave-aware stage 的 durable batch 也走 `DbRepoProvider` seam：`stage_asset_wave_current_or_create_initial` / `stage_asset_wave_create_next` / `stage_asset_wave_complete` 默认 no-op/None，app bridge 才接到 `golish-db::repo::stage_asset_waves`。最终 close 必须调用 `stage_asset_wave_create_next_or_seal_completion`，让“queue 下一波”与“原子发布 org completion 水位”成为互斥结果；不能用普通 create-next 后再单独写 completion。coverage snapshot seam 同时接 current wave ids/values，不能只传 value；present-invalid wave 与 `None` 必须保持可区分。
- completion 的 operation-bound 读取走 `org_stage_completions_get_with_run_id`；app bridge 必须保留 DB 行的 `stage_run_id`，stage_run/orchestrator 再与 current operation UUID 精确比较。默认 trait 把 legacy projection 映成 `stage_run_id=None`，因此 operation-bound caller 会 fail closed，不能只凭 fresh `passed_at` 接受 sibling operation 的 PASS。
- `stage_asset_coverage_for_operation` 是 operation-aware coverage seam：默认实现只为兼容测试 provider 回落旧 `stage_asset_coverage`，生产 app bridge必须把 trusted operation id 传给 snapshot。Enumeration 的 EAS transport handoff 只允许在这条 seam 下缩小 exact-origin denominator；tool executor、submit preview、task close 和 final org gate漏传 operation id 时必须 fail closed 为未排除，而不是读取全局/最新 marker。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit db_traits
```
