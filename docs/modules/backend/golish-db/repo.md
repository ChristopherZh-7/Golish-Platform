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
| 其它：`memories`（向量记忆） / `vault`（凭据） / `notes` / `kb_research` / `wiki_kb` / `prompt_templates` / `vuln_intel` / `vector_store_logs` | 记忆/凭据/笔记/知识库等 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 仅 `pub mod` 声明（40+ 表模块 + `audit`） |
| `scoped.rs` | scope/所有权校验基座（IDOR） |
| `audit/` | 审计相关 repo（嵌套子目录） |
| `<table>.rs` | 各表 scoped CRUD（一个表一个文件） |

## 依赖

- `sqlx`（`PgPool`）、`golish-core`（scope 类型）；被全部 `*-app` + domain crate 消费

## 注意事项 / 坑

- **不变量 I2**：所有 CRUD 必须验资源所有权（含批量）——新增表模块时复用 `scoped.rs`，别写裸 scope-less SQL。
- **raw SQL allowlist**：极少数裸 sqlx 调用在 `check_repo_ownership.py` ALLOWLIST 登记；新增裸 SQL 要么走 repo，要么显式登记。
- **不变量 I9**：repo 方法别在事务里调外部 HTTP/MQ/长耗。
- app crate **不直接写 SQL**：纵向走 `repo::*`，横向跨服务走 `golish-app-core/ports`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-db repo
```
