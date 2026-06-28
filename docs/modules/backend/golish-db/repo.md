# golish-db / repo

> **一句话职责**：各表的 **scoped CRUD helper**——每个表一个模块（sessions/tool_calls/findings/targets/memories/vault/…40+ 个），统一经 `scoped.rs` 做资源所有权（IDOR）校验，是全仓库 DB 读写的权威入口。

- **类型**：目录模块（属于 crate [`golish-db`](../golish-db.md)）
- **路径**：`backend/crates/golish-db/src/repo/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改任何表的 CRUD（新 repo 方法、新表模块）时
- 任何跨服务读写——app crate 不直接写 SQL，走这里的 `repo::*`（横向耦合经 ports，纵向走 repo）
- 改 IDOR scoping 基座（`scoped.rs`）或批量操作的所有权校验时

## 职责

owns 全部表的结构化访问。每个表一个子模块（如 `findings.rs` / `targets.rs` / `memories.rs`），方法是 **scoped CRUD**：写/读都带 owner/project scope 校验（I2 IDOR）。`scoped.rs` 是 scope 校验基座，`audit/` 是审计相关 repo 子区。

## 公开接口

| 区域 | 说明 |
|---|---|
| `scoped`（`scoped.rs`） | scope/所有权校验基座（IDOR 守卫核心） |
| pentest/recon 表：`findings` / `methodology` / `execution_plans` / `evidence_classifications` / `targets` / `target_assets` / `organizations` / `directory_entries` / `sensitive_scan` / `custom_rules` / `passive_scans` / `scan_queue` / `fingerprints` / `vuln_scan` / `screenshots` / `sitemap_store` / `api_endpoints` / `endpoint_tests` / `js_analysis` | 各表 scoped CRUD |
| agent/会话表：`sessions` / `tasks` / `subtasks` / `tool_calls` / `message_chains` / `msg_logs` / `agent_logs` / `search_logs` / `terminal_logs` / `conversation_store` / `operation_state` / `stage_runs` / `sprint_contracts` / `sub_agent_dispatches` | agent 运行/会话数据 |
| harness 物化事实：`technique_outcomes` / `source_query_log` / `expansion_queue` / `stage_asset_waves` | stage gate/reviewer 的 coverage/source/扩展线索投影；`stage_asset_waves` 冻结 wave-aware stage 的当前资产批次 |
| 其它：`memories`（向量记忆） / `vault`（凭据） / `notes` / `kb_research` / `wiki_kb` / `prompt_templates` / `vuln_intel` / `vector_store_logs` | 记忆/凭据/笔记/知识库等 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 仅 `pub mod` 声明（40+ 表模块 + `audit`） |
| `scoped.rs` | scope/所有权校验基座（IDOR） |
| `audit/` | 审计相关 repo（嵌套子目录） |
| `coverage_truth.rs` | stage gate 的只读 DB 真值投影；EAS 会从 `targets.ports` / `fingerprints` / `real_ip` 关联 IP target 推导 found coverage |
| `targets.rs` | recon target CRUD + harness in-scope asset axis；`list_in_scope_values_created_before` 用 `created_at <= cutoff` 支持 no-schema stage wave denominator freeze |
| `stage_asset_waves.rs` | durable per-operation/org/stage asset batch repo：创建/读取 running batch、完成 batch；未分配 in-scope targets 可被后续 expansion pass 读取为待扩展 backlog |
| `<table>.rs` | 各表 scoped CRUD（一个表一个文件） |

## 依赖

- `sqlx`（`PgPool`）、`golish-core`（scope 类型）；被全部 `*-app` + domain crate 消费

## 注意事项 / 坑

- **不变量 I2**：所有 CRUD 必须验资源所有权（含批量）——新增表模块时复用 `scoped.rs`，别写裸 scope-less SQL。
- **raw SQL allowlist**：极少数裸 sqlx 调用在 `check_repo_ownership.py` ALLOWLIST 登记；新增裸 SQL 要么走 repo，要么显式登记。
- **不变量 I9**：repo 方法别在事务里调外部 HTTP/MQ/长耗。
- app crate **不直接写 SQL**：纵向走 `repo::*`，横向跨服务走 `golish-app-core/ports`。
- `source_query_log` 的幂等键必须包含 `organization_id`：`(organization_id, run_id, source, query, target)`。多 org `stage_run` 扇出时，root/子公司同源 provider 查询不能互相覆盖；`list_for_run` 供 gate/reviewer 只读 `(org, run)` 的 source/provider terminal rows，证明 source 尝试，不可当作 found truth。
- `coverage_truth.rs` 是 Found-only 投影：只能把业务表里确实存在的事实注入 gate，不从缺失数据推断 checked_empty；EAS 的 PORT/SERVICE/LIVENESS 必须保留 freshness window 约束。
- `stage_asset_waves` 是 additive schema：wave items 固定 `target_id/value/type/source` 成员关系，gate 仍从业务表/ledger 读事实；新发现 target 不会进入当前 batch denominator，也不会在单个 org PASS 后被自动重跑，后续由 global delta expansion pass 统一消费。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-db repo
```
