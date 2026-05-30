# P1-1 实现计划 · golish-db 泛型 scoped CRUD helper

> 日期：2026-05-30
> 状态：In progress
> 来源：`docs/design/2026-05-29-architecture-optimization.md` §5 P1-1 + §3.1 B-D1
> 负责会话：bajie-mcp-agent-2（临时主控 MCP-4 派发，分发开启）
> 范围铁律：**只动 `backend/crates/golish-db/src/repo/`**；不碰 golish-core / golish-settings / golish / frontend。

---

## 1. 目标

用一个泛型 helper 模块 `repo/scoped.rs` 收敛 B-D1 中跨 10+ repo 文件逐字复制的 CRUD 模板，
**保持每个 repo 模块的公开函数签名完全不变**（调用方在 `golish` crate，不在本次范围内，不能被破坏），
仅把函数体改为委托到泛型 helper。

收敛的模板（证据见 design doc §3.1 B-D1）：

| 模板 | 原始 SQL | 泛型 helper |
|---|---|---|
| 按 id 取单行（非作用域） | `SELECT * FROM <t> WHERE id = $1`（fetch_optional） | `scoped::get_by_id<T>` |
| 作用域 list | `SELECT * FROM <t> WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY <col>` | `scoped::list_by_project<T>` |
| 按 id 删（非作用域） | `DELETE FROM <t> WHERE id = $1` | `scoped::delete_by_id` |
| 作用域删（IDOR） | `DELETE FROM <t> WHERE id=$1 AND project_path IS NOT DISTINCT FROM $2` | `scoped::delete_scoped` |
| 作用域按 id 取整行（IDOR，P0-3 预留） | `SELECT * FROM <t> WHERE id=$1 AND project_path IS NOT DISTINCT FROM $2` | `scoped::get_scoped<T>` |
| JSON `data` upsert | `INSERT … ON CONFLICT (id) DO UPDATE SET data=$2, updated_at=NOW()` | `scoped::upsert_json_data` |
| JSON `data` 作用域读 | `SELECT data FROM <t> WHERE id=$1 AND project_path IS NOT DISTINCT FROM $2` | `scoped::get_json_data_scoped` |
| JSON `data` 项目列表 | `SELECT data FROM <t> WHERE project_path = $1 ORDER BY <col>` | `scoped::list_json_data_by_project` |

## 2. 影响面

- **新增**：`backend/crates/golish-db/src/repo/scoped.rs`
- **改 1 行**：`backend/crates/golish-db/src/repo/mod.rs`（`pub mod scoped;`）
- **委托迁移**（函数体改、签名不变）：
  - `get_by_id`：findings / targets / vault / sessions / pipelines / methodology / tasks / subtasks / execution_plans / tool_calls / js_analysis / organizations(`get_one`)（12）
  - `list_by_project`：findings / targets / vault / methodology / pipelines（5）
  - `delete_by_id`：findings / targets / vault / sessions / pipelines / methodology / execution_plans / js_analysis / organizations / target_assets / memories / vuln_scan(`delete_scan`) / api_endpoints / passive_scans / fingerprints / wiki_kb::poc(`delete_poc`)（16）
  - `delete_scoped`：findings / vault / methodology / pipelines / notes(`delete`)（5）
  - JSON 三件套：methodology / pipelines（upsert + 作用域读 + 项目列表）

> 明确**不动**（签名/语义不同，避免误伤）：`organizations::list`（exact `=` + 复杂 ORDER BY 且 `project_path: &str`）、`execution_plans::list_by_project`（含条件 SQL）、`vuln_intel::delete_feed`（id 为 `&str`）、各 `update_*` / `*_scoped` scalar 读（value/evidence 等）。

## 3. 安全考量（SQL 注入）

helper 用 `format!` 把 `table` / `order_by` 拼进 SQL。**所有调用点传入的都是编译期 `&'static str` 字面量**（表名、`"created_at DESC"` 等），非用户输入，无注入面。
额外加 `is_safe_sql_fragment()` 纯函数 + `debug_assert!` 防御性兜底，并单测其真/假分支。

## 4. 验证（决定性证据）

- 主命令：`cd backend && cargo nextest run -p golish-db`（直接 cargo，看真实退出码；**不信** `just check` 的 step 包装器）
- 编译节奏：批量改完后统一 `cargo build -p golish-db` → 批量修错 → 再跑 nextest（design doc §7.1）
- 新增纯单测（无需活 DB，与现有 `operation_state` 等单测同构）：断言
  `scoped::build_*_sql(...)` 生成的 SQL == 迁移前的原始 SQL 字符串（证明语义零漂移）。

## 5. 回滚

helper 为纯新增；每个 repo 函数逐个委托，未迁移的保持原状。单 commit 可整体 revert，或逐文件 revert。

## 6. 步骤

1. 写本计划（done）
2. 新增 `repo/scoped.rs`（helper + `build_*_sql` 纯函数 + 单测）；`mod.rs` 注册
3. 逐文件把上述函数体改为委托
4. `cargo build -p golish-db` 批量修错至通过
5. `cargo nextest run -p golish-db` 全绿，记录真实退出码
6. `send_to_session(result)` 回报 MCP-4：改了哪些文件 + 退出码证据
